//! `.corrections/config.yml`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Commit trailers or author names/emails that mark a commit as agent-written
    /// (case-insensitive substring match). Added to the built-in list.
    pub agent_markers: Vec<String>,
    /// A human commit counts as a correction only within this many days of the agent commit.
    pub window_days: u32,
    /// Minimum corrections in a cluster before a rule is proposed.
    pub min_support: u32,
    /// Minimum share of flagged agent hunks that were actually corrected.
    pub min_precision: f64,
    /// Path globs never mined or checked.
    pub ignore: Vec<String>,
    /// Promotion: hits needed, and the maximum override share, to move up a mode.
    pub promote_min_hits: u32,
    pub promote_max_override_rate: f64,
    /// Stop hook: how many times a turn can be sent back before it is let through.
    pub stop_max_attempts: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agent_markers: Vec::new(),
            window_days: 14,
            min_support: 3,
            min_precision: 0.5,
            ignore: vec![
                "**/node_modules/**".into(),
                "**/dist/**".into(),
                "**/vendor/**".into(),
                "**/*.min.js".into(),
                "**/*.lock".into(),
            ],
            promote_min_hits: 5,
            promote_max_override_rate: 0.1,
            stop_max_attempts: 3,
        }
    }
}

impl Config {
    /// Load the config, or defaults when the file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        serde_yaml::from_str(&text).with_context(|| format!("invalid {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_yaml::to_string(self)?)?;
        Ok(())
    }
}
