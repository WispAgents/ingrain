//! `ingrain hook <event>`: the Claude Code hooks. Every hook fails open: if anything goes wrong,
//! the agent's session continues.
//!
//! - session-start: repo-wide decisions go into the session.
//! - pre-tool-use: the edit is simulated; an added line that breaks a decision's check is denied.
//!   Otherwise the decisions scoped to the file are shown, once per session.
//! - post-tool-use: learned rules run on the edited file; block-mode hits send the agent back.
//! - stop: rules and decisions are re-checked on the files this session edited.

use super::Outcome;
use crate::check::{self, Finding};
use crate::hitlog::Event as HitEvent;
use crate::overrides;
use crate::repo::Repo;
use crate::session::Session;
use anyhow::Result;
use ingrain_core::{DiffBase, LineRange, Mode};
use ingrain_ledger::scope::{self, Scope};
use ingrain_ledger::{Checker, Record, Target, Violation, message};
use serde_json::{Value, json};
use std::collections::HashMap;
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
    let result = if from_plugin && repo_runs_ingrain(dir, &payload) {
        Ok(None) // the repo's own hooks answer; don't run twice
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

/// True when the repository's committed or local Claude Code settings already run `ingrain hook`.
fn repo_runs_ingrain(dir: &Path, payload: &Value) -> bool {
    let dir = payload["cwd"].as_str().map(Path::new).unwrap_or(dir);
    let Ok(git) = ingrain_core::Git::open(dir) else {
        return false;
    };
    ["settings.json", "settings.local.json"].iter().any(|f| {
        std::fs::read_to_string(git.root().join(".claude").join(f))
            .is_ok_and(|t| t.contains("ingrain hook "))
    })
}

/// The JSON to print, if any.
pub fn handle(dir: &Path, event: Event, payload: &Value) -> Result<Option<Value>> {
    let dir = payload["cwd"].as_str().map(Path::new).unwrap_or(dir);
    let repo = Repo::open(dir)?;
    let session_id = payload["session_id"].as_str().unwrap_or("default");
    let sessions = repo.layout.sessions_dir();
    let mut session = Session::load(&sessions, session_id);

    // Accepted decisions and rules are reviewed by people. Agents propose instead.
    if let Event::PreToolUse = event
        && let Some(rel) = edited_path(&repo, payload)
        && is_reviewed_file(&rel)
    {
        return Ok(Some(deny(format!(
            "`{rel}` is reviewed by people; agents don't edit it. To change a decision, run \
             `ingrain decisions supersede <id> --decision \"...\" --reason \"...\"`; to record a new one, \
             `ingrain decisions add --propose \"...\"`. A person accepts both."
        ))));
    }

    let decisions = repo.ledger.live_lenient();
    let answer = match event {
        Event::SessionStart => session_start(&decisions, &mut session, payload),
        Event::PreToolUse => pre_tool_use(&repo, &decisions, &mut session, payload),
        Event::PostToolUse => post_tool_use(&repo, &mut session, payload)?,
        Event::Stop => stop(&repo, &decisions, &mut session)?,
    };
    // Hooks for parallel tool calls can overlap: keep files others recorded meanwhile.
    let on_disk = Session::load(&sessions, session_id);
    session.edited.extend(on_disk.edited);
    session.save(&sessions, session_id)?;
    Ok(answer)
}

fn context(event: &str, text: String) -> Value {
    json!({ "hookSpecificOutput": { "hookEventName": event, "additionalContext": text } })
}

fn deny(reason: String) -> Value {
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } })
}

/// The repo-relative path an edit targets; `None` for files outside the repository.
fn edited_path(repo: &Repo, payload: &Value) -> Option<String> {
    let file = payload["tool_input"]["file_path"].as_str()?;
    repo.git.relative_in_repo(Path::new(file))
}

/// Accepted decision records and learned rules; proposals stay editable.
fn is_reviewed_file(rel: &str) -> bool {
    let yaml =
        |rest: &str| !rest.contains('/') && (rest.ends_with(".yml") || rest.ends_with(".yaml"));
    rel.strip_prefix("decisions/").is_some_and(yaml)
        || rel.strip_prefix(".corrections/rules/").is_some_and(yaml)
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
    let rel = edited_path(repo, payload)?;
    if decisions.is_empty() {
        return None;
    }
    let current = repo.read(&rel);
    let after = apply_edit(
        payload["tool_name"].as_str().unwrap_or(""),
        &payload["tool_input"],
        current.as_deref(),
    );

    // Check what the file would look like after this edit, before it happens.
    if let Some(after) = &after {
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

fn post_tool_use(repo: &Repo, session: &mut Session, payload: &Value) -> Result<Option<Value>> {
    let Some(rel) = edited_path(repo, payload) else {
        return Ok(None);
    };
    session.edited.insert(rel.clone());
    let rules = repo.compiled_rules()?;
    if rules.is_empty() {
        return Ok(None);
    }
    let targets = check::targets(repo, &DiffBase::Head, Some(&[rel]))?;
    let findings = check::find(&rules, &targets);
    super::check::log(repo, &findings, "hook")?;
    let block = super::check::active(&findings, Mode::Block);
    let warn = super::check::active(&findings, Mode::Warn);
    Ok(if !block.is_empty() {
        let mut all = block;
        all.extend(warn);
        Some(json!({ "decision": "block", "reason": check::agent_message(&all, true) }))
    } else if !warn.is_empty() {
        Some(context("PostToolUse", check::agent_message(&warn, false)))
    } else {
        None
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
        super::check::log_event(repo, &unresolved, "stop", HitEvent::Unresolved)?;
        eprintln!(
            "ingrain: turn ended with {} broken rules and {} broken decisions",
            unresolved.len(),
            broken.len()
        );
        return Ok(None);
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
    fn reviewed_files() {
        assert!(is_reviewed_file("decisions/D-0001.yml"));
        assert!(is_reviewed_file(".corrections/rules/avoid-x.yml"));
        assert!(!is_reviewed_file("decisions/pending/P-1.yml"));
        assert!(!is_reviewed_file("src/decisions/x.yml"));
    }
}
