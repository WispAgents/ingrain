//! Per-session hook state in `.git/ingrain/sessions/<id>.json`: which decisions the agent was
//! already shown (each is shown once), which files it edited (the Stop hook checks only those,
//! not the human's own uncommitted work), and how often the turn was sent back.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub shown: BTreeSet<String>,
    #[serde(default)]
    pub edited: BTreeSet<String>,
    #[serde(default)]
    pub stop_attempts: u32,
    /// Stashes agent commands filled with edits that weren't the person's: not history.
    #[serde(default)]
    pub tainted_stashes: BTreeSet<String>,
    /// Changed files before each running Bash command, by tool use id: the PostToolUse hook
    /// compares to find the files the command changed.
    #[serde(default)]
    pub bash: BTreeMap<String, BashSnapshot>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BashSnapshot {
    /// Seconds since the epoch; snapshots of commands that never finished are dropped later.
    pub time: u64,
    pub files: BTreeMap<String, String>,
    /// The person was asked to approve this command (so it may change reviewed files).
    #[serde(default)]
    pub asked: bool,
    /// `HEAD` before the command, to see commits it made or undid.
    #[serde(default)]
    pub head: Option<String>,
    /// Staged blob of each reviewed file in the index before the command. `None` in a snapshot
    /// from an older ingrain, whose stamps can't be compared either.
    #[serde(default)]
    pub staged: Option<BTreeMap<String, String>>,
    /// Commits every ref pointed at before the command, and the branch `HEAD` was on: to tell
    /// a move through existing history (checkout, merge, rebase, pull) from new content.
    #[serde(default)]
    pub tips: Option<Vec<String>>,
    #[serde(default)]
    pub stash: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    /// MERGE_HEAD, CHERRY_PICK_HEAD, REBASE_HEAD and ORIG_HEAD before the command.
    #[serde(default)]
    pub pending: Vec<String>,
}

impl Session {
    /// Fold in what parallel hooks recorded meanwhile. `consumed` is the Bash snapshot this
    /// hook used up, which must not come back from disk.
    pub fn merge(&mut self, on_disk: Session, consumed: Option<&str>) {
        self.edited.extend(on_disk.edited);
        self.tainted_stashes.extend(on_disk.tainted_stashes);
        for (id, snap) in on_disk.bash {
            if Some(id.as_str()) != consumed {
                self.bash.entry(id).or_insert(snap);
            }
        }
        // A command whose PostToolUse never came (denied, interrupted) leaves its snapshot.
        let hour_ago = now().saturating_sub(3600);
        self.bash.retain(|_, s| s.time >= hour_ago);
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Session {
    pub fn load(dir: &Path, id: &str) -> Self {
        std::fs::read_to_string(file(dir, id))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Write then rename, so concurrent hooks never read half a file.
    pub fn save(&self, dir: &Path, id: &str) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = file(dir, id);
        let tmp = path.with_extension(format!("json.{}", std::process::id()));
        std::fs::write(&tmp, serde_json::to_string(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// Session ids come from the hook payload; keep only safe filename characters.
fn file(dir: &Path, id: &str) -> PathBuf {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(128)
        .collect();
    let safe = if safe.is_empty() {
        "default".to_owned()
    } else {
        safe
    };
    dir.join(format!("{safe}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_safe_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Session::default();
        s.edited.insert("src/a.ts".into());
        s.shown.insert("D-0001".into());
        s.stop_attempts = 2;
        s.save(dir.path(), "../../etc/x").unwrap();
        assert!(dir.path().join("______etc_x.json").exists());
        let back = Session::load(dir.path(), "../../etc/x");
        assert_eq!(back.stop_attempts, 2);
        assert!(back.edited.contains("src/a.ts") && back.shown.contains("D-0001"));
    }
}
