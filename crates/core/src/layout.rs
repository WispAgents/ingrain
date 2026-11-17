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
//!
//! Decisions and rules can also be personal, kept out of the repository:
//!
//! ```text
//! <git common dir>/ingrain/local/   this repository only (`--local`): never committed
//!   decisions/L-0001.yml
//!   corrections/rules/<id>.yml      same layout as `.corrections/`
//! ~/.config/ingrain/                every repository (`--global`; `$INGRAIN_HOME` overrides)
//!   decisions/G-0001.yml
//!   corrections/rules/<id>.yml
//! ```

use crate::git::Git;
use anyhow::Result;
use std::path::{Path, PathBuf};

pub const CORRECTIONS_DIR: &str = ".corrections";

/// Where a set of decisions and learned rules lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    /// `decisions/` and `.corrections/` in the repository, shared with the team.
    Project,
    /// This repository only, inside `.git/`: never committed.
    Local,
    /// Every repository on this machine: `~/.config/ingrain/`.
    Global,
}

impl Store {
    /// All stores, in the order they are read; on a rule id clash the first wins.
    pub const ALL: [Store; 3] = [Store::Project, Store::Local, Store::Global];

    pub fn name(self) -> &'static str {
        match self {
            Store::Project => "project",
            Store::Local => "local",
            Store::Global => "global",
        }
    }

    /// Prefix of accepted decision ids, so ids from different stores never clash.
    pub fn id_prefix(self) -> &'static str {
        match self {
            Store::Project => "D-",
            Store::Local => "L-",
            Store::Global => "G-",
        }
    }

    /// The store an accepted decision id belongs to.
    pub fn of_id(id: &str) -> Option<Store> {
        Store::ALL.into_iter().find(|s| {
            id.strip_prefix(s.id_prefix())
                .is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
        })
    }

    /// The directory holding this store's `decisions/` and corrections, for `root`'s
    /// repository. `None` for the global store when no home directory is known.
    pub fn dir(self, git: &Git) -> Option<PathBuf> {
        match self {
            Store::Project => Some(git.root().to_path_buf()),
            Store::Local => Some(local_dir(git)),
            Store::Global => global_dir(),
        }
    }
}

/// `<git common dir>/ingrain/local`: shared by all worktrees of the repository.
pub fn local_dir(git: &Git) -> PathBuf {
    git.common_dir().join("ingrain").join("local")
}

/// The global store: `$INGRAIN_HOME`, else `$XDG_CONFIG_HOME/ingrain`, else
/// `%APPDATA%\ingrain` on Windows and `~/.config/ingrain` elsewhere.
pub fn global_dir() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    if let Some(dir) = var("INGRAIN_HOME") {
        return Some(dir);
    }
    if let Some(dir) = var("XDG_CONFIG_HOME") {
        return Some(dir.join("ingrain"));
    }
    if cfg!(windows)
        && let Some(dir) = var("APPDATA")
    {
        return Some(dir.join("ingrain"));
    }
    var("HOME")
        .or_else(|| var("USERPROFILE"))
        .map(|home| home.join(".config").join("ingrain"))
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
    pub data: PathBuf,
    /// `.corrections/`, or `corrections/` in a personal store.
    pub corrections: PathBuf,
}

impl Layout {
    pub fn new(root: PathBuf, data: PathBuf) -> Self {
        let corrections = root.join(CORRECTIONS_DIR);
        Self {
            root,
            data,
            corrections,
        }
    }

    /// Layout for the repository that `git` points at.
    pub fn for_repo(git: &Git) -> Result<Self> {
        Ok(Self::new(
            git.root().to_path_buf(),
            git.git_dir().join("ingrain"),
        ))
    }

    /// Layout whose rules and config are in `store` (local data stays in the repository).
    pub fn for_store(git: &Git, store: Store) -> Result<Self> {
        let mut layout = Self::for_repo(git)?;
        if store != Store::Project {
            let dir = store.dir(git).ok_or_else(|| {
                anyhow::anyhow!("no home directory for the global store; set INGRAIN_HOME")
            })?;
            layout.corrections = dir.join("corrections");
        }
        Ok(layout)
    }

    pub fn corrections_dir(&self) -> PathBuf {
        self.corrections.clone()
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
