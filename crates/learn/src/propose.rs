//! The learning pipeline: corrections → clusters → candidate rules → backtested proposals.

use crate::backtest::{self, Score};
use crate::cluster::{self, Anchor, Cluster};
use crate::synth::{self, Candidate, Origin, llm::Llm, template};
use anyhow::Result;
use ingrain_core::{AgentHunk, Correction, Example, IngrainMeta, Mode, RuleFile, Source};
use ingrain_engine::CompiledRule;
use rayon::prelude::*;
use std::collections::HashSet;

/// Hits needed inside the "after it became learnable" window to judge precision there.
const MIN_WINDOW_HITS: u32 = 5;

pub struct ProposeOptions<'a> {
    /// Distinct fix events a rule must catch.
    pub min_support: usize,
    /// Minimum precision on agent hunks, when the rule fires on any.
    pub min_precision: f64,
    /// Maximum share of corrected versions the rule may still flag.
    pub max_after_rate: f64,
    pub llm: Option<&'a dyn Llm>,
    /// How many clusters without a passing template are sent to the LLM.
    pub llm_clusters: usize,
    pub llm_attempts: usize,
    /// Only data before this time is used (a time-split evaluation passes the cutoff).
    pub known_before: i64,
    /// Rule ids that exist already (accepted, pending or rejected).
    pub taken_ids: HashSet<String>,
    /// Already-accepted rules: corrections they catch are not proposed again.
    pub existing: &'a [CompiledRule],
}

impl Default for ProposeOptions<'_> {
    fn default() -> Self {
        Self {
            min_support: 3,
            min_precision: 0.5,
            max_after_rate: 0.1,
            llm: None,
            llm_clusters: 10,
            llm_attempts: 3,
            known_before: i64::MAX,
            taken_ids: HashSet::new(),
            existing: &[],
        }
    }
}

