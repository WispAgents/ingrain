//! The repository a command works on.

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ingrain_core::{Config, DiffBase, Git, Layout, RuleFile, Store, rulefile};
use ingrain_engine::CompiledRule;
use ingrain_ledger::{Ledger, Record};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The store `--local` / `--global` picked for this run.
static CHOSEN: OnceLock<Store> = OnceLock::new();

/// Make `store` the one this run writes to (from `--local` / `--global`).
pub fn choose_store(store: Store) {
    let _ = CHOSEN.set(store);
}

/// The store `--local` / `--global` chose, if any.
pub fn chosen() -> Option<Store> {
    CHOSEN.get().copied()
}

/// True when a personal store has accepted decisions or rules. Cheap: hooks call it on edits.
pub fn personal_in_use(git: &Git) -> bool {
    let has_yaml = |dir: PathBuf| {
        std::fs::read_dir(dir).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                let p = e.path();
                p.is_file()
                    && p.extension().is_some_and(|x| x == "yml" || x == "yaml")
                    && p.file_name().is_some_and(|n| n != "rejected.yml")
            })
        })
    };
    [Store::Local, Store::Global].into_iter().any(|store| {
        store.dir(git).is_some_and(|base| {
            has_yaml(base.join(Ledger::DIR)) || has_yaml(base.join("corrections").join("rules"))
        })
    })
}

/// One store's decisions and rules, as read.
struct Layer {
    store: Store,
    ledger: Ledger,
    rules_dir: PathBuf,
}

pub struct Repo {
    pub git: Git,
    /// Where this run's rules and config live (`.corrections/` for the project store).
    pub layout: Layout,
    pub config: Config,
    /// The store this run writes to: `--local`, `--global`, or the repository's own.
    pub store: Store,
    /// The decision ledger this run writes to (`decisions/` for the project store).
    pub ledger: Ledger,
    /// Every store, project first; reads (hooks, checks, `decisions for`) see them all.
    layers: Vec<Layer>,
    ignore: GlobSet,
    /// When set, accepted rules and config come from this revision instead of the working
    /// tree (pre-commit and CI judge a change by what it started from).
    pinned: Option<String>,
}

/// The store writes go to: the one chosen with `--local` / `--global`; else the project store
/// when the repository has `decisions/` or `.corrections/`; else the local store when
/// `ingrain init --local` set it up; else the project store.
fn target_store(git: &Git) -> Store {
    if let Some(store) = CHOSEN.get() {
        return *store;
    }
    let root = git.root();
    if root.join(Ledger::DIR).is_dir() || root.join(ingrain_core::layout::CORRECTIONS_DIR).is_dir()
    {
        return Store::Project;
    }
    if ingrain_core::layout::local_dir(git).is_dir() {
        return Store::Local;
    }
    Store::Project
}

fn layers(git: &Git) -> Vec<Layer> {
    Store::ALL
        .into_iter()
        .filter_map(|store| {
            let base = store.dir(git)?;
            let layout = Layout::for_store(git, store).ok()?;
            Some(Layer {
                store,
                ledger: Ledger::in_store(&base, store),
                rules_dir: layout.rules_dir(),
            })
        })
        .collect()
}

impl Repo {
    pub fn open(dir: &Path) -> Result<Self> {
        let git = Git::open(dir)?;
        let store = target_store(&git);
        let layout = Layout::for_store(&git, store)?;
        let config = Config::load(&layout.config_file())?;
        Self::with_config(git, layout, config)
    }

    /// Like `open`, but a broken `.corrections/config.yml` falls back to the defaults (with a
    /// warning) instead of failing. Hooks use this: a config an agent broke must not switch off
    /// every hook. Pre-commit and CI use `open`, so a broken config still fails there.
    pub fn open_lenient(dir: &Path) -> Result<Self> {
        let git = Git::open(dir)?;
        let layout = Layout::for_store(&git, target_store(&git))?;
        let config = Config::load(&layout.config_file()).unwrap_or_else(|e| {
            eprintln!("ingrain: {e:#}; using the default config");
            Config::default()
        });
        Self::with_config(git.clone(), layout.clone(), config).or_else(|e| {
            eprintln!("ingrain: {e:#}; using the default config");
            Self::with_config(git, layout, Config::default())
        })
    }

    fn with_config(git: Git, layout: Layout, config: Config) -> Result<Self> {
        let ignore = ignore_set(&config)?;
        let store = target_store(&git);
        let layers = layers(&git);
        let ledger = layers
            .iter()
            .find(|l| l.store == store)
            .map(|l| l.ledger.clone())
            .context("no home directory for the global store; set INGRAIN_HOME")?;
        Ok(Self {
            git,
            layout,
            config,
            store,
            ledger,
            layers,
            ignore,
            pinned: None,
        })
    }

