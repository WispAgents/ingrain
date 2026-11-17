//! `ingrain hook <event>`: the Claude Code hooks. Every hook fails open: if anything goes wrong,
//! the agent's session continues.
//!
//! - session-start: repo-wide decisions go into the session.
//! - pre-tool-use: the edit is simulated; an added line that breaks a decision's check is denied.
//!   Otherwise the decisions scoped to the file are shown, once per session.
//! - post-tool-use: learned rules run on the edited file; block-mode hits send the agent back.
//! - stop: rules and decisions are re-checked on the files this session edited.

use super::Outcome;
use crate::bash;
use crate::check::{self, Finding};
use crate::hitlog::Event as HitEvent;
use crate::overrides;
use crate::repo::Repo;
use crate::session::{BashSnapshot, Session};
use anyhow::Result;
use ingrain_core::{DiffBase, LineRange, Mode};
use ingrain_ledger::scope::{self, Scope};
use ingrain_ledger::{Checker, Record, Target, Violation, message};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

/// Decisions shown per file and at session start; the rest are listed by count.
const SHOW_MAX: usize = 8;

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Event {
    SessionStart,
    PreToolUse,
    PostToolUse,
    Stop,
}

pub fn run(dir: &Path, event: Event, from_plugin: bool) -> Outcome {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(&input).unwrap_or(json!({}));
    let result = if from_plugin && plugin_steps_aside(dir, &payload) {
        Ok(None)
    } else {
        handle(dir, event, &payload)
    };
    match result {
        Ok(Some(answer)) => println!("{answer}"),
        Ok(None) => {}
        Err(e) => eprintln!("ingrain hook error (ignored): {e:#}"),
    }
    Ok(ExitCode::SUCCESS)
}

/// A plugin hook (`--from-plugin`) answers only in repos set up for ingrain whose own Claude Code
/// settings don't already run it: the plugin is installed for every repo, and must neither act in
/// repos that never chose ingrain nor answer twice where the repo's hooks do.
fn plugin_steps_aside(dir: &Path, payload: &Value) -> bool {
    let dir = payload["cwd"].as_str().map(Path::new).unwrap_or(dir);
    if ingrain_core::Git::open(dir).is_err() {
        return true; // not a git repo
    }
    let repo = match Repo::open_lenient(dir) {
        Ok(repo) => repo,
        Err(e) => {
            eprintln!("ingrain hook error (ignored): {e:#}");
            return true;
        }
    };
    if !repo.is_set_up() {
        return true;
    }
    // Claude Code reads settings from the project dir it was started in, which in a monorepo
    // can be below the git root.
    let mut roots = vec![repo.git.root().to_path_buf()];
    if let Some(project) = std::env::var_os("CLAUDE_PROJECT_DIR") {
        roots.push(project.into());
    }
    roots.iter().any(|root| {
        ["settings.json", "settings.local.json"].iter().any(|f| {
            std::fs::read_to_string(root.join(".claude").join(f))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .is_some_and(|settings| crate::hooks::runs_ingrain(&settings))
        })
    })
}

