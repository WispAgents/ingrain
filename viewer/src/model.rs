//! What the viewer shows, read from disk: decisions and learned rules from every store, the
//! proposals waiting for review, and the hit log. Loading runs off the UI thread.

use ingrain_core::{Git, Layout, Mode, RuleFile, Store, rulefile};
use ingrain_ledger::record::Category;
use ingrain_ledger::{Ledger, Record, Status, scope};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One decision record and where it lives.
#[derive(Debug, Clone)]
pub struct Decision {
    pub rec: Record,
    pub store: Store,
    pub path: PathBuf,
    /// In `decisions/pending/`: waiting for a person.
    pub pending: bool,
}

impl Decision {
    pub fn id(&self) -> &str {
        &self.rec.id
    }
    pub fn is_live(&self) -> bool {
        !self.pending && self.rec.status == Status::Accepted
    }
    pub fn is_superseded(&self) -> bool {
        !self.pending && self.rec.status == Status::Superseded
    }
    pub fn enforced(&self) -> bool {
        !self.rec.checks.is_empty()
    }
    /// The line agents are given.
    pub fn agent_line(&self) -> String {
        self.rec.line()
    }
}

/// Hit-log counts for one rule (`.git/ingrain/hits.jsonl`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HitStats {
    /// Distinct violations (same rule, file and matched code count once).
    pub hits: u32,
    pub overrides: u32,
    pub last_seen: i64,
    /// Distinct hits per week for the last [`WEEKS`] weeks, oldest first.
    pub weekly: Vec<u32>,
}

pub const WEEKS: usize = 12;

/// One learned rule and where it lives.
#[derive(Debug, Clone)]
pub struct Rule {
    pub file: RuleFile,
    pub store: Store,
    pub pending: bool,
    pub yaml: String,
    pub stats: HitStats,
}

impl Rule {
    pub fn id(&self) -> &str {
        &self.file.id
    }
    pub fn mode(&self) -> Mode {
        self.file.meta.mode
    }
}

/// Everything the window shows for one repository (or only the personal stores).
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    /// The repository root; `None` when only the global store is open.
    pub root: Option<PathBuf>,
    pub name: String,
    pub branch: String,
    pub decisions: Vec<Decision>,
    pub rules: Vec<Rule>,
    /// Files that didn't parse, with why.
    pub problems: Vec<String>,
    /// Unix seconds the hit log was read at, so ages are relative to the load.
    pub loaded_at: i64,
    /// The commit checked out, so a reload only drops cached history when it moved.
    pub head: String,
    /// Each source and evidence string, and what opening it opens (a URL, or a file inside the
    /// repository), worked out at load so the view never touches the disk.
    pub targets: std::collections::HashMap<String, Option<String>>,
}

impl Workspace {
    pub fn live(&self) -> impl Iterator<Item = &Decision> {
        self.decisions.iter().filter(|d| d.is_live())
    }
    pub fn superseded(&self) -> impl Iterator<Item = &Decision> {
        self.decisions.iter().filter(|d| d.is_superseded())
    }
    pub fn proposals(&self) -> impl Iterator<Item = &Decision> {
        self.decisions.iter().filter(|d| d.pending)
    }
    pub fn accepted_rules(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter().filter(|r| !r.pending)
    }
    pub fn proposed_rules(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter().filter(|r| r.pending)
    }
    pub fn inbox_len(&self) -> usize {
        self.proposals().count() + self.proposed_rules().count()
    }
    pub fn decision(&self, id: &str) -> Option<&Decision> {
        self.decisions.iter().find(|d| d.id() == id)
    }
    pub fn rule(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id() == id)
    }

    /// The supersede chain through `id`, oldest first.
    pub fn lineage(&self, id: &str) -> Vec<&Decision> {
        let mut first = match self.decision(id) {
            Some(d) => d,
            None => return Vec::new(),
        };
        let mut guard = 0;
        while let Some(prev) = first
            .rec
            .supersedes
            .as_deref()
            .and_then(|p| self.decision(p))
        {
            first = prev;
            guard += 1;
            if guard > 64 {
                break;
            }
        }
        let mut chain = vec![first];
        while let Some(next) = chain
            .last()
            .and_then(|d| d.rec.superseded_by.as_deref())
            .and_then(|n| self.decision(n))
        {
            if chain.len() > 64 || chain.iter().any(|d| d.id() == next.id()) {
                break;
            }
            chain.push(next);
        }
        chain
    }

    /// The proposed rule with this id (a proposal may share an id with an accepted rule).
    pub fn proposed_rule(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.pending && r.id() == id)
    }

    /// Live decisions that apply to `path` (repo-relative), as `ingrain decisions for` gives them
    /// to agents, and the accepted rules for its language. `content` is the file's text, for
    /// symbol scopes; the caller reads it off the UI thread.
    pub fn applies_to(
        &self,
        path: &str,
        content: Option<&str>,
    ) -> (Vec<(&Decision, scope::Match)>, Vec<&Rule>) {
        let path = ingrain_core::git::normalize_path(path.trim());
        let records: Vec<Record> = self.live().map(|d| d.rec.clone()).collect();
        let found = scope::applicable(&records, &path, content);
        let decisions = found
            .into_iter()
            .filter_map(|(r, m)| self.decision(&r.id).map(|d| (d, m)))
            .collect();
        let lang = ingrain_engine::lang::detect(&path).map(ingrain_engine::lang::family);
        let rules = self
            .accepted_rules()
            .filter(|r| {
                let rule_lang =
                    ingrain_engine::lang::parse(&r.file.language).map(ingrain_engine::lang::family);
                lang.is_some() && rule_lang == lang
            })
            .collect();
        (decisions, rules)
    }
}

