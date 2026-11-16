//! Turning project texts into proposed decision records.
//!
//! The prompt is the one measured in the validation study (82% of extracted records were
//! real, durable decisions). Every proposal waits for a human in `ingrain decisions review`.

pub mod source;

use crate::record::{Category, Record, Rejection, Status, date_of};
use anyhow::{Context, Result};
use ingrain_learn::synth::llm::Llm;
use serde::Deserialize;
use source::Text;
use std::collections::HashSet;

const PROMPT: &str = r#"You maintain a "decision ledger" for a software repository: short records of durable project
decisions that future code changes (often written by AI coding agents) must follow.

Below are texts from the repository: PR descriptions, review comments, issues, commit messages, or
what a developer told their coding agent. For each text, extract the durable decisions it states, if
any. A decision is something future code should keep doing or avoid, for example:
- a convention ("use `apiClient` instead of raw fetch", "tests use anyio.run, not @pytest.mark.anyio")
- a constraint ("billing must not import from api/", "support Python 3.10")
- an API change that callers must follow ("ClaudeCodeOptions is renamed to ClaudeAgentOptions")
- an architecture or scope choice ("we stay on SQLite", "this feature is out of scope for the SDK")

Not decisions: bug reports, questions, status updates, thanks, one-off fixes that only apply to the
change under discussion, descriptions of what a PR does with no lasting rule.

For each decision give:
- "decision": one imperative sentence an agent can follow
- "reason": why, in one short sentence (empty if not stated)
- "rejected": alternatives the text explicitly rejected (may be empty)
- "scope": list of path globs, module or symbol names it applies to (best guess; ["**"] if repo-wide)
- "category": convention | constraint | api_change | architecture | scope
- "supersedes": the id of an EXISTING decision below that this one replaces, or null
- "forbid_imports" / "forbid_deps": module or package names the decision clearly bans (else [])