/// The JSON to print, if any.
pub fn handle(dir: &Path, event: Event, payload: &Value) -> Result<Option<Value>> {
    let dir = payload["cwd"].as_str().map(Path::new).unwrap_or(dir);
    let tool = payload["tool_name"].as_str().unwrap_or("");

    // Accepted decisions, rules and config are reviewed by people; agents propose instead.
    // Checked before anything else is loaded, so a broken config or ledger can't switch it off.
    // Exception: during first setup nothing reviewed is committed yet, so there is nothing to
    // protect. The agent setting ingrain up may accept proposals and commit them; people review
    // that commit like any other (and CI flags ledger changes).
    let mut bash_ask = None;
    let mut first_setup = false;
    if let Event::PreToolUse = event {
        let git = ingrain_core::Git::open(dir)?;
        // Personal stores never reach git, so there is no first setup to let through.
        if let Some(rel) = edited_file(payload).and_then(|f| bash::personal_reviewed_file(&git, f))
        {
            return Ok(Some(deny(reviewed_message(&rel))));
        }
        if let Some(rel) = edited_path(&git, payload)
            && bash::is_reviewed_file(&rel)
            && project_set_up(&git)
            && !is_first_setup(&git)
        {
            return Ok(Some(deny(reviewed_message(&rel))));
        }
        if tool == "Bash" {
            let command = payload["tool_input"]["command"].as_str().unwrap_or("");
            bash_ask = bash::needs_approval(command);
            // Only setup's own commands, which can't reach another repository's ledger.
            if bash_ask.is_some() && is_first_setup(&git) {
                if bash::setup_command(command, git.root()) {
                    first_setup = true;
                    bash_ask = bash::skips_verification(command);
                } else if let Some(why) = &mut bash_ask {
                    // Headless agents see this as a refusal: say how to get through.
                    why.push_str(SETUP_HINT);
                }
            }
        }
    }

    let repo = Repo::open_lenient(dir)?;
    let session_id = payload["session_id"].as_str().unwrap_or("default");
    let sessions = repo.layout.sessions_dir();
    let mut session = Session::load(&sessions, session_id);
    let tool_use_id = payload["tool_use_id"].as_str().map(str::to_owned);

    let decisions = repo.decisions_lenient();
    let answer = match event {
        Event::SessionStart => {
            // Remember a ledger committed outside agent sessions before an agent can hide it.
            ledger_committed(&repo.git);
            session_start(&decisions, &mut session, payload)
        }
        Event::PreToolUse if tool == "Bash" => {
            // Remember what was already changed, to tell what the command changes. Also
            // before an ask: an approved command's other changes are still checked.
            if let Some(id) = &tool_use_id {
                // Only a git command moves through history; reading every ref costs ~0.1s in
                // a repo with thousands. Without them, a move is judged strictly.
                let command = payload["tool_input"]["command"].as_str().unwrap_or("");
                let tips_read = command.contains("git") || command.contains("gh ");
                let (tips, stash) = if tips_read {
                    let (tips, stash) = bash::tips(&repo.git);
                    (Some(tips), stash)
                } else {
                    (None, None)
                };
                let snap = BashSnapshot {
                    time: crate::session::now(),
                    files: bash::snapshot(&repo.git),
                    asked: bash_ask.is_some() || first_setup,
                    head: bash::head(&repo.git),
                    staged: Some(bash::staged_reviewed(&repo.git)),
                    tips,
                    stash,
                    branch: bash::branch(&repo.git),
                    pending: if tips_read {
                        bash::pending_heads(&repo.git)
                    } else {
                        Vec::new()
                    },
                };
                session.bash.insert(id.clone(), snap);
            }
            bash_ask.map(|why| ask(format!("ingrain: {why}")))
        }
        Event::PreToolUse => pre_tool_use(&repo, &decisions, &mut session, payload),
        Event::PostToolUse => post_tool_use(&repo, &decisions, &mut session, payload)?,
        Event::Stop => stop(&repo, &decisions, &mut session)?,
    };
    // Hooks for parallel tool calls can overlap: keep what others recorded meanwhile. The answer
    // is already decided; failing to save state must not drop it.
    let consumed = matches!(event, Event::PostToolUse)
        .then_some(tool_use_id.as_deref())
        .flatten();
    session.merge(Session::load(&sessions, session_id), consumed);
    if let Err(e) = session.save(&sessions, session_id) {
        eprintln!("ingrain: couldn't save hook state: {e:#}");
    }
    Ok(answer)
}

/// The commits a command moved to or brought in that people already reviewed: a reviewed file
/// that now matches one of them came from history (checkout, merge, rebase, pull, stash), not
/// from the agent. Trusted: commits reachable from the refs before the command, and refs this
/// command fetched (a remote-tracking ref updated by fetch or pull, a branch written by fetch),
/// so not a push, the agent's own commit, or a branch a pull merged into. Never trusted: a
/// branch moved back or sideways onto an existing commit (reset, checkout -B), a stash an
/// agent command filled with its own edits, and, for the person's uncommitted edits, anything
/// but leaving them as they were or a new stash that keeps them.
struct History<'a> {
    repo: &'a Repo,
    /// Commits a file may match, whatever its state before.
    commits: Vec<String>,
    /// Where the command started, and `HEAD` after it.
    was: Option<String>,
    now: Option<String>,
    /// A stash this command made.
    new_stash: Option<String>,
    /// Heads of a merge, cherry-pick or rebase in progress.
    pending: Vec<String>,
}

/// Refs before and after a command, and what can be trusted from them.
struct Refs {
    before: BTreeMap<String, String>,
    after: BTreeMap<String, String>,
    /// Commits reachable from these existed, reviewed, before the command.
    roots: Vec<String>,
}

impl Refs {
    fn new(git: &ingrain_core::Git, before: &BashSnapshot) -> Option<Self> {
        let (tips, was) = (before.tips.as_ref()?, before.head.as_ref()?);
        let before_refs = bash::parse_tips(tips);
        let after = bash::parse_tips(&bash::tips(git).0);
        let mut roots: Vec<String> = before_refs.values().cloned().collect();
        roots.push(was.clone());
        for (name, sha) in &after {
            if before_refs.get(name) == Some(sha) {
                continue;
            }
            // Only refs with reflogs, read one by one: tags (thousands, after `--tags`) never
            // count.
            let fetched = if name.starts_with("refs/remotes/") {
                bash::fetched(git, name, true)
            } else if name.starts_with("refs/heads/") {
                bash::fetched(git, name, false)
            } else {
                false
            };
            if fetched {
                roots.push(sha.clone());
            }
        }
        roots.sort();
        roots.dedup();
        Some(Self {
            before: before_refs,
            after,
            roots,
        })
    }

    fn existed(&self, git: &ingrain_core::Git, commit: &str) -> bool {
        let roots: Vec<&str> = self.roots.iter().map(String::as_str).collect();
        bash::new_commits(git, commit, &roots, 1).is_some_and(|n| n.is_empty())
    }

