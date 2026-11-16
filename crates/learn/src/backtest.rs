//! Score a candidate rule against history.

use crate::cluster;
use ingrain_core::{AgentHunk, Backtest, Correction, LineRange, Snippet};
use ingrain_engine::{CompiledRule, Parsed, lang};
use std::collections::{HashMap, HashSet};

/// Backtest result plus the numbers derived from it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Score {
    pub backtest: Backtest,
    /// Correction indices (into the cluster's slice) whose agent code the rule flags.
    pub caught: Vec<usize>,
    /// Corrections whose agent lines the rule hit (cluster's own source hunks held out).
    pub true_hits: u32,
    /// Hits on agent lines humans kept.
    pub false_hits: u32,
}

impl Score {
    /// Share of hits on agent code that landed on corrected lines.
    /// `None` when the rule never fired on agent code.
    pub fn precision(&self) -> Option<f64> {
        let total = self.true_hits + self.false_hits;
        (total > 0).then(|| f64::from(self.true_hits) / f64::from(total))
    }
    /// Lower bound of the 90% Wilson interval on precision: a rule with 2 of 3
    /// hits right is not trusted like one with 200 of 300. `None` when it never fired.
    pub fn precision_lower_bound(&self) -> Option<f64> {
        let n = f64::from(self.true_hits + self.false_hits);
        if n == 0.0 {
            return None;
        }
        let p = f64::from(self.true_hits) / n;
        let z = 1.645_f64;
        let z2 = z * z;
        let center = p + z2 / (2.0 * n);
        let margin = z * ((p * (1.0 - p) + z2 / (4.0 * n)) / n).sqrt();
        Some(((center - margin) / (1.0 + z2 / n)).max(0.0))
    }

    pub fn after_rate(&self) -> f64 {
        let b = &self.backtest;
        if b.afters == 0 {
            0.0
        } else {
            f64::from(b.after_hits) / f64::from(b.afters)
        }
    }
}

/// True if the rule fires on the snippet's changed lines.
pub fn flags(rule: &CompiledRule, path: &str, snippet: &Snippet) -> bool {
    if snippet.changed.is_empty() || !rule.applies_to(path) {
        return false;
    }
    let Some(language) = lang::detect(path) else {
        return false;
    };
    rule.scan_text(language, &snippet.text)
        .iter()
        .any(|h| LineRange::new(h.start_line, h.end_line).overlaps(&snippet.changed))
}

/// Hits of one rule on one agent hunk, judged against later human corrections.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Judged {
    /// (line in the snippet, was it a true positive)
    pub hits: Vec<(u32, bool)>,
    /// Distinct corrections behind the true hits. A fix that rewrote three flagged
    /// lines is one mistake caught, not three, so true positives are counted per
    /// correction; false hits stay per line (each is a separate warning on kept code).
    pub true_corrections: Vec<String>,
}

impl Judged {
    /// Corrections this hunk's hits caught (not lines).
    pub fn true_hits(&self) -> u32 {
        self.true_corrections.len() as u32
    }
    pub fn false_hits(&self) -> u32 {
        self.hits.iter().filter(|(_, t)| !*t).count() as u32
    }
}

/// Judge every hit of `rule` on the added lines of `hunk`, one per line.
///
/// A hit is a true positive only if it lands on lines a human later corrected *and*
/// the rule also flags that correction's agent code, i.e. the human fixed the thing the
/// rule is about, not something else on the same line. `known` selects which
/// corrections count (by time), so training never sees the future.
pub fn judge(
    rule: &CompiledRule,
    parsed: &Parsed,
    hunk: &AgentHunk,
    corrections: &HashMap<&str, &Correction>,
    known: &dyn Fn(i64) -> bool,
    relevant: &mut HashMap<String, bool>,
) -> Judged {
    let mut seen = HashSet::new();
    let mut out = Judged::default();
    for hit in rule.scan(parsed) {
        let range = LineRange::new(hit.start_line, hit.end_line);
        if !range.overlaps(&hunk.snippet.changed) || !seen.insert(hit.start_line) {
            continue;
        }
        let by = hunk
            .corrected
            .iter()
            .filter(|c| known(c.time) && c.lines.overlaps(&range))
            .find(|c| {
                *relevant.entry(c.correction.clone()).or_insert_with(|| {
                    corrections
                        .get(c.correction.as_str())
                        .is_some_and(|corr| flags(rule, &corr.path, &corr.before))
                })
            });
        if let Some(c) = by
            && !out.true_corrections.contains(&c.correction)
        {
            out.true_corrections.push(c.correction.clone());
        }
        out.hits.push((hit.start_line, by.is_some()));
    }
    out
}