/// Where each store's files are, for loading and for watching.
#[derive(Debug, Clone)]
pub struct Source {
    pub root: Option<PathBuf>,
    /// The repository's own git dir (a worktree's, not the common one): its `HEAD` and reflog.
    pub git_dir: Option<PathBuf>,
    /// `(store, base dir holding decisions/, rules dir)`.
    pub layers: Vec<(Store, PathBuf, PathBuf)>,
    pub hits: Option<PathBuf>,
}

impl Source {
    /// The stores for the repository at (or above) `dir`; only the global store when `dir` is `None`
    /// or not in a repository.
    pub fn open(dir: Option<&Path>) -> Source {
        let git = dir.and_then(|d| Git::open(d).ok());
        match git {
            Some(git) => {
                let layers = Store::ALL
                    .into_iter()
                    .filter_map(|store| {
                        let base = store.dir(&git)?;
                        let layout = Layout::for_store(&git, store).ok()?;
                        Some((store, base, layout.rules_dir()))
                    })
                    .collect();
                let hits = Layout::for_repo(&git).ok().map(|l| l.hits_file());
                Source {
                    root: Some(git.root().to_path_buf()),
                    git_dir: Some(git.git_dir().to_path_buf()),
                    layers,
                    hits,
                }
            }
            None => Source {
                root: None,
                git_dir: None,
                layers: ingrain_core::layout::global_dir()
                    .map(|d| {
                        vec![(
                            Store::Global,
                            d.clone(),
                            d.join("corrections").join("rules"),
                        )]
                    })
                    .unwrap_or_default(),
                hits: None,
            },
        }
    }

    /// The directories and files whose change means a reload.
    pub fn watched(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for (_, base, rules) in &self.layers {
            let ledger = base.join(Ledger::DIR);
            out.push(ledger.join("pending"));
            out.push(ledger);
            out.push(
                rules
                    .parent()
                    .map(|p| p.join("pending"))
                    .unwrap_or_default(),
            );
            out.push(rules.clone());
        }
        out.extend(self.hits.clone());
        if let Some(git_dir) = &self.git_dir {
            // HEAD moves on checkout; the reflog grows on every commit.
            out.push(git_dir.join("HEAD"));
            out.push(git_dir.join("logs").join("HEAD"));
        }
        out
    }