    /// Live decisions from every store. Pre-commit and CI pin the project's to the base revision
    /// with `decisions_at`; personal stores aren't in git, so their files count as they are. A
    /// broken file in a personal store is reported and skipped: it must not fail checks in
    /// every repository the user works in.
    pub fn decisions(&self) -> Result<Vec<Record>> {
        let mut out = Vec::new();
        for layer in &self.layers {
            if layer.store == Store::Project {
                out.extend(layer.ledger.live()?);
            } else {
                out.extend(layer.ledger.live_lenient());
            }
        }
        Ok(out)
    }

    /// Like `decisions`, skipping files that don't parse (hooks).
    pub fn decisions_lenient(&self) -> Vec<Record> {
        self.layers
            .iter()
            .flat_map(|l| l.ledger.live_lenient())
            .collect()
    }

    /// The project's live decisions as committed at `rev`, plus the personal stores' as they are.
    pub fn decisions_at(&self, rev: &str) -> Result<Vec<Record>> {
        let mut out = Vec::new();
        for layer in &self.layers {
            if layer.store == Store::Project {
                out.extend(layer.ledger.live_at(&self.git, rev)?);
            } else {
                out.extend(layer.ledger.live_lenient());
            }
        }
        Ok(out)
    }

    /// Accepted and superseded records from every store; broken files in personal stores are
    /// reported and skipped.
    pub fn all_records(&self) -> Result<Vec<Record>> {
        let mut out = Vec::new();
        for layer in &self.layers {
            if layer.store == Store::Project {
                out.extend(layer.ledger.records()?);
            } else {
                out.extend(layer.ledger.records_lenient());
            }
        }
        Ok(out)
    }

    /// Every store's ledger, project first.
    pub fn ledgers(&self) -> impl Iterator<Item = &Ledger> {
        self.layers.iter().map(|l| &l.ledger)
    }

    /// The ledger holding `id`: by prefix for accepted ids (`D-`, `L-`, `G-`); for a proposal,
    /// the store whose `pending/` has it, else the store this run writes to.
    pub fn ledger_of(&self, id: &str) -> &Ledger {
        let by_prefix = Store::of_id(id).and_then(|s| self.layers.iter().find(|l| l.store == s));
        let by_pending = || {
            self.layers.iter().find(|l| {
                l.store != self.store && l.ledger.pending_path(id).is_ok_and(|p| p.is_file())
            })
        };
        match by_prefix {
            Some(layer) => &layer.ledger,
            None if self.ledger.pending_path(id).is_ok_and(|p| p.is_file()) => &self.ledger,
            None => by_pending().map_or(&self.ledger, |l| &l.ledger),
        }
    }

    /// True when any store has accepted rules or decisions, or the project has `.corrections/`:
    /// ingrain was chosen for this repository (or for every repository).
    pub fn is_set_up(&self) -> bool {
        self.git
            .root()
            .join(ingrain_core::layout::CORRECTIONS_DIR)
            .is_dir()
            || !self.decisions_lenient().is_empty()
            || self.layers.iter().any(|l| {
                l.store != Store::Project
                    && rulefile::load_dir(&l.rules_dir).is_ok_and(|r| !r.is_empty())
            })
    }

    /// Stage `path` when it's in the project store; personal stores stay out of git.
    pub fn stage(&self, path: &Path) -> Result<bool> {
        let root = self.git.root();
        let project = path.starts_with(root.join(Ledger::DIR))
            || path.starts_with(root.join(ingrain_core::layout::CORRECTIONS_DIR));
        if !project {
            return Ok(false);
        }
        let rel = self.git.relative(path);
        self.git.run(&["add", "--", &rel])?;
        Ok(true)
    }

    /// Take accepted rules and config from `rev` from now on. Without this, a change could
    /// delete a rule, set it to shadow or ignore its path, and break it in the same diff.
    ///
    /// A config at `rev` that doesn't parse is reported and the working tree's is kept: the
    /// commit that repairs it must not be blocked by it.
    pub fn pin_to(&mut self, rev: &str) -> Result<()> {
        self.pinned = Some(rev.to_owned());
        if self.store != Store::Project {
            // The config is in a personal store, outside git.
            return Ok(());
        }
        let config = match self.git.show(rev, ".corrections/config.yml") {
            Some(text) => serde_yaml::from_str::<Config>(&text)
                .map_err(anyhow::Error::from)
                .and_then(|c| ignore_set(&c).map(|set| (c, set))),
            None => ignore_set(&Config::default()).map(|set| (Config::default(), set)),
        };
        match config {
            Ok((config, set)) => {
                self.config = config;
                self.ignore = set;
            }
            Err(e) => eprintln!(
                "ingrain: .corrections/config.yml at {rev} is broken ({e:#}); using the working tree's"
            ),
        }
        Ok(())
    }

