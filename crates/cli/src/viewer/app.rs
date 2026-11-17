//! The viewer's state and what its handlers do.

use crate::viewer::actions::{self, recents};
use crate::viewer::bus::{Bus, Msg};
use crate::viewer::history::{self, Commit};
use crate::viewer::model::{Source, Workspace};
use ingrain_core::{Mode, Store};
use ingrain_ledger::record::Category;
use rune::{TextEdit, Tx};
use rune_kit::diff::{self, DiffState};
use rune_kit::shell::PaneState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lens {
    Inbox,
    Decisions,
    Rules,
    Timeline,
    Applies,
    Archive,
}

impl Lens {
    pub const ALL: [Lens; 6] = [
        Lens::Inbox,
        Lens::Decisions,
        Lens::Rules,
        Lens::Timeline,
        Lens::Applies,
        Lens::Archive,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Lens::Inbox => "Inbox",
            Lens::Decisions => "Decisions",
            Lens::Rules => "Learned rules",
            Lens::Timeline => "Timeline",
            Lens::Applies => "Applies to…",
            Lens::Archive => "Archive",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Lens::Inbox => "lens-inbox",
            Lens::Decisions => "lens-decisions",
            Lens::Rules => "lens-rules",
            Lens::Timeline => "lens-timeline",
            Lens::Applies => "lens-applies",
            Lens::Archive => "lens-archive",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Sel {
    Decision(String),
    Rule(String),
}

/// A form open over the window.
pub enum Sheet {
    /// Supersede an accepted decision: the new sentence and why.
    Supersede {
        id: String,
        decision: TextEdit,
        reason: TextEdit,
    },
    /// A commit's patch to a record.
    Commit {
        title: String,
        diff: DiffState,
        lines: usize,
        /// The patch arrived (it can be empty: a merge, or a rename `--follow` crossed).
        loaded: bool,
    },
    /// A command's output as it runs (extract, mine).
    Log { title: String },
}

pub struct Toast {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub ok: bool,
    pub born: std::time::Instant,
}

pub struct App {
    pub bus: Bus,
    pub source: Option<Source>,
    pub ws: Workspace,
    pub generation: u64,
    pub recents: Vec<PathBuf>,
    /// No repository open: the home screen.
    pub home: bool,

    pub lens: Lens,
    pub sel: Option<Sel>,
    pub filter: TextEdit,
    pub store_filter: Option<Store>,
    pub category_filter: Option<Category>,
    pub applies: TextEdit,
    pub sidebar: PaneState,

    pub history: HashMap<PathBuf, Vec<Commit>>,
    pub history_pending: Option<PathBuf>,

    /// The inbox's current item and, while rejecting it, the reason being typed.
    pub inbox_at: usize,
    pub rejecting: Option<TextEdit>,

    pub sheet: Option<Sheet>,
    pub log: Vec<String>,
    /// Labels of commands still running.
    pub running: Vec<String>,
    pub toasts: Vec<Toast>,
    pub next_toast: u64,
    pub drop_over: bool,
    /// The global store's folder (`~/.config/ingrain`, `$INGRAIN_HOME`), read once.
    pub home_dir: Option<PathBuf>,
    /// Proposals taken out of the inbox before their command finished, by the command's label:
    /// a reload that read the files before the CLI wrote them must not bring them back.
    pub hidden: Vec<(String, Sel)>,
    /// The "Applies to" file as last read off the UI thread: path, text, whether it exists.
    pub applies_file: Option<(String, Option<String>, bool)>,
    /// Handed to a text field's edit handler when its form closed before the edit ran.
    pub scratch: TextEdit,
    /// The same, for the commit sheet's diff view.
    pub spare_diff: DiffState,
}

impl App {
    /// The app as these arguments open it (`--repo`, `--store=local|global`, `--lens=`,
    /// `--select=`, `--applies=`).
    pub fn with_args(args: &[String]) -> App {
        App::with_args_in(args, ingrain_core::layout::global_dir())
    }

    /// [`App::with_args`] with the global store (and the recents file) at `home`.
    pub fn with_args_in(args: &[String], home: Option<PathBuf>) -> App {
        let dir = repo_arg(args).or_else(|| std::env::current_dir().ok());
        let mut app = App {
            bus: Bus::default(),
            source: None,
            ws: Workspace::default(),
            generation: 0,
            recents: recents::load(home.as_deref()),
            home_dir: home,
            home: true,
            lens: Lens::Decisions,
            sel: None,
            filter: TextEdit::new(""),
            store_filter: None,
            category_filter: None,
            applies: TextEdit::new(""),
            sidebar: PaneState::new(248.0).range(200.0, 360.0),
            history: HashMap::new(),
            history_pending: None,
            inbox_at: 0,
            rejecting: None,
            sheet: None,
            log: Vec::new(),
            running: Vec::new(),
            toasts: Vec::new(),
            next_toast: 0,
            drop_over: false,
            hidden: Vec::new(),
            applies_file: None,
            scratch: TextEdit::new(""),
            spare_diff: DiffState::new(Vec::new()),
        };
        // The first window shows content, so this first load runs here, once.
        let source = Source::open_in(dir.as_deref(), app.home_dir.as_deref());
        let only = match args.iter().find_map(|a| a.strip_prefix("--store=")) {
            Some("local") => Some(Store::Local),
            Some("global") => Some(Store::Global),
            _ => None,
        };
        let personal = only == Some(Store::Global);
        if source.root.is_some() {
            let ws = source.load();
            app.open_source(source, ws);
            if only.is_some() {
                // `ingrain --local viewer` / `--global viewer` in a repository: only that store,
                // which the CLI can still change from there.
                app.store_filter = only;
                app.lens = Lens::Decisions;
                app.ensure_selection();
            }
        } else {
            // Not in a repository: home, with the personal store behind it (read-only: the
            // CLI changes stores from inside a repository).
            app.ws = source.load();
            app.source = Some(source);
            app.home = !personal;
        }
        if let Some(lens) = args.iter().find_map(|a| a.strip_prefix("--lens=")) {
            app.lens = Lens::ALL
                .into_iter()
                .find(|l| l.key() == format!("lens-{lens}"))
                .unwrap_or(app.lens);
            app.home = false;
        }
        if let Some(p) = args.iter().find_map(|a| a.strip_prefix("--applies=")) {
            app.applies = TextEdit::new(p);
            app.read_applies();
        }
        if let Some(id) = args.iter().find_map(|a| a.strip_prefix("--select=")) {
            // Go to the lens that lists it, unless `--lens` chose one.
            if !args.iter().any(|a| a.starts_with("--lens=")) {
                let lens = match (app.ws.decision(id), app.ws.rule(id)) {
                    (Some(d), _) if d.pending => Lens::Inbox,
                    (Some(d), _) if d.is_superseded() => Lens::Archive,
                    (Some(_), _) => Lens::Decisions,
                    (None, Some(r)) if r.pending => Lens::Inbox,
                    (None, Some(_)) => Lens::Rules,
                    (None, None) => app.lens,
                };
                app.set_lens(lens);
            }
            if app.lens == Lens::Inbox {
                if let Some(at) = app
                    .inbox()
                    .iter()
                    .position(|s| matches!(s, Sel::Decision(i) | Sel::Rule(i) if i == id))
                {
                    app.inbox_at = at;
                    app.ensure_selection();
                }
            } else {
                app.select_id(id);
            }
        }
        app
    }

    pub fn root(&self) -> Option<&Path> {
        self.ws.root.as_deref()
    }

    fn open_source(&mut self, source: Source, ws: Workspace) {
        self.ws = ws;
        self.hidden.clear();
        self.store_filter = None;
        self.category_filter = None;
        if let Some(root) = &source.root {
            recents::push(&mut self.recents, root, self.home_dir.as_deref());
        }
        self.generation = self.bus.watch(source.clone());
        self.source = Some(source);
        self.home = false;
        self.sel = None;
        self.inbox_at = 0;
        self.history.clear();
        self.lens = if self.ws.inbox_len() > 0 {
            Lens::Inbox
        } else {
            Lens::Decisions
        };
        self.ensure_selection();
    }

    /// Open the repository at or above `dir`, loading it on a worker.
    pub fn open_dir(&mut self, dir: &Path, _tx: &mut Tx) {
        let dir = dir.to_path_buf();
        let home = self.home_dir.clone();
        self.bus.spawn(move |_| {
            let source = Source::open_in(Some(&dir), home.as_deref());
            if source.root.is_none() {
                return Some(Msg::Done {
                    label: "Not a git repository".into(),
                    ok: false,
                    output: dir.display().to_string(),
                });
            }
            let ws = source.load();
            Some(Msg::Opened { source, ws })
        });
    }

    /// The personal (global) store on its own, read-only.
    pub fn open_personal(&mut self) {
        let home = self.home_dir.clone();
        self.bus.spawn(move |_| {
            let source = Source::open_in(None, home.as_deref());
            let ws = source.load();
            Some(Msg::Opened { source, ws })
        });
    }

    /// Read the "Applies to" file on a worker; the view uses the answer when it matches.
    pub fn read_applies(&mut self) {
        let path = ingrain_core::git::normalize_path(self.applies.text.trim());
        let Some(root) = self.ws.root.clone() else {
            return;
        };
        if path.is_empty() || self.applies_file.as_ref().is_some_and(|(p, ..)| *p == path) {
            return;
        }
        self.bus.spawn(move |_| {
            let file = root.join(&path);
            let exists = file.exists();
            let content = std::fs::read_to_string(&file)
                .ok()
                .filter(|c| c.len() < 2_000_000);
            Some(Msg::AppliesFile {
                path,
                content,
                exists,
            })
        });
    }

    /// The decisions and rules for the "Applies to" path, with the file's text once read.
    pub fn applies(
        &self,
    ) -> (
        Vec<(
            &crate::viewer::model::Decision,
            ingrain_ledger::scope::Match,
        )>,
        Vec<&crate::viewer::model::Rule>,
    ) {
        let path = ingrain_core::git::normalize_path(self.applies.text.trim());
        let content = self
            .applies_file
            .as_ref()
            .filter(|(p, ..)| *p == path)
            .and_then(|(_, c, _)| c.as_deref());
        self.ws.applies_to(&path, content)
    }

    /// Whether the "Applies to" path exists, once known.
    pub fn applies_exists(&self) -> Option<bool> {
        let path = ingrain_core::git::normalize_path(self.applies.text.trim());
        self.applies_file
            .as_ref()
            .filter(|(p, ..)| *p == path)
            .map(|(_, _, e)| *e)
    }

    /// Toasts leave after five seconds (failures after ten).
    pub fn expire_toasts(&mut self, _now: f64, tx: &mut Tx) -> bool {
        let before = self.toasts.len();
        self.toasts
            .retain(|t| t.born.elapsed().as_secs() < if t.ok { 5 } else { 10 });
        if !self.toasts.is_empty() {
            tx.wake_after(std::time::Duration::from_millis(500));
        }
        before != self.toasts.len()
    }

    /// Read everything again, on a worker.
    pub fn reload(&mut self) {
        if let Some(src) = self.source.clone() {
            let generation = self.generation;
            self.bus.spawn(move |_| {
                Some(Msg::Loaded {
                    generation,
                    ws: src.load(),
                })
            });
        }
    }

    pub fn go_home(&mut self) {
        self.home = true;
    }

    pub fn toast(&mut self, ok: bool, title: impl Into<String>, body: impl Into<String>) {
        self.next_toast += 1;
        self.toasts.push(Toast {
            id: self.next_toast,
            title: title.into(),
            body: body.into(),
            ok,
            born: std::time::Instant::now(),
        });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    // ---- selection ----

    pub fn select_id(&mut self, id: &str) {
        if self.ws.decision(id).is_some() {
            self.select(Sel::Decision(id.to_owned()));
        } else if self.ws.rule(id).is_some() {
            self.select(Sel::Rule(id.to_owned()));
        }
    }

    pub fn select(&mut self, sel: Sel) {
        self.sel = Some(sel);
        self.request_history();
    }

    /// Ids of the list the current lens shows, in order (for ↑/↓ and a first selection).
    pub fn visible(&self) -> Vec<Sel> {
        let q = self.filter.text.trim().to_lowercase();
        let pass = |text: &str| q.is_empty() || text.to_lowercase().contains(&q);
        let store_ok = |s: Store| self.store_filter.is_none_or(|f| f == s);
        let decisions = |live: bool| {
            self.ws
                .decisions
                .iter()
                .filter(move |d| if live { d.is_live() } else { d.is_superseded() })
                .filter(|d| store_ok(d.store))
                .filter(|d| self.category_filter.is_none_or(|c| c == d.rec.category))
                .filter(|d| {
                    pass(&format!(
                        "{} {} {} {}",
                        d.rec.id,
                        d.rec.decision,
                        d.rec.reason,
                        d.rec.scope.join(" ")
                    ))
                })
                .map(|d| Sel::Decision(d.rec.id.clone()))
                .collect::<Vec<_>>()
        };
        match self.lens {
            Lens::Decisions => decisions(true),
            Lens::Archive => decisions(false),
            Lens::Timeline => {
                let mut v: Vec<_> = self
                    .ws
                    .decisions
                    .iter()
                    .filter(|d| !d.pending && store_ok(d.store))
                    .filter(|d| pass(&format!("{} {}", d.rec.id, d.rec.decision)))
                    .collect();
                v.sort_by(|a, b| b.rec.date.cmp(&a.rec.date).then(b.rec.id.cmp(&a.rec.id)));
                v.into_iter()
                    .map(|d| Sel::Decision(d.rec.id.clone()))
                    .collect()
            }
            Lens::Rules => self
                .ws
                .accepted_rules()
                .filter(|r| store_ok(r.store))
                .filter(|r| {
                    pass(&format!(
                        "{} {} {}",
                        r.id(),
                        r.file.message,
                        r.file.language
                    ))
                })
                .map(|r| Sel::Rule(r.id().to_owned()))
                .collect(),
            Lens::Inbox => self.inbox(),
            Lens::Applies => {
                let (d, r) = self.applies();
                if self.applies.text.trim().is_empty() {
                    return Vec::new();
                }
                d.iter()
                    .map(|(d, _)| Sel::Decision(d.rec.id.clone()))
                    .chain(r.iter().map(|r| Sel::Rule(r.id().to_owned())))
                    .collect()
            }
        }
    }

    pub fn inbox(&self) -> Vec<Sel> {
        self.ws
            .proposals()
            .map(|d| Sel::Decision(d.rec.id.clone()))
            .chain(
                self.ws
                    .proposed_rules()
                    .map(|r| Sel::Rule(r.id().to_owned())),
            )
            .collect()
    }

    /// Keep a selection that still exists, else pick the first visible item.
    pub fn ensure_selection(&mut self) {
        let items = self.visible();
        let ok = self.sel.as_ref().is_some_and(|s| match s {
            Sel::Decision(id) => self.ws.decision(id).is_some(),
            Sel::Rule(id) => self.ws.rule(id).is_some(),
        });
        if self.lens == Lens::Inbox {
            // The cursor follows the item, not the position: proposals arriving or leaving
            // ahead of it must not swap the card under a reason being typed.
            match self
                .sel
                .as_ref()
                .and_then(|s| items.iter().position(|x| x == s))
            {
                Some(at) => self.inbox_at = at,
                None => {
                    self.inbox_at = self.inbox_at.min(items.len().saturating_sub(1));
                    self.sel = items.get(self.inbox_at).cloned();
                    self.rejecting = None;
                }
            }
        } else if !ok || self.sel.as_ref().is_some_and(|s| !items.contains(s)) {
            self.sel = items.first().cloned();
        }
        self.request_history();
    }

    pub fn set_lens(&mut self, lens: Lens) {
        if lens == Lens::Inbox && self.lens != Lens::Inbox {
            // Entering the inbox starts on its current card, not on whatever the list selected.
            self.sel = self.inbox().get(self.inbox_at).cloned();
        }
        self.lens = lens;
        self.home = false;
        self.rejecting = None;
        self.ensure_selection();
    }

    pub fn step(&mut self, delta: isize, tx: &mut Tx) {
        let items = self.visible();
        if items.is_empty() {
            return;
        }
        if self.lens == Lens::Inbox {
            self.inbox_at =
                (self.inbox_at as isize + delta).clamp(0, items.len() as isize - 1) as usize;
            self.rejecting = None;
            self.sel = items.get(self.inbox_at).cloned();
            return;
        }
        let at = self
            .sel
            .as_ref()
            .and_then(|s| items.iter().position(|x| x == s));
        let next = match at {
            Some(i) => (i as isize + delta).clamp(0, items.len() as isize - 1) as usize,
            None => 0,
        };
        let sel = items[next].clone();
        tx.scroll_into_view(row_key(&sel));
        self.select(sel);
    }

    // ---- history ----

    fn selected_path(&self) -> Option<PathBuf> {
        match self.sel.as_ref()? {
            Sel::Decision(id) => self.ws.decision(id).map(|d| d.path.clone()),
            Sel::Rule(id) => self.ws.rule(id).map(|r| r.file.path.clone()),
        }
    }

    pub fn request_history(&mut self) {
        let (Some(path), Some(root)) = (self.selected_path(), self.ws.root.clone()) else {
            return;
        };
        if self.history.contains_key(&path) || self.history_pending.as_ref() == Some(&path) {
            return;
        }
        self.history_pending = Some(path.clone());
        self.bus.spawn(move |_| {
            let commits = history::log(&root, &path);
            Some(Msg::History { path, commits })
        });
    }

    pub fn open_commit(&mut self, hash: String, subject: String) {
        let (Some(root), Some(path)) = (self.ws.root.clone(), self.selected_path()) else {
            return;
        };
        self.sheet = Some(Sheet::Commit {
            title: format!("{} · {}", &hash[..hash.len().min(8)], subject),
            diff: DiffState::new(Vec::new()),
            lines: 0,
            loaded: false,
        });
        self.bus.spawn(move |_| {
            let patch = history::patch(&root, &hash, Some(&path));
            Some(Msg::Patch { hash, patch })
        });
    }

    // ---- writes, all through the CLI ----

    fn store_flag(store: Store) -> Vec<String> {
        match store {
            Store::Project => vec![],
            Store::Local => vec!["--local".into()],
            Store::Global => vec!["--global".into()],
        }
    }

    fn cli(&mut self, label: impl Into<String>, args: Vec<String>) {
        let label = label.into();
        if self.ws.root.is_none() {
            // The CLI opens a repository for every change, even to personal stores.
            self.toast(false, "Open a repository to make changes", "Personal rules are read-only here; run `ingrain --global viewer` inside any repository.");
            return;
        }
        self.running.push(label.clone());
        actions::run(&self.bus, self.ws.root.as_deref(), label, args, false);
    }

    pub fn accept(&mut self, sel: &Sel) {
        match sel {
            Sel::Decision(id) => self.cli(
                format!("Accepted {id}"),
                vec!["decisions".into(), "accept".into(), id.clone()],
            ),
            Sel::Rule(id) => {
                let store = self
                    .ws
                    .proposed_rule(id)
                    .map_or(Store::Project, |r| r.store);
                let mut args = Self::store_flag(store);
                args.extend(["rules".into(), "accept".into(), id.clone()]);
                self.cli(format!("Accepted rule {id}"), args)
            }
        }
        self.drop_from_inbox(sel);
    }

    pub fn reject(&mut self, sel: &Sel, reason: String) {
        let reason = if reason.trim().is_empty() {
            "rejected in the viewer".to_owned()
        } else {
            reason
        };
        match sel {
            Sel::Decision(id) => self.cli(
                format!("Rejected {id}"),
                vec![
                    "decisions".into(),
                    "reject".into(),
                    id.clone(),
                    "--reason".into(),
                    reason,
                ],
            ),
            Sel::Rule(id) => {
                let store = self
                    .ws
                    .proposed_rule(id)
                    .map_or(Store::Project, |r| r.store);
                let mut args = Self::store_flag(store);
                args.extend([
                    "rules".into(),
                    "reject".into(),
                    id.clone(),
                    "--reason".into(),
                    reason,
                ]);
                self.cli(format!("Rejected rule {id}"), args)
            }
        }
        self.rejecting = None;
        self.drop_from_inbox(sel);
    }

    /// Optimistic: the item leaves the inbox at once; the watcher's reload confirms it, and a
    /// failed command brings it back (the reload after the toast).
    fn drop_from_inbox(&mut self, sel: &Sel) {
        if self.ws.root.is_none() {
            return;
        }
        if let Some(label) = self.running.last() {
            self.hidden.push((label.clone(), sel.clone()));
        }
        Self::hide(&mut self.ws, sel);
        self.sel = None;
        self.ensure_selection();
    }

    fn hide(ws: &mut Workspace, sel: &Sel) {
        match sel {
            Sel::Decision(id) => ws.decisions.retain(|d| !(d.pending && &d.rec.id == id)),
            Sel::Rule(id) => ws.rules.retain(|r| !(r.pending && r.file.id == *id)),
        }
    }

    pub fn set_mode(&mut self, id: &str, mode: Mode) {
        let Some(rule) = self
            .ws
            .rules
            .iter_mut()
            .find(|r| r.file.id == id && !r.pending)
        else {
            return;
        };
        if rule.file.meta.mode == mode {
            return;
        }
        rule.file.meta.mode = mode; // optimistic
        let mut args = Self::store_flag(rule.store);
        args.extend([
            "rules".into(),
            "mode".into(),
            id.to_owned(),
            mode.to_string(),
        ]);
        self.cli(format!("{id} → {mode}"), args);
    }

    pub fn start_supersede(&mut self, id: &str) {
        let Some(d) = self.ws.decision(id) else {
            return;
        };
        let mut decision = TextEdit::new(d.rec.decision.clone());
        decision.focused = true;
        self.sheet = Some(Sheet::Supersede {
            id: id.to_owned(),
            decision,
            reason: TextEdit::new(""),
        });
    }

    /// Whether the supersede form can be sent: the CLI needs both a decision and a reason.
    pub fn supersede_ready(&self) -> bool {
        matches!(&self.sheet, Some(Sheet::Supersede { decision, reason, .. })
            if !decision.text.trim().is_empty() && !reason.text.trim().is_empty())
    }

    pub fn submit_supersede(&mut self) {
        if !self.supersede_ready() {
            // Keep the form and what was typed.
            self.toast(
                false,
                "Say why it changes",
                "A replacement needs the new decision and a reason.",
            );
            return;
        }
        let Some(Sheet::Supersede {
            id,
            decision,
            reason,
        }) = self.sheet.take()
        else {
            return;
        };
        let (text, why) = (
            decision.text.trim().to_owned(),
            reason.text.trim().to_owned(),
        );
        let store = self.ws.decision(&id).map_or(Store::Project, |d| d.store);
        let mut args = Self::store_flag(store);
        args.extend([
            "decisions".into(),
            "supersede".into(),
            id.clone(),
            "--decision".into(),
            text,
        ]);
        args.extend(["--reason".into(), why]);
        self.cli(format!("Proposed a replacement for {id}"), args);
    }

    pub fn extract(&mut self) {
        self.sheet = Some(Sheet::Log {
            title: "Extracting decisions".into(),
        });
        let label = EXTRACT.to_owned();
        if self.running.contains(&label) {
            return; // already running: the sheet shows its output
        }
        self.log.clear();
        self.running.push(label.clone());
        actions::run(
            &self.bus,
            self.ws.root.as_deref(),
            label,
            vec!["decisions".into(), "extract".into()],
            true,
        );
    }

    pub fn mine_rules(&mut self) {
        self.sheet = Some(Sheet::Log {
            title: "Learning rules from corrections".into(),
        });
        let label = LEARN.to_owned();
        if self.running.contains(&label) {
            return;
        }
        self.log.clear();
        self.running.push(label.clone());
        let root = self.ws.root.clone();
        // mine, then propose, in one worker so propose sees what mine captured.
        self.bus.spawn(move |bus| {
            let (ok, out) =
                actions::exec(bus, root.as_deref(), &["rules".into(), "mine".into()], true);
            if !ok {
                return Some(Msg::Done {
                    label,
                    ok,
                    output: out,
                });
            }
            let (ok, output) = actions::exec(
                bus,
                root.as_deref(),
                &["rules".into(), "propose".into()],
                true,
            );
            Some(Msg::Done { label, ok, output })
        });
    }

    // ---- poll ----

    pub fn poll(&mut self, tx: &mut Tx) -> bool {
        let msgs = self.bus.drain();
        if msgs.is_empty() {
            return false;
        }
        for m in msgs {
            match m {
                Msg::Loaded { generation, mut ws } if generation == self.generation => {
                    for (_, sel) in &self.hidden {
                        Self::hide(&mut ws, sel);
                    }
                    if ws.head != self.ws.head {
                        // A commit or checkout: history may have changed.
                        self.history.clear();
                        self.history_pending = None;
                    }
                    self.ws = ws;
                    self.ensure_selection();
                }
                Msg::Opened { source, ws } => {
                    tx.set_title(format!("{} — ingrain", ws.name));
                    let personal = source.root.is_none();
                    self.open_source(source, ws);
                    if personal {
                        self.lens = Lens::Decisions;
                        self.ensure_selection();
                    }
                }
                Msg::AppliesFile {
                    path,
                    content,
                    exists,
                } => {
                    self.applies_file = Some((path, content, exists));
                    self.ensure_selection();
                }
                Msg::Loaded { .. } => {}
                Msg::History { path, commits } => {
                    if self.history_pending.as_ref() == Some(&path) {
                        self.history_pending = None;
                    }
                    self.history.insert(path, commits);
                }
                Msg::Patch { hash, patch } => {
                    // Only the commit still open; a slow answer for an earlier one is dropped.
                    if let Some(Sheet::Commit {
                        diff: d,
                        lines,
                        title,
                        loaded,
                    }) = &mut self.sheet
                        && title.starts_with(&hash[..hash.len().min(8)])
                    {
                        *loaded = true;
                        *lines = patch.lines().count();
                        d.set_files(diff::parse(&patch));
                    }
                }
                Msg::Line(line) => {
                    self.log.push(line);
                    if self.log.len() > 400 {
                        self.log.drain(..100);
                    }
                }
                Msg::Done { label, ok, output } => {
                    if let Some(i) = self.running.iter().position(|l| l == &label) {
                        self.running.remove(i);
                    }
                    // The command is over: the reload below shows what it really did.
                    if let Some(i) = self.hidden.iter().position(|(l, _)| l == &label) {
                        self.hidden.remove(i);
                    }
                    if ok {
                        // A person knows what they just accepted or rejected: only a long job
                        // finishing out of sight (its log closed) is worth saying.
                        let long = label == EXTRACT || label == LEARN;
                        if long && !matches!(self.sheet, Some(Sheet::Log { .. })) {
                            let last = output
                                .lines()
                                .rfind(|l| !l.trim().is_empty())
                                .unwrap_or_default();
                            self.toast(true, label, last.to_owned());
                        }
                    } else {
                        self.toast(
                            false,
                            "Command failed",
                            if output.is_empty() { label } else { output },
                        );
                    }
                    // Reload now rather than at the watcher's next tick (and undo an optimistic
                    // change the CLI refused).
                    if let Some(src) = self.source.clone() {
                        let generation = self.generation;
                        self.bus.spawn(move |_| {
                            Some(Msg::Loaded {
                                generation,
                                ws: src.load(),
                            })
                        });
                    }
                }
            }
        }
        true
    }
}

/// Labels of the long jobs, which say when they finish.
const EXTRACT: &str = "Extraction finished";
const LEARN: &str = "Rule proposals ready";

pub fn row_key(sel: &Sel) -> String {
    match sel {
        Sel::Decision(id) => format!("row-{id}"),
        Sel::Rule(id) => format!("row-rule-{id}"),
    }
}

/// `--repo PATH`, `--repo=PATH` or a bare path among the app's arguments.
fn repo_arg(args: &[String]) -> Option<PathBuf> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--repo" {
            return it.next().map(PathBuf::from);
        }
        if let Some(p) = a.strip_prefix("--repo=") {
            return Some(PathBuf::from(p));
        }
        if !a.starts_with('-') {
            return Some(PathBuf::from(a));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn every_lens_renders_and_a_click_selects() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(root)
                .status()
                .unwrap()
                .success()
        );
        write(
            &root.join("decisions/D-0001.yml"),
            "id: D-0001\ndecision: Use `apiClient`.\nstatus: accepted\nscope: ['src/**']\ndate: 2026-01-02\n",
        );
        write(
            &root.join("decisions/D-0002.yml"),
            "id: D-0002\ndecision: Log with logger.\nstatus: accepted\ndate: 2026-02-02\n",
        );
        write(
            &root.join("decisions/pending/P-1.yml"),
            "id: P-1\ndecision: Prefer tabs.\nstatus: proposed\n",
        );
        let args = vec!["--repo".to_owned(), root.to_string_lossy().into_owned()];
        let app = App::with_args_in(&args, Some(root.join("home")));
        assert_eq!(
            app.lens,
            Lens::Inbox,
            "proposals waiting: open on the inbox"
        );
        let env = Env {
            system_dark: false,
            scale: 1.0,
        };
        let mut rt = Runtime::new(app, crate::viewer::ui::view, (1180.0, 760.0), env);
        rt.frame(0.0);
        assert!(
            rt.find("accept").is_some(),
            "the inbox card has its actions"
        );
        for lens in Lens::ALL {
            rt.update(|s, _| s.set_lens(lens));
            rt.frame(1.0 / 60.0);
            assert!(rt.find(lens.key()).is_some(), "{lens:?} keeps the sidebar");
        }
        rt.update(|s, _| s.set_lens(Lens::Decisions));
        for _ in 0..60 {
            rt.frame(1.0 / 60.0);
        }
        assert_eq!(rt.state.sel, Some(Sel::Decision("D-0001".into())));
        let (x, y) = rt.find("row-D-0002").expect("second row");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.sel, Some(Sel::Decision("D-0002".into())));
        rt.update(|s, _| {
            s.applies = TextEdit::new("src/a.ts");
            s.set_lens(Lens::Applies);
        });
        assert_eq!(
            rt.state.visible(),
            vec![
                Sel::Decision("D-0001".into()),
                Sel::Decision("D-0002".into())
            ]
        );
    }

    #[test]
    fn the_inbox_cursor_follows_its_item_and_supersede_needs_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(root)
                .status()
                .unwrap()
                .success()
        );
        write(
            &root.join("decisions/D-0001.yml"),
            "id: D-0001\ndecision: Use tabs.\nstatus: accepted\n",
        );
        write(
            &root.join("decisions/pending/P-b.yml"),
            "id: P-b\ndecision: B.\nstatus: proposed\n",
        );
        write(
            &root.join("decisions/pending/P-c.yml"),
            "id: P-c\ndecision: C.\nstatus: proposed\n",
        );
        let mut app = App::with_args_in(
            &["--repo".to_owned(), root.to_string_lossy().into_owned()],
            Some(root.join("home")),
        );
        app.inbox_at = 1;
        app.sel = None;
        app.ensure_selection();
        assert_eq!(app.sel, Some(Sel::Decision("P-c".into())));
        app.rejecting = Some(TextEdit::new("half typed"));

        // A proposal lands ahead of it: the card stays on P-c, and so does the reason.
        write(
            &root.join("decisions/pending/P-a.yml"),
            "id: P-a\ndecision: A.\nstatus: proposed\n",
        );
        app.ws = app.source.as_ref().unwrap().load();
        app.ensure_selection();
        assert_eq!(app.sel, Some(Sel::Decision("P-c".into())));
        assert_eq!(app.inbox_at, 2);
        assert!(app.rejecting.is_some());

        // A reload that read the files before a command finished doesn't bring a hidden item back.
        app.hidden
            .push(("Accepted P-a".into(), Sel::Decision("P-a".into())));
        let mut ws = app.source.as_ref().unwrap().load();
        for (_, sel) in &app.hidden {
            App::hide(&mut ws, sel);
        }
        assert!(ws.decision("P-a").is_none());

        // Supersede without a reason keeps the form and its text.
        app.start_supersede("D-0001");
        if let Some(Sheet::Supersede { decision, .. }) = &mut app.sheet {
            *decision = TextEdit::new("Use spaces.");
        }
        assert!(!app.supersede_ready());
        app.submit_supersede();
        assert!(
            matches!(&app.sheet, Some(Sheet::Supersede { decision, .. }) if decision.text == "Use spaces.")
        );
        assert!(app.running.is_empty(), "nothing ran");
    }
}
