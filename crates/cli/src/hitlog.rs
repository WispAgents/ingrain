//! The check log in `.git/ingrain/hits.jsonl`: every hit and override, used for promotion.

use anyhow::Result;
use ingrain_core::{Correction, Mode, store};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Hit,
    /// The line carries an `ingrain-ignore <rule>: <reason>` comment.
    Override,
    /// The stop hook let a turn end with open block-mode violations.
    Unresolved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub time: i64,
    pub rule: String,
    pub path: String,
    pub line: u32,
    pub mode: Mode,
    pub event: Event,
    /// `hook`, `stop`, `staged`, `ci` or `manual`.
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The matched code, which identifies a violation across line shifts.
    #[serde(default)]
    pub matched: String,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn append(path: &Path, entries: &[Entry]) -> Result<()> {
    store::append(path, entries)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleStats {
    pub hits: u32,
    pub overrides: u32,
    pub unresolved: u32,
    pub last_seen: i64,
    /// Distinct hits whose flagged code a human later removed: a mined correction
    /// (git fix-up, review suggestion, transcript edit) of the same file, made after
    /// the hit, whose agent side contains the matched code and whose fixed side doesn't.
    pub confirmed: u32,
}

impl RuleStats {
    pub fn override_rate(&self) -> f64 {
        let total = self.hits + self.overrides;
        if total == 0 {
            0.0
        } else {
            f64::from(self.overrides) / f64::from(total)
        }
    }
}

/// Per-rule counts of distinct violations: the same matched code in the same file
/// counts once however many checks see it (every edit re-checks the whole file).
pub fn stats(path: &Path) -> Result<BTreeMap<String, RuleStats>> {
    stats_confirmed(path, &[])
}

/// [`stats`], plus which hits mined corrections confirm (see [`RuleStats::confirmed`]).
pub fn stats_confirmed(
    path: &Path,
    corrections: &[Correction],
) -> Result<BTreeMap<String, RuleStats>> {
    let entries: Vec<Entry> = store::read(path)?;
    // Per file: (time, agent side, fixed side), whitespace-normalised.
    let mut fixes: HashMap<&str, Vec<(i64, String, String)>> = HashMap::new();
    for c in corrections {
        fixes.entry(clean_path(&c.path)).or_default().push((
            c.time,
            normalize(&c.before.text),
            normalize(&c.after.text),
        ));
    }
    let mut seen = HashSet::new();
    let mut out: BTreeMap<String, RuleStats> = BTreeMap::new();
    for e in entries {
        let what = if e.matched.is_empty() {
            e.line.to_string()
        } else {
            e.matched.clone()
        };
        let key = (
            e.rule.clone(),
            e.path.clone(),
            what,
            format!("{:?}", e.event),
        );
        // The log is chronological, so the first entry is when the hit was first seen.
        if !seen.insert(key) {
            continue;
        }
        let s = out.entry(e.rule.clone()).or_default();
        match e.event {
            Event::Hit => {
                s.hits += 1;
                if confirmed(&e, &fixes) {
                    s.confirmed += 1;
                }
            }
            Event::Override => s.overrides += 1,
            Event::Unresolved => s.unresolved += 1,
        }
        s.last_seen = s.last_seen.max(e.time);
    }
    Ok(out)
}

/// A human corrected the hit's code after it was flagged. Entries without the matched
/// code (older logs) can't be confirmed.
fn confirmed(e: &Entry, fixes: &HashMap<&str, Vec<(i64, String, String)>>) -> bool {
    let matched = normalize(&e.matched);
    if matched.is_empty() {
        return false;
    }
    fixes.get(clean_path(&e.path)).is_some_and(|fs| {
        fs.iter().any(|(time, before, after)| {
            *time >= e.time && before.contains(&matched) && !after.contains(&matched)
        })
    })
}

fn clean_path(p: &str) -> &str {
    p.strip_prefix("./").unwrap_or(p)
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(line: u32, matched: &str, time: i64) -> Entry {
        Entry {
            time,
            rule: "r".into(),
            path: "a.ts".into(),
            line,
            mode: Mode::Shadow,
            event: Event::Hit,
            context: "hook".into(),
            reason: None,
            matched: matched.into(),
        }
    }

    #[test]
    fn one_violation_seen_by_many_checks_counts_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hits.jsonl");
        // The same fetch, re-checked on every edit while lines shift around it.
        append(
            &path,
            &[
                entry(3, "fetch(u)", 0),
                entry(5, "fetch(u)", 120),
                entry(9, "fetch(u)", 600),
            ],
        )
        .unwrap();
        append(&path, &[entry(2, "fetch(v)", 700)]).unwrap();
        assert_eq!(stats(&path).unwrap()["r"].hits, 2);
    }

    #[test]
    fn hits_a_human_later_removed_are_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hits.jsonl");
        append(
            &path,
            &[
                entry(3, "fetch(u)", 100),
                entry(4, "fetch(v)", 100),
                entry(5, "fetch(w)", 100),
                entry(6, "", 100),
            ],
        )
        .unwrap();
        let fix = |before: &str, after: &str, time: i64| {
            let snip = |t: &str| ingrain_core::Snippet {
                text: t.into(),
                changed: ingrain_core::LineRange::new(1, 1),
                first_line: 1,
            };
            Correction {
                id: format!("{before}{time}"),
                path: "a.ts".into(),
                lang: "TypeScript".into(),
                before: snip(before),
                after: snip(after),
                source: ingrain_core::Source::Transcript {
                    session: "s".into(),
                },
                agent_time: 0,
                time,
                note: None,
            }
        };
        let fixes = [
            // Removed after the hit: confirmed.
            fix("x = fetch(u);", "x = api.get(u);", 200),
            // Before the hit: the hit can't be what made the human fix it.
            fix("x = fetch(v);", "x = api.get(v);", 50),
            // Edited, but the flagged code stayed.
            fix("x = fetch(w);", "y = fetch(w);", 200),
        ];
        let s = &stats_confirmed(&path, &fixes).unwrap()["r"];
        assert_eq!((s.hits, s.confirmed), (4, 1));
        assert_eq!(stats(&path).unwrap()["r"].confirmed, 0);
    }
}