    /// Paths in `config.ignore` are never mined or checked.
    pub fn is_ignored(&self, path: &str) -> bool {
        self.ignore.is_match(path)
    }

    /// Accepted rules from every store; on an id clash the project's wins, then the local one.
    pub fn accepted(&self) -> Result<Vec<RuleFile>> {
        let mut rules = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for layer in &self.layers {
            let found = if layer.store == Store::Project {
                self.project_rules()?
            } else {
                personal_rules(&layer.rules_dir)
            };
            for rule in found {
                if seen.insert(rule.id.clone()) {
                    rules.push(rule);
                }
            }
        }
        rules.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rules)
    }

    /// Rules accepted in this run's store only (what `rules mode` and `rules review` change).
    pub fn accepted_here(&self) -> Result<Vec<RuleFile>> {
        if self.store == Store::Project {
            self.project_rules()
        } else {
            rulefile::load_dir(&self.layout.rules_dir())
        }
    }

    fn project_rules(&self) -> Result<Vec<RuleFile>> {
        let dir = self
            .git
            .root()
            .join(ingrain_core::layout::CORRECTIONS_DIR)
            .join("rules");
        let Some(rev) = &self.pinned else {
            return rulefile::load_dir(&dir);
        };
        let listing = self
            .git
            .try_run(&[
                "ls-tree",
                "--name-only",
                "--end-of-options",
                rev,
                ".corrections/rules/",
            ])
            .unwrap_or_default();
        let mut rules = Vec::new();
        for path in listing
            .lines()
            .filter(|p| p.ends_with(".yml") || p.ends_with(".yaml"))
        {
            let text = self
                .git
                .show(rev, path)
                .with_context(|| format!("reading {path} at {rev}"))?;
            // A broken rule is skipped, as in `compiled_rules`, so the fix can be committed.
            match RuleFile::parse(&text, self.git.root().join(path)) {
                Ok(rule) => rules.push(rule),
                Err(e) => eprintln!("ingrain: skipping {path} at {rev}: {e:#}"),
            }
        }
        rules.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rules)
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

    /// Files with added lines as decision-check targets. Unlike learned rules, decisions also
    /// check files ast-grep can't parse (Cargo.toml, go.mod, requirements.txt), and
    /// `config.ignore` doesn't apply to them. Manifests also get their content before the change,
    /// so a check sees which dependencies were added.
    pub fn decision_targets(
        &self,
        base: &DiffBase,
        only: Option<&[String]>,
    ) -> Result<Vec<ingrain_ledger::Target>> {
        use ingrain_core::git::Content;
        let files: Vec<(String, Vec<ingrain_core::LineRange>)> = match only {
            Some(paths) => paths
                .iter()
                .filter_map(|p| match self.git.added_lines_for(base, p) {
                    Ok(added) => Some((p.clone(), added)),
                    Err(e) => {
                        eprintln!("ingrain: skipping {p}: {e:#}");
                        None
                    }
                })
                .collect(),
            None => self.git.added_lines(base)?,
        };
        let side = base.content();
        let before_rev = base.before_rev(&self.git);
        let mut out = Vec::new();
        for (path, added) in files {
            if added.is_empty() || path.starts_with("decisions/") {
                continue;
            }
            let content = match &side {
                Content::WorkingTree => self.read(&path),
                Content::Index => self.git.show_index(&path),
                Content::Revision(rev) => self.git.show(rev, &path),
            };
            let Some(content) = content else { continue };
            let before = if ingrain_ledger::manifest::is_manifest(&path) {
                before_rev
                    .as_deref()
                    .and_then(|rev| self.git.show(rev, &path))
            } else {
                None
            };
            out.push(ingrain_ledger::Target {
                path,
                content,
                added: Some(added),
                before,
            });
        }
        Ok(out)
    }
}

/// Rules in a personal store; a file that doesn't parse is reported and skipped, so one broken
/// file can't fail checks in every repository.
fn personal_rules(dir: &Path) -> Vec<RuleFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rules: Vec<RuleFile> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
        .filter_map(|p| {
            RuleFile::load(&p)
                .map_err(|e| eprintln!("ingrain: skipping {}: {e:#}", p.display()))
                .ok()
        })
        .collect();
    rules.sort_by(|a, b| a.id.cmp(&b.id));
    rules
}

fn ignore_set(config: &Config) -> Result<GlobSet> {
    let mut globs = GlobSetBuilder::new();
    for pattern in &config.ignore {
        globs.add(Glob::new(pattern).with_context(|| format!("bad ignore glob `{pattern}`"))?);
    }
    Ok(globs.build()?)
}