/// Score `rule` on a cluster's corrections and on the agent-hunk population.
///
/// `prefilter` is a token the matched code must contain; hunks without it are
/// skipped without parsing. Only pass it for rules that can't match without the
/// token (template rules). Only hunks written in `[since, known_before)` count:
/// code from before humans first made this correction was fine at the time (think
/// of a renamed API), and `known_before` keeps a time-split evaluation from seeing
/// the future. `all` maps correction ids to corrections, to check that a hit on
/// corrected lines is about what the human fixed.
///
/// The hunks the cluster's own corrections were mined from are held out: a rule
/// always fires on the code it was learned from, so those hits say nothing about
/// precision. Each source hunk is held out per fix event (leave-one-event-out): it is
/// scored only when the rule also catches a correction from *another* fix event, so
/// it then tests a rule that fix event alone would not have produced. With a single
/// fix event every source hunk is excluded, and the rule has precision evidence only
/// if it fires on other agent code.
#[allow(clippy::too_many_arguments)]
pub fn score(
    rule: &CompiledRule,
    corrections: &[&Correction],
    hunks: &[AgentHunk],
    all: &HashMap<&str, &Correction>,
    prefilter: Option<&str>,
    since: i64,
    known_before: i64,
) -> Score {
    let mut s = Score::default();
    for (i, c) in corrections.iter().enumerate() {
        s.backtest.befores += 1;
        if flags(rule, &c.path, &c.before) {
            s.backtest.before_hits += 1;
            s.caught.push(i);
        }
        if !c.after.changed.is_empty() {
            s.backtest.afters += 1;
            if flags(rule, &c.path, &c.after) {
                s.backtest.after_hits += 1;
            }
        }
    }

    // Fix event of each of the cluster's corrections, and the events the rule catches.
    let event_of: HashMap<&str, String> = corrections
        .iter()
        .map(|c| (c.id.as_str(), cluster::source_key(c)))
        .collect();
    let caught_events: HashSet<&str> = s
        .caught
        .iter()
        .map(|&i| event_of[corrections[i].id.as_str()].as_str())
        .collect();
    let own_only = |h: &AgentHunk| {
        let own: HashSet<&str> = h
            .corrected
            .iter()
            .filter_map(|l| event_of.get(l.correction.as_str()))
            .map(String::as_str)
            .collect();
        !own.is_empty() && caught_events.iter().all(|e| own.contains(e))
    };

    let known = |t: i64| t < known_before;
    let mut relevant = HashMap::new();
    let mut true_corrections: HashSet<String> = HashSet::new();
    for h in hunks
        .iter()
        .filter(|h| h.time >= since && h.time < known_before)
    {
        if own_only(h) {
            continue;
        }
        if prefilter.is_some_and(|p| !h.snippet.text.contains(p)) || !rule.applies_to(&h.path) {
            continue;
        }
        let Some(language) = lang::detect(&h.path) else {
            continue;
        };
        s.backtest.kept += 1;
        let parsed = Parsed::new(language, &h.snippet.text);
        let judged = judge(rule, &parsed, h, all, &known, &mut relevant);
        // One correction split across chunks of the same commit still counts once.
        true_corrections.extend(judged.true_corrections.iter().cloned());
        s.false_hits += judged.false_hits();
        s.backtest.kept_hits += judged.false_hits();
    }
    s.true_hits = true_corrections.len() as u32;
    s
}

