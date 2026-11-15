//! Where ingrain keeps its files inside a repository.
//!
//! ```text
//! .corrections/              committed, reviewed like code
//!   config.yml
//!   rules/<id>.yml           accepted rules
//!   pending/<id>.yml         proposals (gitignored)
//!   rejected.yml             rejected proposals with reasons
//! .git/ingrain/              local data, never committed
//!   corrections.jsonl
//!   agent_hunks.jsonl
//!   hits.jsonl               check log: hits, fixes, overrides
//! ```

use crate::git::Git;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub const CORRECTIONS_DIR: &str = ".corrections";

#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
    pub data: PathBuf,
}

impl Layout {
    pub fn new(root: PathBuf, data: PathBuf) -> Self {
        Self { root, data }
    }

    /// Layout for the repository that `git` points at.
    pub fn for_repo(git: &Git) -> Result<Self> {
        Ok(Self::new(
            git.root().to_path_buf(),
            git.git_dir().join("ingrain"),
        ))
    }

    pub fn corrections_dir(&self) -> PathBuf {
        self.root.join(CORRECTIONS_DIR)
    }
    pub fn config_file(&self) -> PathBuf {
        self.corrections_dir().join("config.yml")
    }
    pub fn rules_dir(&self) -> PathBuf {
        self.corrections_dir().join("rules")
    }
    pub fn pending_dir(&self) -> PathBuf {
        self.corrections_dir().join("pending")
    }
    pub fn rejected_file(&self) -> PathBuf {
        self.corrections_dir().join("rejected.yml")
    }
    /// `<dir>/<id>.yml`. Ids are slugs; anything else (`../x`) is rejected so a rule
    /// id can't point outside `.corrections/`.
    pub fn rule_path(&self, dir: &Path, id: &str) -> Result<PathBuf> {
        let valid = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            && !id.starts_with('.');
        if !valid {
            anyhow::bail!("invalid rule id `{id}` (use letters, digits, `-`, `_`)");
        }
        Ok(dir.join(format!("{id}.yml")))
    }

    pub fn corrections_file(&self) -> PathBuf {
        self.data.join("corrections.jsonl")
    }
    pub fn agent_hunks_file(&self) -> PathBuf {
        self.data.join("agent_hunks.jsonl")
    }
    pub fn hits_file(&self) -> PathBuf {
        self.data.join("hits.jsonl")
    }
    pub fn sessions_dir(&self) -> PathBuf {
        self.data.join("sessions")
    }
}