    /// Local branches moved back or sideways onto a commit that existed, and the commits
    /// they dropped that changed reviewed files.
    fn rewound(&self, git: &ingrain_core::Git) -> Vec<(String, bool)> {
        self.before
            .iter()
            .filter(|(name, _)| name.starts_with("refs/heads/"))
            .filter_map(|(name, old)| {
                let new = self.after.get(name)?;
                if new == old || !self.existed(git, new) {
                    return None;
                }
                // Fast-forward: nothing dropped. If git can't tell, count it as rewound.
                let dropped = bash::new_commits(git, old, &[new.as_str()], 1);
                if dropped.as_ref().is_some_and(|d| d.is_empty()) {
                    return None;
                }
                // Dropped commits whose decisions are upstream anyway (a squash- or
                // rebase-merged PR) drop nothing reviewed: compare the trees.
                let mut args = vec!["diff", "--name-only", old.as_str(), new.as_str(), "--"];
                args.extend(REVIEWED_PATHSPECS);
                let touched = git.try_run(&args).is_none_or(|out| !out.trim().is_empty());
                Some((name.trim_start_matches("refs/heads/").to_owned(), touched))
            })
            .collect()
    }
}

impl<'a> History<'a> {
    fn new(
        repo: &'a Repo,
        before: &BashSnapshot,
        refs: Option<&Refs>,
        now_head: Option<&str>,
        moved_head: bool,
        tainted: &BTreeSet<String>,
    ) -> Self {
        let git = &repo.git;
        let mut history = Self {
            repo,
            commits: Vec::new(),
            was: None,
            now: None,
            new_stash: None,
            pending: Vec::new(),
        };
        let (Some(refs), Some(was)) = (refs, &before.head) else {
            return history;
        };
        if refs.rewound(git).iter().any(|(_, touched)| *touched) {
            return history;
        }
        history.was = Some(was.clone());
        history.now = now_head.map(str::to_owned).or(Some(was.clone()));
        let stash_before = refs.before.get("refs/stash");
        if let Some(stash) = stash_before
            && !tainted.contains(stash)
        {
            history.commits.push(stash.clone());
            // `stash -u` keeps untracked files in a third parent.
            history
                .commits
                .extend(bash::head_named(git, &format!("{stash}^3")));
        }
        if let Some(stash) = refs.after.get("refs/stash")
            && Some(stash) != stash_before
            && bash::created_since(git, stash, before.time)
        {
            history.new_stash = Some(stash.clone());
        }
        // A merge, cherry-pick or rebase stopped halfway names what it brings in, before or
        // after the command (`--abort`, `--continue`); ORIG_HEAD is where a rebase started.
        let after_pending = bash::pending_heads(git);
        for entry in before.pending.iter().chain(&after_pending) {
            let Some((name, c)) = entry.split_once(' ') else {
                continue;
            };
            // ORIG_HEAD is where a reset, merge or rebase started: history only for a command
            // that moved HEAD itself (`rebase --continue`), not for a restore from it.
            if name == "ORIG_HEAD" && !moved_head {
                continue;
            }
            if refs.existed(git, c) {
                history.commits.push(c.to_owned());
                if name != "ORIG_HEAD" {
                    history.pending.push(c.to_owned());
                }
            }
        }
        if moved_head && let Some(now) = now_head {
            if refs.existed(git, now) {
                history.commits.push(now.to_owned());
            } else if let Some(new) = bash::new_commits(git, now, &[was.as_str()], 100_000) {
                // Trusted tips it merged or rebased onto.
                let new: std::collections::HashSet<&str> = new.iter().map(String::as_str).collect();
                history.commits.extend(
                    refs.roots
                        .iter()
                        .filter(|t| new.contains(t.as_str()))
                        .take(64)
                        .cloned(),
                );
            }
        }
        history.commits.sort();
        history.commits.dedup();
        history
    }

    /// `path` on disk, in the index and in `HEAD` is as reviewed history has it.
    fn explains(
        &self,
        path: &str,
        before: &BashSnapshot,
        staged_before: Option<&String>,
        staged: Option<&String>,
    ) -> bool {
        let git = &self.repo.git;
        let (Some(was), Some(now)) = (&self.was, &self.now) else {
            return false;
        };
        let disk = bash::blob_on_disk(git, path);
        let was_blob = bash::blob_at(git, was, path);
        let now_blob = bash::blob_at(git, now, path);
        // Content a merge, cherry-pick or rebase in progress put there isn't the person's edit.
        let from_operation = |stamp: &str| {
            self.pending.iter().any(|c| match git.show(c, path) {
                Some(text) => bash::stamp_holds(stamp, text.as_bytes()),
                None => stamp == "deleted",
            })
        };
        if let Some(stamp) = before.files.get(path)
            && !from_operation(stamp)
        {
            // The person's uncommitted edit, back as it was (autostash, a stash around a
            // pull): not a change.
            let same = match std::fs::read(git.root().join(path)) {
                Ok(bytes) => bash::stamp_holds(stamp, &bytes),
                Err(_) => stamp == "deleted",
            };
            if same && now_blob == was_blob && staged == staged_before {
                return true;
            }
            // Gone: only a new stash that keeps it explains that.
            let kept = self.new_stash.as_deref().is_some_and(|s| {
                [s.to_owned(), format!("{s}^3")]
                    .iter()
                    .any(|c| match git.show(c, path) {
                        Some(text) => bash::stamp_holds(stamp, text.as_bytes()),
                        None => stamp == "deleted",
                    })
            });
            if !kept {
                return false;
            }
        }
        let mut blobs: Vec<Option<String>> = self
            .commits
            .iter()
            .map(|c| bash::blob_at(git, c, path))
            .collect();
        // Where it started counts only while `HEAD` still has the same version (not after
        // `git rm --cached` and a commit).
        if now_blob == was_blob {
            blobs.push(was_blob);
        }
        let staged_ok = match staged {
            Some(s) => disk.as_ref() == Some(s) || blobs.contains(&Some(s.clone())),
            None => now_blob.is_none(),
        };
        blobs.contains(&disk) && (blobs.contains(&now_blob) || now_blob == disk) && staged_ok
    }
}

