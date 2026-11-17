//! Decision records and where they live.
//!
//! ```text
//! decisions/D-0012.yml        accepted (or superseded) decisions, committed
//! decisions/pending/<id>.yml  proposals waiting for review (gitignored)
//! decisions/rejected.yml      rejected proposals, so they aren't proposed again
//! .git/decisions/             local state: extraction cursor, hook sessions, override log
//! ```

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Waiting for a human.
    #[default]
    Proposed,
    /// Live: injected into agent sessions and checked.
    Accepted,
    /// Replaced by a newer decision; kept for history.
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    #[default]
    Convention,
    Constraint,
    ApiChange,
    Architecture,
    Scope,
}

/// An ast-grep pattern the decision forbids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternCheck {
    /// ast-grep language name, e.g. `TypeScript`, `Python`.
    pub language: String,
    pub pattern: String,
}

/// Deterministic checks. Decisions without checks are still injected; they just can't block.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checks {
    /// Module names or prefixes that must not be imported in scope (`node-fetch`, `requests`, `../api`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbid_imports: Vec<String>,
    /// Packages that must not be added to package.json, pyproject.toml, requirements*.txt, Cargo.toml, go.mod.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbid_deps: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbid_patterns: Vec<PatternCheck>,
}

impl Checks {
    pub fn is_empty(&self) -> bool {
        self.forbid_imports.is_empty()
            && self.forbid_deps.is_empty()
            && self.forbid_patterns.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    /// One imperative sentence an agent can follow.
    pub decision: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// Alternatives that were considered and rejected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<String>,
    /// Path globs (`src/billing/**`) or symbol / module names (`apiClient`). Empty or `**` means repo-wide.
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub category: Category,
    #[serde(default)]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Checks::is_empty")]
    pub checks: Checks,
    /// Where the decision was made: PR, review comment, commit, session.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub date: String,
}

impl Record {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let record: Record = serde_yaml::from_str(&text)
            .with_context(|| format!("invalid record {}", path.display()))?;
        if record.decision.trim().is_empty() {
            bail!("{} has an empty `decision`", path.display());
        }
        Ok(record)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_yaml::to_string(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn is_live(&self) -> bool {
        self.status == Status::Accepted
    }

    /// One line for agents and listings.
    pub fn line(&self) -> String {
        if self.reason.is_empty() {
            format!("{}: {}", self.id, self.decision)
        } else {
            format!("{}: {} (why: {})", self.id, self.decision, self.reason)
        }
    }
}

/// A rejected proposal, remembered so extraction doesn't bring it back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rejection {
    pub decision: String,
    pub reason: String,
}

/// The ledger directory of one repository.
#[derive(Debug, Clone)]
pub struct Ledger {
    pub dir: PathBuf,
    /// Where it lives; decides the prefix of accepted ids (`D-`, `L-`, `G-`).
    pub store: ingrain_core::Store,
}

impl Ledger {
    pub const DIR: &'static str = "decisions";

    /// The project ledger: `<root>/decisions`.
    pub fn new(root: &Path) -> Self {
        Self::in_store(root, ingrain_core::Store::Project)
    }

    /// `<base>/decisions` for `store` (see `ingrain_core::Store::dir`).
    pub fn in_store(base: &Path, store: ingrain_core::Store) -> Self {
        Self {
            dir: base.join(Self::DIR),
            store,
        }
    }

    pub fn pending_dir(&self) -> PathBuf {
        self.dir.join("pending")
    }

    pub fn rejected_file(&self) -> PathBuf {
        self.dir.join("rejected.yml")
    }

