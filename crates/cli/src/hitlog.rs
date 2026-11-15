//! The check log in `.git/ingrain/hits.jsonl`: every hit and override, used for promotion.

use anyhow::Result;
use ingrain_core::{Mode, store};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

/// Per-rule counts. The same (rule, path, line) seen by several checks in one
/// minute (post-edit, then stop) counts once.
pub fn stats(path: &Path) -> Result<BTreeMap<String, RuleStats>> {
    let entries: Vec<Entry> = store::read(path)?;
    let mut seen = std::collections::HashSet::new();
    let mut out: BTreeMap<String, RuleStats> = BTreeMap::new();
    for e in entries {
        let key = (
            e.rule.clone(),
            e.path.clone(),
            e.line,
            e.time / 60,
            format!("{:?}", e.event),
        );
        if !seen.insert(key) {
            continue;
        }
        let s = out.entry(e.rule.clone()).or_default();
        match e.event {
            Event::Hit => s.hits += 1,
            Event::Override => s.overrides += 1,
            Event::Unresolved => s.unresolved += 1,
        }
        s.last_seen = s.last_seen.max(e.time);
    }
    Ok(out)
}