/// A stash an agent command made holds reviewed files other than the person's own edits (the
/// agent's, a formatter's): popping it later is not reviewed history.
fn stash_tainted(repo: &Repo, before: &BashSnapshot, stash: &str) -> bool {
    let git = &repo.git;
    let Some(was) = &before.head else {
        return true;
    };
    let mut paths: Vec<String> = Vec::new();
    let mut args = vec!["diff", "--name-only", "-z", was.as_str(), stash, "--"];
    args.extend(REVIEWED_PATHSPECS);
    paths.extend(
        git.try_run(&args)
            .unwrap_or_default()
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_owned),
    );
    let untracked = format!("{stash}^3");
    let mut args = vec![
        "ls-tree",
        "-r",
        "-z",
        "--name-only",
        untracked.as_str(),
        "--",
    ];
    args.extend(REVIEWED_PATHSPECS);
    if bash::head_named(git, &untracked).is_some() {
        paths.extend(
            git.try_run(&args)
                .unwrap_or_default()
                .split('\0')
                .filter(|p| !p.is_empty())
                .map(str::to_owned),
        );
    }
    paths.iter().any(|p| {
        let stored = git.show(stash, p).or_else(|| git.show(&untracked, p));
        match (before.files.get(p), stored) {
            (Some(stamp), Some(text)) => !bash::stamp_holds(stamp, text.as_bytes()),
            (Some(stamp), None) => stamp != "deleted",
            (None, _) => true,
        }
    })
}

const SETUP_HINT: &str = " ingrain is being set up in this repository (no decision is committed \
     yet), so `ingrain decisions accept|reject` and `git add` / `git commit` of the setup go \
     through without asking when run as their own command: no `cd` elsewhere, `--repo`, \
     variables, or other programs in the same call. Run them that way.";

/// The repository itself chose ingrain: `.corrections/` or a live decision in `decisions/`. A
/// repo that only sees the user's global decisions keeps its own `decisions/` folder (ADRs) free.
fn project_set_up(git: &ingrain_core::Git) -> bool {
    git.root()
        .join(ingrain_core::layout::CORRECTIONS_DIR)
        .is_dir()
        || !ingrain_ledger::Ledger::new(git.root())
            .live_lenient()
            .is_empty()
}

/// No decision, rule or ingrain config was ever committed: ingrain is being set up. The
/// repository's history (every ref, so other branches and worktrees count) is read, not just
/// `HEAD`, so resetting or checking out an older commit doesn't reopen setup; nor does a
/// nested repository (`git init` in a subfolder, a submodule) inside one that has a ledger.
fn is_first_setup(git: &ingrain_core::Git) -> bool {
    // Personal stores are never committed, so no setup commit would review what an agent
    // accepts there: once one is in use (or `init --local` made one), setup is over.
    if ledger_committed(git)
        || crate::repo::personal_in_use(git)
        || ingrain_core::layout::local_dir(git).is_dir()
    {
        return false;
    }
    let mut outer: Vec<std::path::PathBuf> =
        git.root().parent().into_iter().map(Into::into).collect();
    if let Some(project) = std::env::var_os("CLAUDE_PROJECT_DIR") {
        outer.push(project.into());
    }
    while let Some(dir) = outer.pop() {
        let Ok(enclosing) = ingrain_core::Git::open(&dir) else {
            continue;
        };
        if enclosing.root() == git.root() {
            continue;
        }
        if ledger_committed(&enclosing) {
            return false;
        }
        outer.extend(enclosing.root().parent().map(Into::into));
    }
    true
}

/// Pathspecs of the files `bash::is_reviewed_file` protects.
const REVIEWED_PATHSPECS: &[&str] = &[
    ":(glob)decisions/*.yml",
    ":(glob)decisions/*.yaml",
    ":(glob).corrections/rules/*.yml",
    ":(glob).corrections/rules/*.yaml",
    ".corrections/config.yml",
    ".corrections/rejected.yml",
];

/// Some commit on some ref has a reviewed file. Once true it stays true: remembered in the
/// shared git dir (deleting the marker only costs a recount). If git can't tell, protect.
fn ledger_committed(git: &ingrain_core::Git) -> bool {
    // Relative to the root, where git runs; absolute for worktrees.
    let marker = git.try_run(&["rev-parse", "--git-common-dir"]).map(|d| {
        git.root()
            .join(d.trim())
            .join("ingrain")
            .join("ledger-committed")
    });
    if marker.as_ref().is_some_and(|m| m.is_file()) {
        return true;
    }
    // Commits that touched a reviewed file, from every ref, HEAD and the reflogs (a deleted
    // branch or a reset leaves its commits there); an empty repo has none.
    let mut log = vec!["rev-list", "--all", "--reflog", "-1", "--"];
    log.extend(REVIEWED_PATHSPECS);
    let found = git.try_run(&log).is_none_or(|out| !out.trim().is_empty());
    if found && let Some(m) = marker {
        let _ = std::fs::create_dir_all(m.parent().expect("has a parent"));
        let _ = std::fs::write(m, "");
    }
    found
}