Existing decisions (don't repeat them; use their id in "supersedes" if a text changes one):
{existing}

Most texts contain no decision; return an empty list for them. Reply with only JSON:
{"results": [{"id": "<text id>", "records": [...]}, ...]} covering every id below.

{texts}"#;

#[derive(Debug, Deserialize)]
struct Reply {
    results: Vec<ReplyItem>,
}

#[derive(Debug, Deserialize)]
struct ReplyItem {
    id: String,
    #[serde(default)]
    records: Vec<Extracted>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Extracted {
    pub decision: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub rejected: Vec<String>,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub supersedes: Option<String>,
    #[serde(default)]
    pub forbid_imports: Vec<String>,
    #[serde(default)]
    pub forbid_deps: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub proposals: Vec<Record>,
    pub texts: usize,
    pub calls: usize,
    pub failed_calls: usize,
    pub duplicates: usize,
    /// Ids of texts whose batch succeeded, and of texts whose batch failed.
    pub processed: Vec<String>,
    pub failed: Vec<String>,
}

/// Extract proposals from `texts`. `known` are live and pending records (for dedup and
/// supersedes); `rejected` are past rejections.
pub fn extract(
    llm: &dyn Llm,
    texts: &[Text],
    known: &[Record],
    rejected: &[Rejection],
    batch: usize,
    workers: usize,
) -> Outcome {
    let existing = if known.is_empty() {
        "(none yet)".to_owned()
    } else {
        known
            .iter()
            .filter(|r| r.status != Status::Superseded)
            .map(|r| format!("- {}: {}", r.id, r.decision))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let batches: Vec<&[Text]> = texts.chunks(batch.max(1)).collect();
    let results: Vec<Result<Vec<(String, Extracted)>>> =
        parallel(&batches, workers, |b| run_batch(llm, b, &existing));

    let mut out = Outcome {
        texts: texts.len(),
        calls: batches.len(),
        ..Default::default()
    };
    let mut seen: Vec<String> = known
        .iter()
        .map(|r| r.decision.clone())
        .chain(rejected.iter().map(|r| r.decision.clone()))
        .collect();
    let live_ids: HashSet<&str> = known
        .iter()
        .filter(|r| r.is_live())
        .map(|r| r.id.as_str())
        .collect();
    for (batch, r) in batches.iter().zip(results) {
        let ids = batch.iter().map(|t| t.id.clone());
        let Ok(items) = r else {
            out.failed_calls += 1;
            out.failed.extend(ids);
            continue;
        };
        out.processed.extend(ids);
        for (text_id, e) in items {
            let Some(text) = texts.iter().find(|t| t.id == text_id) else {
                continue;
            };
            // A proposal that replaces a live decision restates it by design; never drop it.
            let supersedes = e
                .supersedes
                .as_deref()
                .is_some_and(|s| live_ids.contains(s));
            if e.decision.trim().is_empty()
                || (!supersedes && seen.iter().any(|s| similar(s, &e.decision)))
            {
                out.duplicates += 1;
                continue;
            }
            seen.push(e.decision.clone());
            out.proposals.push(to_record(e, text, &live_ids));
        }
    }
    out
}

fn run_batch(llm: &dyn Llm, batch: &[Text], existing: &str) -> Result<Vec<(String, Extracted)>> {
    let texts = batch
        .iter()
        .map(|t| {
            let paths = if t.paths.is_empty() {
                String::new()
            } else {
                format!(
                    " · files: {}",
                    t.paths
                        .iter()
                        .take(10)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let body: String = t.text.chars().take(3000).collect();
            format!(
                "### id: {} · {} · {} · {}{paths}\n{body}",
                t.id,
                t.kind,
                t.author,
                date_of(t.time)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let prompt = PROMPT
        .replace("{existing}", existing)
        .replace("{texts}", &texts);
    let mut last = None;
    for _ in 0..2 {
        match llm.complete(&prompt).and_then(|reply| parse(&reply)) {
            Ok(items) => return Ok(items),
            Err(e) => last = Some(e),
        }
    }
    Err(last.expect("loop ran"))
}

fn parse(reply: &str) -> Result<Vec<(String, Extracted)>> {
    let start = reply.find('{').context("no JSON in reply")?;
    let end = reply.rfind('}').context("no JSON in reply")?;
    let parsed: Reply =
        serde_json::from_str(&reply[start..=end]).context("reply is not the expected JSON")?;
    Ok(parsed
        .results
        .into_iter()
        .flat_map(|item| item.records.into_iter().map(move |r| (item.id.clone(), r)))
        .collect())
}

fn to_record(e: Extracted, text: &Text, live: &HashSet<&str>) -> Record {
    let category = match e.category.as_deref() {
        Some("constraint") => Category::Constraint,
        Some("api_change") => Category::ApiChange,
        Some("architecture") => Category::Architecture,
        Some("scope") => Category::Scope,
        _ => Category::Convention,
    };
    let mut scope: Vec<String> = e
        .scope
        .into_iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    if scope.is_empty() {
        scope = text.paths.iter().take(5).cloned().collect();
    }
    let id = crate::record::proposal_id(&format!("{}\0{}", text.url, e.decision));
    Record {
        id,
        decision: e.decision.trim().to_owned(),
        reason: e.reason.trim().to_owned(),
        rejected: e.rejected,
        scope,
        category,
        status: Status::Proposed,
        supersedes: e.supersedes.filter(|s| live.contains(s.as_str())),
        superseded_by: None,
        checks: crate::record::Checks {
            forbid_imports: e.forbid_imports,
            forbid_deps: e.forbid_deps,
            forbid_patterns: Vec::new(),
        },
        sources: vec![text.url.clone()],
        author: text.author.clone(),
        date: date_of(text.time),
    }
}

/// Same decision in other words: word-set overlap (Jaccard) of at least 0.6, with the same
/// polarity (a negated decision never duplicates a plain one) and the same numbers (`3.10` is
/// not `3.12`).
pub fn similar(a: &str, b: &str) -> bool {
    let (wa, wb) = (Words::of(a), Words::of(b));
    if wa.negated != wb.negated || wa.numbers != wb.numbers {
        return false;
    }
    if wa.words.is_empty() || wb.words.is_empty() {
        return false;
    }
    let inter = wa.words.intersection(&wb.words).count() as f64;
    let union = wa.words.union(&wb.words).count() as f64;
    inter / union >= 0.6
}

/// The parts of a decision that `similar` compares.
struct Words {
    words: HashSet<String>,
    numbers: HashSet<String>,
    negated: bool,
}

impl Words {
    fn of(s: &str) -> Self {
        const STOP: &[&str] = &["the", "and", "for", "use", "with", "instead"];
        const NEGATIONS: &[&str] = &[
            "not",
            "no",
            "never",
            "dont",
            "doesnt",
            "didnt",
            "cant",
            "cannot",
            "wont",
            "shouldnt",
            "mustnt",
            "isnt",
            "arent",
            "nor",
            "avoid",
            "without",
            "forbid",
            "forbidden",
            "disallow",
            "ban",
            "banned",
            "stop",
        ];
        let mut out = Words {
            words: HashSet::new(),
            numbers: HashSet::new(),
            negated: false,
        };
        // `don't` → `dont`, so the apostrophe doesn't split it.
        let lower = s.to_lowercase().replace(['\'', '\u{2019}'], "");
        for raw in lower.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.') {
            let token = raw.trim_matches('.');
            if token.is_empty() {
                continue;
            }
            // Numbers and versions (`3`, `3.10`) are kept whole, whatever their length.
            if token.chars().all(|c| c.is_ascii_digit() || c == '.') {
                out.numbers.insert(token.to_owned());
                continue;
            }
            for w in token.split('.').filter(|w| !w.is_empty()) {
                if NEGATIONS.contains(&w) {
                    out.negated = true;
                } else if w.chars().all(|c| c.is_ascii_digit()) {
                    out.numbers.insert(w.to_owned());
                } else if w.len() > 2 && !STOP.contains(&w) {
                    out.words.insert(w.to_owned());
                }
            }
        }
        out
    }
}

/// Run `f` over `items` on up to `workers` threads, keeping order.
fn parallel<T: Sync, R: Send>(items: &[T], workers: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<R>>> =
        items.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..workers.max(1).min(items.len().max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= items.len() {
                        break;
                    }
                    let r = f(&items[i]);
                    *slots[i].lock().expect("no poisoned lock") = Some(r);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .expect("no poisoned lock")
                .expect("every slot filled")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    struct Fake(String);
    impl Llm for Fake {
        fn complete(&self, _: &str) -> Result<String> {
            Ok(self.0.clone())
        }
        fn describe(&self) -> String {
            "fake".into()
        }
    }
    struct Broken;
    impl Llm for Broken {
        fn complete(&self, _: &str) -> Result<String> {
            Err(anyhow!("down"))
        }
        fn describe(&self) -> String {
            "broken".into()
        }
    }

    fn text(id: &str) -> Text {
        Text {
            id: id.into(),
            kind: "pr".into(),
            author: "ana".into(),
            time: 1_772_600_767,
            text: "We stay on SQLite; no Redis.".into(),
            url: format!("https://x/{id}"),
            paths: vec!["src/cache.ts".into()],
        }
    }

    #[test]
    fn extracts_dedups_and_links_supersedes() {
        let reply = r#"Sure: {"results":[{"id":"pr1","records":[
            {"decision":"Keep the cache on SQLite; don't add Redis.","reason":"one less service","scope":[],"category":"architecture","supersedes":"D-0001","forbid_deps":["redis"]},
            {"decision":"Keep the cache on SQLite and don't add Redis","scope":["src/**"]}]},
            {"id":"pr2","records":[{"decision":"Use apiClient for HTTP calls.","supersedes":"D-9999"}]}]}"#;
        let known = vec![Record {
            id: "D-0001".into(),
            decision: "Cache in memory only.".into(),
            status: Status::Accepted,
            ..Default::default()
        }];
        let out = extract(
            &Fake(reply.into()),
            &[text("pr1"), text("pr2")],
            &known,
            &[],
            8,
            2,
        );
        assert_eq!(out.proposals.len(), 2, "{:?}", out.proposals);
        assert_eq!(out.duplicates, 1);
        let p = &out.proposals[0];
        assert!(p.id.starts_with("P-"));
        assert_eq!(p.supersedes.as_deref(), Some("D-0001"));
        assert_eq!(
            p.scope,
            vec!["src/cache.ts"],
            "falls back to the text's files"
        );
        assert_eq!(p.checks.forbid_deps, vec!["redis"]);
        assert_eq!(p.date, "2026-03-04");
        assert_eq!(out.proposals[1].supersedes, None, "unknown ids are dropped");
    }

    #[test]
    fn failed_calls_are_counted() {
        let out = extract(&Broken, &[text("pr1")], &[], &[], 8, 1);
        assert_eq!((out.failed_calls, out.proposals.len()), (1, 0));
    }

    #[test]
    fn similarity() {
        assert!(similar(
            "Use apiClient for all HTTP calls",
            "Use apiClient for HTTP calls"
        ));
        assert!(!similar(
            "Use apiClient for HTTP calls",
            "Keep the cache on SQLite"
        ));
    }

    #[test]
    fn opposite_polarity_and_other_versions_are_not_duplicates() {
        assert!(!similar(
            "Use SQLite for the cache",
            "Do not use SQLite for the cache"
        ));
        assert!(!similar(
            "Use SQLite for the cache",
            "Don't use SQLite for the cache"
        ));
        assert!(!similar("Support Python 3.10", "Support Python 3.12"));
        assert!(!similar(
            "Run CI against Python 3.10 on every pull request build",
            "Run CI against Python 3.12 on every pull request build"
        ));
        assert!(similar(
            "Do not use axios for HTTP calls",
            "Never use axios for any HTTP calls"
        ));
        assert!(similar("Support Python 3.12.", "Support Python 3.12"));
    }

    #[test]
    fn a_superseding_proposal_is_never_a_duplicate() {
        let reply = r#"{"results":[{"id":"pr1","records":[
            {"decision":"Use SQLite for the whole cache","supersedes":"D-0001"}]}]}"#;
        let known = vec![Record {
            id: "D-0001".into(),
            decision: "Use SQLite for the cache".into(),
            status: Status::Accepted,
            ..Default::default()
        }];
        let out = extract(&Fake(reply.into()), &[text("pr1")], &known, &[], 8, 1);
        assert_eq!((out.proposals.len(), out.duplicates), (1, 0));
        assert_eq!(out.proposals[0].supersedes.as_deref(), Some("D-0001"));
    }
}