    pub fn load(&self) -> Workspace {
        let mut ws = Workspace {
            root: self.root.clone(),
            loaded_at: now(),
            ..Default::default()
        };
        match &self.root {
            Some(root) => {
                ws.name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let git = Git::open(root).ok();
                let rev = |args: &[&str]| {
                    git.as_ref()
                        .and_then(|g| g.try_run(args))
                        .map(|b| b.trim().to_owned())
                        .unwrap_or_default()
                };
                ws.branch = rev(&["rev-parse", "--abbrev-ref", "HEAD"]);
                ws.head = rev(&["rev-parse", "HEAD"]);
            }
            None => ws.name = "Personal rules".into(),
        }
        let stats = self
            .hits
            .as_deref()
            .map(|p| hit_stats(p, ws.loaded_at))
            .unwrap_or_default();
        for (store, base, rules_dir) in &self.layers {
            let ledger = Ledger::in_store(base, *store);
            load_records(&ledger.dir, *store, false, &mut ws);
            load_records(&ledger.pending_dir(), *store, true, &mut ws);
            load_rules(rules_dir, *store, false, &stats, &mut ws);
            if let Some(corrections) = rules_dir.parent() {
                load_rules(&corrections.join("pending"), *store, true, &stats, &mut ws);
            }
        }
        let sources: Vec<String> = ws
            .decisions
            .iter()
            .flat_map(|d| d.rec.sources.iter())
            .chain(ws.rules.iter().flat_map(|r| r.file.meta.evidence.iter()))
            .cloned()
            .collect();
        for s in sources {
            let target = crate::actions::source_target(self.root.as_deref(), &s);
            ws.targets.insert(s, target);
        }
        ws
    }
}

fn load_records(dir: &Path, store: Store, pending: bool, ws: &mut Workspace) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
        .filter(|p| p.file_name().is_some_and(|n| n != "rejected.yml"))
        .collect();
    paths.sort();
    for path in paths {
        match Record::load(&path) {
            Ok(rec) => ws.decisions.push(Decision {
                rec,
                store,
                path,
                pending,
            }),
            Err(e) => ws.problems.push(format!("{e:#}")),
        }
    }
}