fn context(event: &str, text: String) -> Value {
    json!({ "hookSpecificOutput": { "hookEventName": event, "additionalContext": text } })
}

fn reviewed_message(rel: &str) -> String {
    format!(
        "`{rel}` is reviewed by people; agents don't edit it. To change a decision, run \
         `ingrain decisions supersede <id> --decision \"...\" --reason \"...\"`; to record a new one, \
         `ingrain decisions add --propose \"...\"`. A person accepts both."
    )
}

/// Let the person decide in Claude Code's permission prompt.
fn ask(reason: String) -> Value {
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": reason } })
}

fn deny(reason: String) -> Value {
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } })
}

/// The file an edit targets, as the tool gave it.
fn edited_file(payload: &Value) -> Option<&Path> {
    let input = &payload["tool_input"];
    input["file_path"]
        .as_str()
        .or_else(|| input["notebook_path"].as_str())
        .map(Path::new)
}

/// The repo-relative path an edit targets; `None` for files outside the repository.
fn edited_path(git: &ingrain_core::Git, payload: &Value) -> Option<String> {
    git.relative_in_repo(edited_file(payload)?)
}

fn session_start(decisions: &[Record], session: &mut Session, payload: &Value) -> Option<Value> {
    // After /compact or /clear the model no longer has what it was shown.
    if matches!(payload["source"].as_str(), Some("compact" | "clear")) {
        session.shown.clear();
    }
    if decisions.is_empty() {
        return None;
    }
    let wide: Vec<&Record> = decisions
        .iter()
        .filter(|r| Scope::of(r).is_repo_wide())
        .collect();
    let scoped = decisions.len() - wide.len();
    let shown: Vec<&Record> = wide.iter().take(SHOW_MAX).copied().collect();
    session.shown.extend(shown.iter().map(|r| r.id.clone()));
    Some(context(
        "SessionStart",
        message::session(&shown, scoped, wide.len() - shown.len()),
    ))
}

fn pre_tool_use(
    repo: &Repo,
    decisions: &[Record],
    session: &mut Session,
    payload: &Value,
) -> Option<Value> {
    // Not recorded as edited here: the edit may still be denied. PostToolUse records it.
    let rel = edited_path(&repo.git, payload)?;
    let tool = payload["tool_name"].as_str().unwrap_or("");
    let current = repo.read(&rel);
    let after = apply_edit(tool, &payload["tool_input"], current.as_deref());

    // An ignore comment is an exception to a rule or decision: a person grants it. Asked
    // after the decision check, so approving the comment can't let a broken decision through.
    let ask_ignore = adds_ignore_comment(
        tool,
        &payload["tool_input"],
        current.as_deref(),
        after.as_deref(),
    )
    .map(ask);

    // Check what the file would look like after this edit, before it happens.
    if let Some(after) = &after
        && !decisions.is_empty()
    {
        let added = added_lines(current.as_deref().unwrap_or(""), after);
        let checker = Checker::new(decisions);
        if !checker.is_empty() && !added.is_empty() {
            let target = Target {
                path: rel.clone(),
                content: after.clone(),
                added: Some(added),
                before: current.clone(),
            };
            let found = checker.check(&target);
            let broken: Vec<&Violation> = found.iter().filter(|v| v.overridden.is_none()).collect();
            if !broken.is_empty() {
                return Some(deny(message::violations(&broken, decisions, true)));
            }
        }
    }
    if ask_ignore.is_some() {
        return ask_ignore;
    }

    // Otherwise, show the decisions for this file that the agent hasn't seen yet.
    let content = after.as_deref().or(current.as_deref());
    let new: Vec<&Record> = scope::applicable(decisions, &rel, content)
        .into_iter()
        .map(|(r, _)| r)
        .filter(|r| !session.shown.contains(&r.id))
        .collect();
    if new.is_empty() {
        return None;
    }
    let shown: Vec<&Record> = new.iter().take(SHOW_MAX).copied().collect();
    session.shown.extend(shown.iter().map(|r| r.id.clone()));
    Some(context(
        "PreToolUse",
        message::scoped(&rel, &shown, new.len() - shown.len()),
    ))
}

