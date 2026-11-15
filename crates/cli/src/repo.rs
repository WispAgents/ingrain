//! The repository a command works on.

use anyhow::{Context, Result};
use ingrain_core::{Config, Git, Layout, RuleFile, rulefile};
use ingrain_engine::CompiledRule;
use std::path::Path;

pub struct Repo {
    pub git: Git,
    pub layout: Layout,
    pub config: Config,
}

impl Repo {
    pub fn open(dir: &Path) -> Result<Self> {
        let git = Git::open(dir)?;
        let layout = Layout::for_repo(&git)?;
        let config = Config::load(&layout.config_file())?;
        Ok(Self {
            git,
            layout,
            config,
        })
    }

    pub fn accepted(&self) -> Result<Vec<RuleFile>> {
        rulefile::load_dir(&self.layout.rules_dir())
    }

    pub fn pending(&self) -> Result<Vec<RuleFile>> {
        rulefile::load_dir(&self.layout.pending_dir())
    }

    /// Accepted rules, compiled. A broken rule file is reported and skipped so one
    /// bad rule can't take down every check.
    pub fn compiled_rules(&self) -> Result<Vec<(RuleFile, CompiledRule)>> {
        let mut out = Vec::new();
        for rule in self.accepted()? {
            match CompiledRule::compile(&rule.to_yaml()) {
                Ok(c) => out.push((rule, c)),
                Err(e) => eprintln!("ingrain: skipping {}: {e:#}", rule.path.display()),
            }
        }
        Ok(out)
    }

    pub fn find_pending(&self, id: &str) -> Result<RuleFile> {
        let path = self.layout.rule_path(&self.layout.pending_dir(), id);
        RuleFile::load(&path).with_context(|| format!("no pending rule `{id}`"))
    }

    pub fn find_accepted(&self, id: &str) -> Result<RuleFile> {
        let path = self.layout.rule_path(&self.layout.rules_dir(), id);
        RuleFile::load(&path).with_context(|| format!("no accepted rule `{id}`"))
    }
}
