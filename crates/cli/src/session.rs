//! Per-session hook state in `.git/ingrain/sessions/<id>.json`: the files the agent
//! edited (so the Stop hook only judges the agent's own work, not the human's
//! uncommitted changes) and how often the turn was already sent back.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub edited: BTreeSet<String>,
    #[serde(default)]
    pub stop_attempts: u32,
}

impl Session {
    pub fn load(dir: &Path, id: &str) -> Result<Self> {
        let path = file(dir, id);
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?).unwrap_or_default())
    }

    pub fn save(&self, dir: &Path, id: &str) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(file(dir, id), serde_json::to_string(self)?)?;
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
        s.stop_attempts = 2;
        s.save(dir.path(), "../../etc/x").unwrap();
        assert!(dir.path().join("______etc_x.json").exists());
        let back = Session::load(dir.path(), "../../etc/x").unwrap();
        assert_eq!(back.stop_attempts, 2);
        assert!(back.edited.contains("src/a.ts"));
    }
}