fn post_tool_use(
    repo: &Repo,
    decisions: &[Record],
    session: &mut Session,
    payload: &Value,
) -> Result<Option<Value>> {
    let is_bash = payload["tool_name"].as_str() == Some("Bash");
    let files: Vec<String> = if is_bash {
        // Only files the command changed, not the person's own uncommitted work.
        let Some(before) = payload["tool_use_id"]
            .as_str()
            .and_then(|id| session.bash.remove(id))
        else {
            return Ok(None);
        };
        let mut changed = bash::changed(&before.files, &bash::snapshot(&repo.git));
        let now_head = bash::head(&repo.git);
        let moved_head = before.head.is_some() && before.head != now_head
            || before.head.is_none() && now_head.is_some();
        if moved_head && let Some(to) = &now_head {
            match &before.head {
                Some(from) => changed.extend(bash::moved(&repo.git, from, to)),
                // The first commit: everything in it.
                None => changed.extend(bash::tracked(&repo.git)),
            }
            changed.sort();
            changed.dedup();
        }
        // The pre-check reads shell syntax and can be fooled (variables, globs); this sees
        // what actually changed. Commands the person approved may change reviewed files.
        // A file whose content is what it was before the command was only recorded by it
        // (committed, staged); its content was checked when it changed. Reviewed files are
        // stamped with a content hash, so keeping the size and mtime doesn't hide an edit.
        // A commit must also hold what was on disk, not something staged around it
        // (`git apply --cached`).
        // A snapshot from an older ingrain (upgraded mid-command) has no index and other
        // stamps: nothing can be compared, so let this one command pass.
        let legacy = before.staged.is_none();
        let staged_before = before.staged.clone().unwrap_or_default();
        let recorded = |p: &str| {
            let Some(was) = before.files.get(p) else {
                return false;
            };
            if *was != bash::stamp_of(&repo.git, p) {
                return false;
            }
            // What was committed is what was staged before (a partly staged file), or what
            // was on disk (`commit -a`).
            !moved_head
                || now_head.as_deref().is_some_and(|head| {
                    let committed = repo
                        .git
                        .try_run(&["rev-parse", "--verify", "-q", &format!("{head}:{p}")])
                        .map(|b| b.trim().to_owned());
                    match committed {
                        Some(blob) => {
                            staged_before.get(p) == Some(&blob)
                                || repo
                                    .git
                                    .show(head, p)
                                    .is_some_and(|text| bash::stamp_holds(was, text.as_bytes()))
                        }
                        None => was == "deleted" || !staged_before.contains_key(p),
                    }
                })
        };
        // The index too (`git update-index`, `git apply --cached`, `git rm --cached`): a staged
        // change counts unless it stages what is on disk, as it was (`git add`).
        let staged_now = bash::staged_reviewed(&repo.git);
        let restaged: Vec<String> = staged_before
            .keys()
            .chain(staged_now.keys())
            .filter(|p| staged_before.get(*p) != staged_now.get(*p))
            .cloned()
            .collect();
        // Unstaged back to what is committed (`git reset`, `git restore --staged`).
        let unstaged = |p: &str| {
            let head_blob = now_head.as_deref().and_then(|head| {
                repo.git
                    .try_run(&["rev-parse", "--verify", "-q", &format!("{head}:{p}")])
                    .map(|b| b.trim().to_owned())
            });
            staged_now.get(p) == head_blob.as_ref()
                && before
                    .files
                    .get(p)
                    .is_none_or(|was| *was == bash::stamp_of(&repo.git, p))
        };
        let staged_as_on_disk = |p: &str| {
            before.files.get(p).is_some_and(|was| {
                *was == bash::stamp_of(&repo.git, p)
                    && repo
                        .git
                        .show_index(p)
                        .is_some_and(|text| bash::stamp_holds(was, text.as_bytes()))
            })
        };
        let mut reviewed: Vec<&String> = changed
            .iter()
            .filter(|p| bash::is_reviewed_file(p))
            .filter(|p| !recorded(p))
            .collect();
        for p in &restaged {
            if !moved_head && !staged_as_on_disk(p) && !unstaged(p) && !reviewed.contains(&p) {
                reviewed.push(p);
            }
        }
        // Reading refs (and their reflogs) only when it matters: a reviewed file changed, or a
        // local branch or the stash moved.
        let refs = if (!reviewed.is_empty() && !before.asked)
            || bash::heads_or_stash_moved(&repo.git, &before)
        {
            Refs::new(&repo.git, &before)
        } else {
            None
        };
        if let Some(refs) = &refs
            && let Some(stash) = refs.after.get("refs/stash")
            && refs.before.get("refs/stash") != Some(stash)
            && bash::created_since(&repo.git, stash, before.time)
            && stash_tainted(repo, &before, stash)
        {
            session.tainted_stashes.insert(stash.clone());
        }
        if !reviewed.is_empty() && !before.asked {
            let history = History::new(
                repo,
                &before,
                refs.as_ref(),
                now_head.as_deref(),
                moved_head,
                &session.tainted_stashes,
            );
            reviewed.retain(|p| {
                !history.explains(p, &before, staged_before.get(*p), staged_now.get(*p))
            });
        }
        // A branch moved back past commits that changed reviewed files, even when the files
        // here didn't change (`git branch -f main HEAD~1`).
        let dropped: Vec<String> = refs
            .as_ref()
            .map(|r| r.rewound(&repo.git))
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, touched)| *touched)
            .map(|(name, _)| name)
            .collect();
        if !before.asked && reviewed.is_empty() && !dropped.is_empty() && !legacy {
            let list: Vec<String> = dropped.iter().map(|b| format!("`{b}`")).collect();
            let list = list.join(", ");
            return Ok(Some(json!({
                "decision": "block",
                "reason": format!(
                    "That command moved {list} back past commits that changed decisions or \
                     learned rules, which people review. Stop and tell the user what it did so \
                     they can keep or undo it (`git reflog` has the old commit)."
                ),
                "systemMessage": format!(
                    "ingrain: the agent moved {list} back past reviewed decision changes. Check it."
                ),
            })));
        }
        if legacy {
            reviewed.clear();
        }
        if !before.asked && !reviewed.is_empty() {
            // The person may have edited it meanwhile, so don't tell the agent to restore it.
            let list: Vec<String> = reviewed.iter().map(|p| format!("`{p}`")).collect();
            let list = list.join(", ");
            return Ok(Some(json!({
                "decision": "block",
                "reason": format!(
                    "{list} changed while that command ran; people review these files and agents \
                     don't edit them. If your command changed them, stop and tell the user what it \
                     did so they can keep or undo it. Don't edit or restore them yourself. {}",
                    reviewed_message(reviewed[0]).split_once(". ").map_or("", |(_, rest)| rest)
                ),
                "systemMessage": format!(
                    "ingrain: {list} changed while the agent ran a shell command. Check the change."
                ),
            })));
        }
        changed
            .into_iter()
            .filter(|p| repo.git.root().join(p).is_file())
            .collect()
    } else {
        edited_path(&repo.git, payload).into_iter().collect()
    };
    if files.is_empty() {
        return Ok(None);
    }
    session.edited.extend(files.iter().cloned());
    let rules = repo.compiled_rules()?;
    let findings = if rules.is_empty() {
        Vec::new()
    } else {
        let targets = check::targets(repo, &DiffBase::Head, Some(&files))?;
        check::find(&rules, &targets)
    };
    if let Err(e) = super::check::log(repo, &findings, "hook") {
        eprintln!("ingrain: couldn't log rule hits: {e:#}");
    }
    let block = super::check::active(&findings, Mode::Block);
    let warn = super::check::active(&findings, Mode::Warn);
    // Edits were checked against decisions before they ran; shell commands only can be after.
    let broken: Vec<Violation> = if is_bash && !decisions.is_empty() {
        let checker = Checker::new(decisions);
        repo.decision_targets(&DiffBase::Head, Some(&files))?
            .iter()
            .flat_map(|t| checker.check(t))
            .filter(|v| v.overridden.is_none())
            .collect()
    } else {
        Vec::new()
    };
    Ok(if !block.is_empty() || !broken.is_empty() {
        let mut reason = String::new();
        if !block.is_empty() {
            let mut all = block;
            all.extend(warn);
            reason.push_str(&check::agent_message(&all, true));
        }
        if !broken.is_empty() {
            if !reason.is_empty() {
                reason.push('\n');
            }
            let refs: Vec<&Violation> = broken.iter().collect();
            reason.push_str(&message::violations(&refs, decisions, false));
        }
        Some(json!({ "decision": "block", "reason": reason }))
    } else if !warn.is_empty() {
        Some(context("PostToolUse", check::agent_message(&warn, false)))
    } else {
        None
    })
}