    /// `decisions/<id>.yml`; ids are checked so they can't point outside the ledger.
    pub fn path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.dir.join(format!("{}.yml", valid_id(id)?)))
    }

    pub fn pending_path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.pending_dir().join(format!("{}.yml", valid_id(id)?)))
    }

    /// Accepted and superseded records, sorted by id.
    pub fn records(&self) -> Result<Vec<Record>> {
        load_dir(&self.dir)
    }

    pub fn live(&self) -> Result<Vec<Record>> {
        Ok(self
            .records()?
            .into_iter()
            .filter(Record::is_live)
            .collect())
    }

    /// Live records, skipping (and reporting) files that don't parse, so one broken file
    /// (say, with merge-conflict markers) doesn't switch off every hook.
    pub fn live_lenient(&self) -> Vec<Record> {
        self.records_lenient()
            .into_iter()
            .filter(Record::is_live)
            .collect()
    }

    /// Accepted and superseded records, skipping (and reporting) files that don't parse.
    pub fn records_lenient(&self) -> Vec<Record> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<Record> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_record_file(p))
            .filter_map(|p| {
                Record::load(&p)
                    .map_err(|e| eprintln!("decisions: skipping {}: {e:#}", p.display()))
                    .ok()
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Live records as committed at `rev`, so a change can't retire a decision and break it
    /// in the same diff.
    pub fn live_at(&self, git: &ingrain_core::Git, rev: &str) -> Result<Vec<Record>> {
        let listing = git
            .try_run(&[
                "ls-tree",
                "--name-only",
                "--end-of-options",
                rev,
                &format!("{}/", Self::DIR),
            ])
            .unwrap_or_default();
        let mut out = Vec::new();
        for path in listing.lines().filter(|p| is_record_file(Path::new(p))) {
            let text = git
                .show(rev, path)
                .with_context(|| format!("reading {path} at {rev}"))?;
            let r: Record = serde_yaml::from_str(&text)
                .with_context(|| format!("invalid record {path} at {rev}"))?;
            if r.is_live() {
                out.push(r);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// A proposal id that isn't taken yet.
    pub fn fresh_pending_id(&self, seed: &str) -> Result<String> {
        // Personal stores mix in their name, so the same text proposed to two stores gets two
        // ids (`accept` finds a proposal's store by its id). Project ids stay as they were.
        let base = match self.store {
            ingrain_core::Store::Project => proposal_id(seed),
            store => proposal_id(&format!("{}\0{seed}", store.name())),
        };
        let mut id = base.clone();
        let mut n = 2;
        while self.pending_path(&id)?.exists() {
            id = format!("{base}-{n}");
            n += 1;
        }
        Ok(id)
    }

    pub fn pending(&self) -> Result<Vec<Record>> {
        load_dir(&self.pending_dir())
    }

    pub fn get(&self, id: &str) -> Result<Record> {
        Record::load(&self.path(id)?).with_context(|| format!("no decision `{id}`"))
    }

    pub fn get_pending(&self, id: &str) -> Result<Record> {
        Record::load(&self.pending_path(id)?).with_context(|| format!("no pending decision `{id}`"))
    }

    /// The next free `D-NNNN` id (`L-` in the local store, `G-` in the global one).
    pub fn next_id(&self) -> Result<String> {
        let prefix = self.store.id_prefix();
        let max = self
            .records()?
            .iter()
            .filter_map(|r| r.id.strip_prefix(prefix)?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        Ok(format!("{prefix}{:04}", max + 1))
    }

    pub fn rejections(&self) -> Result<Vec<Rejection>> {
        let path = self.rejected_file();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&path)?;
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        serde_yaml::from_str(&text).with_context(|| format!("invalid {}", path.display()))
    }

    pub fn add_rejection(&self, r: Rejection) -> Result<()> {
        let mut all = self.rejections()?;
        all.push(r);
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.rejected_file(), serde_yaml::to_string(&all)?)?;
        Ok(())
    }
}

fn valid_id(id: &str) -> Result<&str> {
    let ok = !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        bail!("invalid decision id `{id}`");
    }
    Ok(id)
}

fn load_dir(dir: &Path) -> Result<Vec<Record>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file() && is_record_file(&path) {
            out.push(Record::load(&path)?);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn is_record_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "yml" || e == "yaml")
        && path.file_name().is_some_and(|n| n != "rejected.yml")
}

/// `P-xxxxxxxx`: a stable id for a proposal, from what it says and where it came from.
pub fn proposal_id(seed: &str) -> String {
    // FNV-1a: stable across runs and platforms; enough for proposal file names.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in seed.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("P-{:08x}", h as u32)
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Today's date as `YYYY-MM-DD` (UTC).
pub fn today() -> String {
    date_of(now())
}

/// `YYYY-MM-DD` to unix seconds (start of day, UTC).
pub fn parse_since(date: &str) -> Result<i64> {
    crate::extract::source::parse_time(&format!("{date}T00:00:00Z"))
        .with_context(|| format!("`{date}` is not a YYYY-MM-DD date"))
}

/// Unix seconds to `YYYY-MM-DD` (UTC), Howard Hinnant's civil-from-days.
pub fn date_of(secs: i64) -> String {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_next_id() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::new(dir.path());
        assert_eq!(ledger.next_id().unwrap(), "D-0001");
        let r = Record {
            id: "D-0001".into(),
            decision: "Use apiClient for HTTP calls.".into(),
            scope: vec!["src/**".into()],
            status: Status::Accepted,
            checks: Checks {
                forbid_deps: vec!["axios".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        r.save(&ledger.path("D-0001").unwrap()).unwrap();
        assert_eq!(ledger.next_id().unwrap(), "D-0002");
        let back = ledger.get("D-0001").unwrap();
        assert_eq!(back, r);
        assert_eq!(ledger.live().unwrap().len(), 1);
        assert!(ledger.path("../x").is_err());
    }

    #[test]
    fn personal_stores_number_their_own_ids() {
        let dir = tempfile::tempdir().unwrap();
        let global = Ledger::in_store(dir.path(), ingrain_core::Store::Global);
        assert_eq!(global.next_id().unwrap(), "G-0001");
        let r = Record {
            id: "G-0001".into(),
            decision: "Prefer early returns.".into(),
            status: Status::Accepted,
            ..Default::default()
        };
        r.save(&global.path("G-0001").unwrap()).unwrap();
        assert_eq!(global.next_id().unwrap(), "G-0002");
        let local = Ledger::in_store(&dir.path().join("l"), ingrain_core::Store::Local);
        assert_eq!(local.next_id().unwrap(), "L-0001");
    }

    #[test]
    fn dates() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(1_772_600_767), "2026-03-04");
    }
}