pub struct Proposal {
    pub rule: RuleFile,
    pub compiled: CompiledRule,
    pub anchor: Anchor,
    /// Ids of the corrections the rule catches.
    pub caught: Vec<String>,
    pub score: Score,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProposeStats {
    pub corrections: usize,
    pub clusters: usize,
    pub template_rules: usize,
    pub llm_tried: usize,
    pub llm_rules: usize,
    pub llm_errors: usize,
    /// Every cluster and how each template candidate scored, for `--explain`.
    pub explain: Vec<ClusterExplain>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClusterExplain {
    pub anchor: String,
    pub lang: String,
    pub support: usize,
    pub members: usize,
    pub candidates: Vec<CandidateExplain>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CandidateExplain {
    pub label: String,
    pub before_hits: u32,
    pub befores: u32,
    pub after_hits: u32,
    pub true_hits: u32,
    pub false_hits: u32,
    pub accepted: bool,
}

/// A candidate that compiled, with its score.
type Scored = (Candidate, CompiledRule, Score);

pub fn propose(
    corrections: &[Correction],
    hunks: &[AgentHunk],
    opts: &ProposeOptions<'_>,
) -> Result<(Vec<Proposal>, ProposeStats)> {
    let train: Vec<Correction> = corrections
        .iter()
        .filter(|c| c.time < opts.known_before)
        .filter(|c| {
            !opts
                .existing
                .iter()
                .any(|r| backtest::flags(r, &c.path, &c.before))
        })
        .cloned()
        .collect();
    let clusters = cluster::cluster(&train, opts.min_support);
    let mut stats = ProposeStats {
        corrections: train.len(),
        clusters: clusters.len(),
        ..Default::default()
    };

    let all = backtest::by_id(&train);
    // Template rules always contain the anchor token, so hunks without it can be
    // skipped; model-written rules can match without it and are scored on every hunk.
    let evaluate = |rule: &CompiledRule, cl: &Cluster, origin: Origin| -> Score {
        let members: Vec<&Correction> = cl.members.iter().map(|&i| &train[i]).collect();
        let since = learnable_at(&members, opts.min_support);
        let prefilter = (origin == Origin::Template).then_some(cl.anchor.token.as_str());
        let windowed = backtest::score(
            rule,
            &members,
            hunks,
            &all,
            prefilter,
            since,
            opts.known_before,
        );
        if windowed.true_hits + windowed.false_hits >= MIN_WINDOW_HITS {
            return windowed;
        }
        // Too little evidence after the rule became learnable: judge it on all earlier
        // agent code rather than trusting (or rejecting) it on a hit or two.
        backtest::score(rule, &members, hunks, &all, prefilter, 0, opts.known_before)
    };
    let accept = |s: &Score, cl: &Cluster| -> bool {
        let caught_support = cl
            .members
            .iter()
            .enumerate()
            .filter(|(i, _)| s.caught.contains(i))
            .map(|(_, &m)| cluster::source_key(&train[m]))
            .collect::<HashSet<_>>()
            .len();
        caught_support >= opts.min_support
            && s.after_rate() <= opts.max_after_rate
            && s.precision_lower_bound()
                .is_none_or(|p| p >= opts.min_precision)
    };

    // Templates: cheap, run in parallel.
    let templated: Vec<(Option<Scored>, ClusterExplain)> = clusters
        .par_iter()
        .map(|cl| {
            let members: Vec<&Correction> = cl.members.iter().map(|&i| &train[i]).collect();
            let id = rule_id(&cl.anchor);
            let mut explain = ClusterExplain {
                anchor: if cl.anchor.callee {
                    format!("{}(...)", cl.anchor.token)
                } else {
                    cl.anchor.token.clone()
                },
                lang: cl.anchor.lang.clone(),
                support: cl.support,
                members: cl.members.len(),
                candidates: Vec::new(),
            };
            let mut best: Option<Scored> = None;
            for cand in template::candidates(cl, &members, &id) {
                let Ok(compiled) = CompiledRule::compile(&cand.yaml) else {
                    continue;
                };
                let score = evaluate(&compiled, cl, Origin::Template);
                let accepted = accept(&score, cl);
                explain.candidates.push(CandidateExplain {
                    label: cand.label.clone(),
                    before_hits: score.backtest.before_hits,
                    befores: score.backtest.befores,
                    after_hits: score.backtest.after_hits,
                    true_hits: score.true_hits,
                    false_hits: score.false_hits,
                    accepted,
                });
                let rank = |s: &Score| {
                    s.true_hits as i64 - s.false_hits as i64 + s.backtest.before_hits as i64
                };
                if accepted && best.as_ref().is_none_or(|(_, _, b)| rank(&score) > rank(b)) {
                    best = Some((cand, compiled, score));
                }
            }
            (best, explain)
        })
        .collect();

    let mut found: Vec<(usize, Candidate, CompiledRule, Score)> = Vec::new();
    let mut need_llm = Vec::new();
    for (i, (t, explain)) in templated.into_iter().enumerate() {
        stats.explain.push(explain);
        match t {
            Some((cand, compiled, score)) => found.push((i, cand, compiled, score)),
            None => need_llm.push(i),
        }
    }
    stats.template_rules = found.len();

    if let Some(llm) = opts.llm {
        let tried: Vec<usize> = need_llm.into_iter().take(opts.llm_clusters).collect();
        stats.llm_tried = tried.len();
        let results: Vec<(usize, Result<Option<Scored>>)> = tried
            .par_iter()
            .with_max_len(1)
            .map(|&i| {
                let cl = &clusters[i];
                let members: Vec<&Correction> = cl.members.iter().map(|&m| &train[m]).collect();
                let r = synth::with_llm(
                    llm,
                    cl,
                    &members,
                    &rule_id(&cl.anchor),
                    opts.llm_attempts,
                    &|rule| evaluate(rule, cl, Origin::Llm),
                    &|s| accept(s, cl),
                );
                (i, r)
            })
            .collect();
        for (i, r) in results {
            match r {
                Ok(Some((cand, compiled, score))) => {
                    stats.llm_rules += 1;
                    found.push((i, cand, compiled, score));
                }
                Ok(None) => {}
                Err(_) => stats.llm_errors += 1,
            }
        }
    }

    // Best rules first; drop rules that mostly catch what a better rule already catches.
    found.sort_by(|a, b| {
        let key = |s: &Score| (s.caught.len(), s.true_hits as i64 - s.false_hits as i64);
        key(&b.3).cmp(&key(&a.3))
    });
    let mut covered: HashSet<String> = HashSet::new();
    let mut ids = opts.taken_ids.clone();
    let mut proposals = Vec::new();
    for (i, cand, compiled, score) in found {
        let cl = &clusters[i];
        let caught: Vec<String> = score
            .caught
            .iter()
            .map(|&k| train[cl.members[k]].id.clone())
            .collect();
        let new = caught.iter().filter(|id| !covered.contains(*id)).count();
        if new * 5 < caught.len() {
            continue; // at least 20% of what it catches must be new
        }
        let id = unique_id(&rule_id(&cl.anchor), &cl.anchor.lang, &mut ids);
        let members: Vec<&Correction> = score
            .caught
            .iter()
            .map(|&k| &train[cl.members[k]])
            .collect();
        let mut rule = RuleFile::from_parts(&cand.yaml, meta(&cand, &score, &members))?;
        rule.set_id(&id);
        let compiled = if id == compiled.id {
            compiled
        } else {
            CompiledRule::compile(&rule.to_yaml())?
        };
        covered.extend(caught.iter().cloned());
        proposals.push(Proposal {
            rule,
            compiled,
            anchor: cl.anchor.clone(),
            caught,
            score,
        });
    }
    Ok((proposals, stats))
}

/// When the rule could first have been learned: the time of the cluster's
/// `min_support`-th distinct fix. Precision is measured on agent code written after
/// that, which is the code the rule would actually have run on.
fn learnable_at(members: &[&Correction], min_support: usize) -> i64 {
    let mut first_seen: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for c in members {
        let t = first_seen.entry(cluster::source_key(c)).or_insert(c.time);
        *t = (*t).min(c.time);
    }
    let mut times: Vec<i64> = first_seen.into_values().collect();
    times.sort_unstable();
    times
        .get(min_support.saturating_sub(1))
        .or(times.last())
        .copied()
        .unwrap_or(0)
}

fn meta(cand: &Candidate, score: &Score, members: &[&Correction]) -> IngrainMeta {
    let support = members
        .iter()
        .map(|c| cluster::source_key(c))
        .collect::<HashSet<_>>()
        .len() as u32;
    IngrainMeta {
        mode: Mode::Shadow,
        support,
        origin: cand.origin.as_str().into(),
        evidence: distinct_events(members).take(5).map(evidence).collect(),
        backtest: Some(score.backtest.clone()),
        example: best_example(members),
    }
}

/// One correction per fix event (a commit that fixed three hunks is one piece of evidence).
fn distinct_events<'a>(members: &'a [&'a Correction]) -> impl Iterator<Item = &'a Correction> + 'a {
    let mut seen = HashSet::new();
    members
        .iter()
        .copied()
        .filter(move |c| seen.insert(cluster::source_key(c)))
}

/// The shortest correction that has both sides, so the agent sees a clear before → after.
fn best_example(members: &[&Correction]) -> Option<Example> {
    let side = |s: &ingrain_core::Snippet| first_lines(s.changed_text().trim_matches('\n'), 6);
    members
        .iter()
        .map(|c| (side(&c.before), side(&c.after)))
        .filter(|(b, a)| !b.trim().is_empty() && !a.trim().is_empty())
        .min_by_key(|(b, a)| b.len() + a.len())
        .map(|(before, after)| Example { before, after })
}

fn evidence(c: &Correction) -> String {
    match &c.source {
        Source::GitFixup { fix_commit, .. } => format!(
            "commit {} {}: {}",
            &fix_commit[..fix_commit.len().min(10)],
            c.path,
            c.note
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect::<String>()
        ),
        Source::ReviewSuggestion { url } => format!("review {url}"),
        Source::Transcript { session } => format!("session {session} {}", c.path),
    }
}

fn first_lines(s: &str, n: usize) -> String {
    s.lines().take(n).collect::<Vec<_>>().join("\n")
}

/// `avoid-fetch-call`, `avoid-any`, ...
pub fn rule_id(a: &Anchor) -> String {
    let slug: String = a
        .token
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "token" } else { slug };
    if a.callee {
        format!("avoid-{slug}-call")
    } else {
        format!("avoid-{slug}")
    }
}

fn unique_id(base: &str, lang: &str, taken: &mut HashSet<String>) -> String {
    let mut candidates = vec![base.to_owned(), format!("{base}-{}", lang.to_lowercase())];
    candidates.extend((2..).take(50).map(|n| format!("{base}-{n}")));
    let id = candidates
        .into_iter()
        .find(|c| !taken.contains(c))
        .expect("50 suffixes is enough");
    taken.insert(id.clone());
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::tests::correction;

    #[test]
    fn proposes_template_rule_from_repeated_corrections() {
        let cs: Vec<Correction> = (0..4)
            .map(|i| {
                correction(
                    &format!("c{i}"),
                    "TypeScript",
                    &format!("const r{i} = await fetch(u{i});"),
                    &format!("const r{i} = await apiClient.get(u{i});"),
                    &format!("fix{i}"),
                )
            })
            .collect();
        let (props, stats) = propose(&cs, &[], &ProposeOptions::default()).unwrap();
        assert_eq!(stats.template_rules, props.len());
        let fetch = props
            .iter()
            .find(|p| p.anchor.token == "fetch")
            .expect("fetch rule");
        assert_eq!(fetch.rule.id, "avoid-fetch-call");
        assert_eq!(fetch.caught.len(), 4);
        assert_eq!(fetch.rule.meta.mode, Mode::Shadow);
        assert_eq!(fetch.rule.meta.support, 4);
        assert!(fetch.rule.to_yaml().contains("fetch($$$ARGS)"));
    }

    #[test]
    fn learnable_at_is_the_nth_distinct_fix() {
        let mk = |fix: &str, t: i64| {
            let mut c = correction(fix, "TypeScript", "a", "b", fix);
            c.time = t;
            c
        };
        let cs = [
            mk("f1", 10),
            mk("f1", 5),
            mk("f2", 20),
            mk("f3", 30),
            mk("f4", 40),
        ];
        let refs: Vec<&Correction> = cs.iter().collect();
        assert_eq!(learnable_at(&refs, 3), 30);
        assert_eq!(learnable_at(&refs[..2], 3), 5);
    }

    #[test]
    fn respects_taken_ids_and_cutoff() {
        let cs: Vec<Correction> = (0..3)
            .map(|i| {
                let mut c = correction(
                    &format!("c{i}"),
                    "TypeScript",
                    "x = fetch(a);",
                    "x = api.get(a);",
                    &format!("f{i}"),
                );
                c.time = i as i64;
                c
            })
            .collect();
        let opts = ProposeOptions {
            known_before: 2,
            ..Default::default()
        };
        let (props, _) = propose(&cs, &[], &opts).unwrap();
        assert!(props.is_empty(), "only 2 corrections before the cutoff");

        let opts = ProposeOptions {
            taken_ids: ["avoid-fetch-call".to_string()].into(),
            ..Default::default()
        };
        let (props, _) = propose(&cs, &[], &opts).unwrap();
        assert!(
            props
                .iter()
                .any(|p| p.rule.id == "avoid-fetch-call-typescript")
        );
    }
}