const IGNORE_MARKERS: &[&str] = &["ingrain-ignore", "decisions-ignore"];

/// Why the person should approve an edit that adds an `ingrain-ignore` / `decisions-ignore`
/// comment, if it does.
fn adds_ignore_comment(
    tool: &str,
    input: &Value,
    current: Option<&str>,
    after: Option<&str>,
) -> Option<String> {
    let count =
        |text: &str| -> usize { IGNORE_MARKERS.iter().map(|m| text.matches(m).count()).sum() };
    let added = match after {
        Some(after) => count(after) > count(current.unwrap_or("")),
        // The edit couldn't be simulated: compare what it writes with what it replaces.
        None => {
            let pairs: Vec<&Value> = match tool {
                "MultiEdit" => input["edits"].as_array()?.iter().collect(),
                _ => vec![input],
            };
            pairs.iter().any(|e| {
                let new = e["new_string"]
                    .as_str()
                    .or_else(|| e["content"].as_str())
                    .unwrap_or("");
                count(new) > count(e["old_string"].as_str().unwrap_or(""))
            })
        }
    };
    added.then(|| {
        "ingrain: this edit adds an ignore comment, an exception to a project decision or learned \
         rule. Approve only if the exception is right."
            .to_owned()
    })
}

fn stop(repo: &Repo, decisions: &[Record], session: &mut Session) -> Result<Option<Value>> {
    let edited: Vec<String> = session.edited.iter().cloned().collect();
    if edited.is_empty() {
        return Ok(None);
    }
    let targets = check::targets(repo, &DiffBase::Head, Some(&edited))?;
    let rules = repo.compiled_rules()?;
    let findings = check::find(&rules, &targets);
    let blocked: Vec<&Finding> = super::check::active(&findings, Mode::Block);
    let violations: Vec<Violation> = {
        let checker = Checker::new(decisions);
        repo.decision_targets(&DiffBase::Head, Some(&edited))?
            .iter()
            .flat_map(|t| checker.check(t))
            .collect()
    };
    if let Err(e) = overrides::log(repo, &violations) {
        eprintln!("ingrain: couldn't log decision overrides: {e:#}");
    }
    let broken: Vec<&Violation> = violations
        .iter()
        .filter(|v| v.overridden.is_none())
        .collect();
    if blocked.is_empty() && broken.is_empty() {
        session.stop_attempts = 0;
        return Ok(None);
    }
    if session.stop_attempts >= repo.config.stop_max_attempts {
        // Let the turn end; record it for the person to see.
        session.stop_attempts = 0;
        let unresolved: Vec<Finding> = blocked.into_iter().cloned().collect();
        if let Err(e) = super::check::log_event(repo, &unresolved, "stop", HitEvent::Unresolved) {
            eprintln!("ingrain: couldn't log unresolved hits: {e:#}");
        }
        let note = format!(
            "ingrain: turn ended with {} broken rules and {} broken decisions after {} attempts",
            unresolved.len(),
            broken.len(),
            repo.config.stop_max_attempts
        );
        eprintln!("{note}");
        return Ok(Some(json!({ "systemMessage": note })));
    }
    session.stop_attempts += 1;
    let mut reason = String::new();
    if !blocked.is_empty() {
        reason.push_str(&check::agent_message(&blocked, true));
    }
    if !broken.is_empty() {
        if !reason.is_empty() {
            reason.push('\n');
        }
        reason.push_str(&message::violations(&broken, decisions, false));
    }
    Ok(Some(json!({ "decision": "block", "reason": reason })))
}

