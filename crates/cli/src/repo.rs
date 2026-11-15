//! The repository a command works on.

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ingrain_core::{Config, DiffBase, Git, Layout, RuleFile, rulefile};
use ingrain_engine::CompiledRule;
use ingrain_ledger::Ledger;
use std::path::{Path, PathBuf};

pub struct Repo {
    pub git: Git,
    pub layout: Layout,
    pub config: Config,
    /// The decision ledger in `decisions/`.
    pub ledger: Ledger,
    ignore: GlobSet,
}

impl Repo {
    pub fn open(dir: &Path) -> Result<Self> {
        let git = Git::open(dir)?;
        let layout = Layout::for_repo(&git)?;
        let config = Config::load(&layout.config_file())?;
        let mut globs = GlobSetBuilder::new();
        for pattern in &config.ignore {
            globs.add(Glob::new(pattern).with_context(|| format!("bad ignore glob `{pattern}`"))?);
        }
        let ignore = globs.build()?;
        let ledger = Ledger::new(git.root());
        Ok(Self {
            git,
            layout,
            config,
            ledger,
            ignore,
        })
    }

    /// Paths in `config.ignore` are never mined or checked.
    pub fn is_ignored(&self, path: &str) -> bool {
        self.ignore.is_match(path)
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
        let path = self.layout.rule_path(&self.layout.pending_dir(), id)?;
        RuleFile::load(&path).with_context(|| format!("no pending rule `{id}`"))
    }

    pub fn find_accepted(&self, id: &str) -> Result<RuleFile> {
        let path = self.layout.rule_path(&self.layout.rules_dir(), id)?;
        RuleFile::load(&path).with_context(|| format!("no accepted rule `{id}`"))
    }

    /// Local state shared by rules and decisions: `.git/ingrain/`.
    pub fn state_dir(&self) -> PathBuf {
        self.layout.data.clone()
    }

    /// A working-tree file, if it exists.
    pub fn read(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.git.root().join(rel)).ok()
    }

    /// The same files as decision-check targets. Manifests also get their content before the
    /// change, so a check sees which dependencies were added.
    pub fn decision_targets(
        &self,
        targets: &[crate::check::Target],
        base: &DiffBase,
    ) -> Vec<ingrain_ledger::Target> {
        let before_rev = base.before_rev(&self.git);
        targets
            .iter()
            .filter(|t| !t.path.starts_with("decisions/"))
            .map(|t| ingrain_ledger::Target {
                path: t.path.clone(),
                content: t.content.clone(),
                added: Some(t.added.clone()),
                before: if ingrain_ledger::manifest::is_manifest(&t.path) {
                    before_rev
                        .as_deref()
                        .and_then(|rev| self.git.show(rev, &t.path))
                } else {
                    None
                },
            })
            .collect()
    }
}