fn load_rules(
    dir: &Path,
    store: Store,
    pending: bool,
    stats: &BTreeMap<String, HitStats>,
    ws: &mut Workspace,
) {
    if !dir.is_dir() {
        return;
    }
    match rulefile::load_dir(dir) {
        Ok(files) => {
            for file in files {
                if ws
                    .rules
                    .iter()
                    .any(|r| r.id() == file.id && r.pending == pending)
                {
                    continue; // the first store wins on an id clash, as in the CLI
                }
                let yaml = std::fs::read_to_string(&file.path).unwrap_or_default();
                let stats = stats.get(&file.id).cloned().unwrap_or_default();
                ws.rules.push(Rule {
                    file,
                    store,
                    pending,
                    yaml,
                    stats,
                });
            }
        }
        Err(e) => ws.problems.push(format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct HitEntry {
    time: i64,
    rule: String,
    path: String,
    event: String,
    #[serde(default)]
    matched: String,
}

/// Per-rule counts from the hit log. A violation (rule, file, matched code) counts once.
pub fn hit_stats(path: &Path, now: i64) -> BTreeMap<String, HitStats> {
    let mut out: BTreeMap<String, HitStats> = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    let mut seen = std::collections::HashSet::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(e) = serde_json::from_str::<HitEntry>(line) else {
            continue;
        };
        let s = out.entry(e.rule.clone()).or_insert_with(|| HitStats {
            weekly: vec![0; WEEKS],
            ..Default::default()
        });
        s.last_seen = s.last_seen.max(e.time);
        match e.event.as_str() {
            "hit" => {
                if seen.insert((e.rule.clone(), e.path.clone(), e.matched.clone())) {
                    s.hits += 1;
                    let age_weeks = (now - e.time).max(0) / (7 * 86_400);
                    if (age_weeks as usize) < WEEKS {
                        s.weekly[WEEKS - 1 - age_weeks as usize] += 1;
                    }
                }
            }
            "override" => s.overrides += 1,
            _ => {}
        }
    }
    out
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn category_name(c: Category) -> &'static str {
    match c {
        Category::Convention => "convention",
        Category::Constraint => "constraint",
        Category::ApiChange => "API change",
        Category::Architecture => "architecture",
        Category::Scope => "scope",
    }
}

pub const CATEGORIES: [Category; 5] = [
    Category::Architecture,
    Category::Constraint,
    Category::Convention,
    Category::ApiChange,
    Category::Scope,
];

/// `YYYY-MM-DD` → `(year, month)`.
pub fn year_month(date: &str) -> Option<(i32, u32)> {
    let mut it = date.split('-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    (1..=12).contains(&m).then_some((y, m))
}

pub fn month_name(m: u32) -> &'static str {
    [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ][(m.clamp(1, 12) - 1) as usize]
}

/// Days since the civil epoch for `YYYY-MM-DD` (Howard Hinnant's days-from-civil).
pub fn days_of(date: &str) -> Option<i64> {
    let mut it = date.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.get(..2)?.parse().ok()?;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// "today", "3 days ago", "2 months ago" for a `YYYY-MM-DD` date.
pub fn age(date: &str, now: i64) -> String {
    let Some(days) = days_of(date) else {
        return String::new();
    };
    ago(now / 86_400 - days)
}

/// Ages for unix seconds.
pub fn age_secs(secs: i64, now: i64) -> String {
    if secs <= 0 {
        return "never".into();
    }
    let d = (now - secs) / 86_400;
    if d == 0 {
        let h = (now - secs) / 3600;
        return match h {
            0 => "just now".into(),
            1 => "an hour ago".into(),
            h => format!("{h} hours ago"),
        };
    }
    ago(d)
}

fn ago(days: i64) -> String {
    match days {
        i64::MIN..=0 => "today".into(),
        1 => "yesterday".into(),
        2..=13 => format!("{days} days ago"),
        14..=59 => format!("{} weeks ago", days / 7),
        60..=729 => format!("{} months ago", days / 30),
        _ => format!("{} years ago", days / 365),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(ok.success());
        dir
    }

    #[test]
    fn loads_live_superseded_and_pending_with_lineage() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = repo();
        let d = dir.path().join("decisions");
        write(
            &d.join("D-0001.yml"),
            "id: D-0001\ndecision: Use fetch.\nstatus: superseded\nsuperseded_by: D-0002\nscope: ['src/**']\ndate: 2026-01-03\n",
        );
        write(
            &d.join("D-0002.yml"),
            "id: D-0002\ndecision: Use apiClient.\nstatus: accepted\nsupersedes: D-0001\nscope: ['src/**']\ndate: 2026-03-10\n",
        );
        write(
            &d.join("pending/P-1.yml"),
            "id: P-1\ndecision: Prefer tabs.\nstatus: proposed\n",
        );
        write(&d.join("broken.yml"), "decision: ''\n");
        unsafe { std::env::set_var("INGRAIN_HOME", dir.path().join("home")) };
        let ws = Source::open(Some(dir.path())).load();
        assert_eq!(ws.live().count(), 1);
        assert_eq!(ws.superseded().count(), 1);
        assert_eq!(ws.inbox_len(), 1);
        assert_eq!(ws.problems.len(), 1, "{:?}", ws.problems);
        let chain: Vec<_> = ws
            .lineage("D-0002")
            .iter()
            .map(|d| d.id().to_owned())
            .collect();
        assert_eq!(chain, ["D-0001", "D-0002"]);
        let (applies, _) = ws.applies_to("src/a.ts", None);
        assert_eq!(
            applies.iter().map(|(d, _)| d.id()).collect::<Vec<_>>(),
            ["D-0002"]
        );
        assert!(ws.applies_to("docs/x.md", None).0.is_empty());
    }

    #[test]
    fn hit_stats_count_distinct_violations_by_week() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hits.jsonl");
        let now = 100 * 86_400 * 7;
        let line = |t: i64, ev: &str, path: &str| {
            format!(
                r#"{{"time":{t},"rule":"r","path":"{path}","line":1,"mode":"shadow","event":"{ev}","context":"hook","matched":"x"}}"#
            )
        };
        write(
            &p,
            &[
                line(now - 10, "hit", "a"),
                line(now - 5, "hit", "a"),
                line(now - 8 * 86_400, "hit", "b"),
                line(now, "override", "a"),
            ]
            .join("\n"),
        );
        let s = &hit_stats(&p, now)["r"];
        assert_eq!((s.hits, s.overrides, s.last_seen), (2, 1, now));
        assert_eq!(s.weekly[WEEKS - 1], 1);
        assert_eq!(s.weekly[WEEKS - 2], 1);
    }

    #[test]
    fn dates() {
        assert_eq!(days_of("1970-01-02"), Some(1));
        assert_eq!(
            age("2026-11-10", days_of("2026-11-17").unwrap() * 86_400),
            "7 days ago"
        );
        assert_eq!(year_month("2026-11-17"), Some((2026, 11)));
    }
}