/// The file after an Edit, MultiEdit or Write, or `None` if the edit wouldn't apply.
fn apply_edit(tool: &str, input: &Value, current: Option<&str>) -> Option<String> {
    let replace = |text: &str, e: &Value| -> Option<String> {
        let old = e["old_string"].as_str()?;
        let new = e["new_string"].as_str()?;
        if old.is_empty() {
            return Some(format!("{new}{text}"));
        }
        if !text.contains(old) {
            return None;
        }
        Some(if e["replace_all"].as_bool().unwrap_or(false) {
            text.replace(old, new)
        } else {
            text.replacen(old, new, 1)
        })
    };
    match tool {
        "Write" => input["content"].as_str().map(str::to_owned),
        "Edit" => replace(current.unwrap_or(""), input),
        "MultiEdit" => input["edits"]
            .as_array()?
            .iter()
            .try_fold(current.unwrap_or("").to_owned(), |t, e| replace(&t, e)),
        _ => None,
    }
}

/// Lines of `after` that aren't in `before` (as a multiset, trimmed so re-indenting existing
/// code doesn't count as adding it), as ranges.
fn added_lines(before: &str, after: &str) -> Vec<LineRange> {
    let mut have: HashMap<&str, usize> = HashMap::new();
    for l in before.lines() {
        *have.entry(l.trim()).or_default() += 1;
    }
    let mut out: Vec<LineRange> = Vec::new();
    for (i, l) in after.lines().enumerate() {
        let n = i as u32 + 1;
        let left = have.entry(l.trim()).or_default();
        if *left > 0 {
            *left -= 1;
            continue;
        }
        match out.last_mut() {
            Some(r) if r.end + 1 == n => r.end = n,
            _ => out.push(LineRange::new(n, n)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulates_edits() {
        let cur = "a\nb\nc\n";
        assert_eq!(
            apply_edit(
                "Edit",
                &json!({"old_string": "b", "new_string": "B\nB2"}),
                Some(cur)
            )
            .unwrap(),
            "a\nB\nB2\nc\n"
        );
        assert!(
            apply_edit(
                "Edit",
                &json!({"old_string": "zz", "new_string": "y"}),
                Some(cur)
            )
            .is_none()
        );
        let multi = json!({"edits": [{"old_string": "a", "new_string": "A"}, {"old_string": "c", "new_string": "C"}]});
        assert_eq!(
            apply_edit("MultiEdit", &multi, Some(cur)).unwrap(),
            "A\nb\nC\n"
        );
        assert_eq!(
            apply_edit("Write", &json!({"content": "x"}), None).unwrap(),
            "x"
        );
    }

    #[test]
    fn added_line_ranges() {
        assert_eq!(
            added_lines("a\nb\nc", "a\nX\nY\nb\nc\nZ"),
            vec![LineRange::new(2, 3), LineRange::new(6, 6)]
        );
        assert!(added_lines("a\nb", "b\na").is_empty());
        assert!(
            added_lines("import x", "if (y) {\n  import x\n}")
                .iter()
                .all(|r| !(r.start == 2 && r.end == 2))
        );
    }

    #[test]
    fn ignore_comments_need_approval() {
        let cur = "a\n";
        let edit = json!({"old_string": "a", "new_string": "a // ingrain-ignore x: why"});
        let after = apply_edit("Edit", &edit, Some(cur));
        assert!(adds_ignore_comment("Edit", &edit, Some(cur), after.as_deref()).is_some());
        // Moving an existing comment around doesn't add one.
        let cur = "f() // decisions-ignore D-1: y\n";
        let edit = json!({"old_string": "f()", "new_string": "g()"});
        let after = apply_edit("Edit", &edit, Some(cur));
        assert!(adds_ignore_comment("Edit", &edit, Some(cur), after.as_deref()).is_none());
        // Unsimulated edit: compare new_string with old_string.
        let edit = json!({"old_string": "zz", "new_string": "zz # ingrain-ignore r: q"});
        assert!(adds_ignore_comment("Edit", &edit, Some("a"), None).is_some());
    }
}