/// Index corrections by id.
pub fn by_id<'a>(
    corrections: impl IntoIterator<Item = &'a Correction>,
) -> HashMap<&'a str, &'a Correction> {
    corrections
        .into_iter()
        .map(|c| (c.id.as_str(), c))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cluster::tests::correction;
    use ingrain_core::CorrectedLines;

    const RULE: &str =
        "id: no-fetch\nlanguage: TypeScript\nrule:\n  pattern: fetch($$$A)\nmessage: m\n";

    pub(crate) fn hunk(text: &str, corrected: Option<LineRange>, time: i64) -> AgentHunk {
        AgentHunk {
            commit: "a".into(),
            path: "src/x.ts".into(),
            lang: "TypeScript".into(),
            time,
            snippet: Snippet {
                text: text.into(),
                changed: LineRange::new(1, text.lines().count() as u32),
                first_line: 1,
            },
            corrected: corrected
                .map(|lines| CorrectedLines {
                    lines,
                    time: 5,
                    correction: "1".into(),
                })
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn scores_corrections_and_hunks() {
        let rule = CompiledRule::compile(RULE).unwrap();
        let c = correction(
            "1",
            "TypeScript",
            "await fetch(u);",
            "await api.get(u);",
            "f",
        );
        // A second fix event, so c's own hunk tests a rule `g` alone would also give.
        let other = correction("2", "TypeScript", "fetch(w);", "api.get(w);", "g");
        let hunks = vec![
            hunk("a();\nawait fetch(u);", Some(LineRange::new(2, 2)), 1),
            hunk("await fetch(v);", None, 1),
            hunk("nothing here", None, 1),
        ];
        let all = by_id([&c, &other]);
        let s = score(
            &rule,
            &[&c, &other],
            &hunks,
            &all,
            Some("fetch"),
            0,
            i64::MAX,
        );
        assert_eq!(s.backtest.before_hits, 2);
        assert_eq!(s.backtest.after_hits, 0);
        assert_eq!(s.true_hits, 1);
        assert_eq!(s.false_hits, 1);
        assert_eq!(s.precision(), Some(0.5));
        assert_eq!(
            s.backtest.kept, 2,
            "prefilter skipped the hunk without `fetch`"
        );
    }

    #[test]
    fn a_single_fix_does_not_certify_itself() {
        let rule = CompiledRule::compile(RULE).unwrap();
        // One fix rewrote three fetch lines of one agent hunk.
        let text = "x = fetch(a);\ny = fetch(b);\nz = fetch(c);";
        let c = correction("1", "TypeScript", text, "x = api.get(a);", "f");
        let all = by_id([&c]);
        let hunks = vec![hunk(text, Some(LineRange::new(1, 3)), 1)];
        let s = score(&rule, &[&c], &hunks, &all, None, 0, i64::MAX);
        assert_eq!(s.caught, vec![0]);
        assert_eq!((s.true_hits, s.false_hits), (0, 0), "own hunk held out");
        assert_eq!(s.backtest.kept, 0);
        assert_eq!(s.precision_lower_bound(), None);

        // The same hunk judged for a rule from another cluster: one correction, one true hit.
        let s = score(&rule, &[], &hunks, &all, None, 0, i64::MAX);
        assert_eq!(
            (s.true_hits, s.false_hits),
            (1, 0),
            "per correction, not per line"
        );
    }

    #[test]
    fn source_hunks_count_when_another_fix_supports_the_rule() {
        let rule = CompiledRule::compile(RULE).unwrap();
        let a = correction("1", "TypeScript", "fetch(a);", "api.get(a);", "f1");
        let b = correction("2", "TypeScript", "fetch(b);", "api.get(b);", "f2");
        let all = by_id([&a, &b]);
        let mut hb = hunk("fetch(b);", Some(LineRange::new(1, 1)), 1);
        hb.corrected[0].correction = "2".into();
        let hunks = vec![hunk("fetch(a);", Some(LineRange::new(1, 1)), 1), hb];
        let s = score(&rule, &[&a, &b], &hunks, &all, None, 0, i64::MAX);
        assert_eq!((s.true_hits, s.false_hits), (2, 0));
        // Two corrections from one commit are one fix event: both hunks held out.
        let b1 = correction("2", "TypeScript", "fetch(b);", "api.get(b);", "f1");
        let all = by_id([&a, &b1]);
        let s = score(&rule, &[&a, &b1], &hunks, &all, None, 0, i64::MAX);
        assert_eq!((s.true_hits, s.false_hits), (0, 0));
    }

    #[test]
    fn hit_on_lines_fixed_for_another_reason_is_false() {
        let rule = CompiledRule::compile(RULE).unwrap();
        // The human renamed a variable on the fetch line; the fetch stayed.
        let c = correction("1", "TypeScript", "const a = 1;", "const b = 1;", "f");
        let all = by_id([&c]);
        let hunks = vec![hunk(
            "const a = await fetch(u);",
            Some(LineRange::new(1, 1)),
            1,
        )];
        let s = score(&rule, &[], &hunks, &all, None, 0, i64::MAX);
        assert_eq!((s.true_hits, s.false_hits), (0, 1));
    }

    #[test]
    fn small_samples_get_a_low_bound() {
        let few = Score {
            true_hits: 2,
            false_hits: 1,
            ..Default::default()
        };
        let many = Score {
            true_hits: 200,
            false_hits: 100,
            ..Default::default()
        };
        assert!(few.precision_lower_bound().unwrap() < 0.35);
        assert!(many.precision_lower_bound().unwrap() > 0.6);
        assert_eq!(Score::default().precision_lower_bound(), None);
    }

    #[test]
    fn hides_the_future() {
        let rule = CompiledRule::compile(RULE).unwrap();
        let hunks = vec![hunk("await fetch(u);", Some(LineRange::new(1, 1)), 1)];
        // The correction happened at t=5; at t=3 the line looks kept.
        let c = correction(
            "1",
            "TypeScript",
            "await fetch(u);",
            "await api.get(u);",
            "f",
        );
        let all = by_id([&c]);
        let s = score(&rule, &[], &hunks, &all, None, 0, 3);
        assert_eq!((s.true_hits, s.false_hits), (0, 1));
        // Code written before the window doesn't count at all.
        let s = score(&rule, &[], &hunks, &all, None, 2, i64::MAX);
        assert_eq!((s.true_hits, s.false_hits), (0, 0));
    }
}
