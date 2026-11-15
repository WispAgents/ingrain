use super::Outcome;
use crate::check::{self, Finding};
use crate::hitlog::{self, Entry, Event};
use crate::repo::Repo;
use crate::session::Session;
use anyhow::{Context, Result};
use ingrain_core::{DiffBase, Mode};
use serde_json::{Value, json};
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// Files to check (default: every file with added lines).
    paths: Vec<String>,
    /// Check staged changes (pre-commit).
    #[arg(long, conflicts_with_all = ["diff", "hook"])]
    staged: bool,
    /// Check added lines in a revision range, e.g. `origin/main...HEAD` (CI).
    #[arg(long, value_name = "RANGE", conflicts_with = "hook")]
    diff: Option<String>,
    /// Run as a Claude Code hook: reads the hook JSON on stdin and answers in JSON.
    #[arg(long, value_enum)]
    hook: Option<Hook>,
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Hook {
    PostEdit,
    Stop,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Text,
    Json,
    Github,
}

pub fn run(dir: &Path, args: Args) -> Outcome {
    match args.hook {
        // A hook must never break the agent's session because ingrain failed.
        Some(hook) => {
            if let Err(e) = run_hook(dir, hook) {
                eprintln!("ingrain hook error (ignored): {e:#}");
            }
            Ok(ExitCode::SUCCESS)
        }
        None => run_cli(dir, args),
    }
}

fn run_cli(dir: &Path, args: Args) -> Outcome {
    let repo = Repo::open(dir)?;
    let rules = repo.compiled_rules()?;
    let (base, context) = if args.staged {
        (DiffBase::Staged, "staged")
    } else if let Some(range) = &args.diff {
        (DiffBase::Range(range.clone()), "ci")
    } else {
        (DiffBase::Head, "manual")
    };
    let only: Vec<String> = args
        .paths
        .iter()
        .map(|p| repo.git.relative(Path::new(p)))
        .collect();
    let targets = check::targets(&repo, &base, (!only.is_empty()).then_some(only.as_slice()))?;
    let findings = check::find(&rules, &targets);
    log(&repo, &findings, context)?;

    let visible: Vec<&Finding> = findings.iter().filter(|f| f.mode != Mode::Shadow).collect();
    match args.format {
        Format::Json => println!("{}", serde_json::to_string_pretty(&to_json(&findings))?),
        Format::Github => {
            for f in visible.iter().filter(|f| f.is_active()) {
                let level = if f.mode == Mode::Block {
                    "error"
                } else {
                    "warning"
                };
                println!(
                    "::{level} file={},line={},title=ingrain {}::{}",
                    f.path,
                    f.line,
                    f.rule,
                    escape_gh(&f.message)
                );
            }
        }
        Format::Text => {
            for f in &visible {
                let tag = if f.is_active() {
                    f.mode.to_string()
                } else {
                    "overridden".into()
                };
                println!("{}:{}  [{tag}] {}  {}", f.path, f.line, f.rule, f.message);
            }
            let shadow = findings.len() - visible.len();
            if shadow > 0 {
                println!("({shadow} shadow-mode hits logged, not shown)");
            }
        }
    }
    let blocking = visible
        .iter()
        .any(|f| f.is_active() && f.mode == Mode::Block);
    Ok(if blocking {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn run_hook(dir: &Path, hook: Hook) -> Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let payload: Value = if input.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&input)?
    };
    let dir = payload["cwd"].as_str().map(Path::new).unwrap_or(dir);
    let repo = Repo::open(dir)?;
    let rules = repo.compiled_rules()?;
    if rules.is_empty() {
        return Ok(());
    }
    let session_id = payload["session_id"].as_str().unwrap_or("default");
    match hook {
        Hook::PostEdit => {
            let file = payload["tool_input"]["file_path"]
                .as_str()
                .context("hook input has no tool_input.file_path")?;
            let rel = repo.git.relative(Path::new(file));
            let sessions = repo.layout.sessions_dir();
            let mut state = Session::load(&sessions, session_id)?;
            if state.edited.insert(rel.clone()) {
                state.save(&sessions, session_id)?;
            }
            let targets = check::targets(&repo, &DiffBase::Head, Some(&[rel]))?;
            let findings = check::find(&rules, &targets);
            log(&repo, &findings, "hook")?;
            let block: Vec<&Finding> = active(&findings, Mode::Block);
            let warn: Vec<&Finding> = active(&findings, Mode::Warn);
            if !block.is_empty() {
                let mut all = block.clone();
                all.extend(warn);
                print_json(
                    &json!({ "decision": "block", "reason": check::agent_message(&all, true) }),
                );
            } else if !warn.is_empty() {
                print_json(&json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PostToolUse",
                        "additionalContext": check::agent_message(&warn, false),
                    }
                }));
            }
        }
        Hook::Stop => {
            let sessions = repo.layout.sessions_dir();
            let mut state = Session::load(&sessions, session_id)?;
            // Only the files this session's agent edited; the human's uncommitted work is theirs.
            let edited: Vec<String> = state.edited.iter().cloned().collect();
            let targets = if edited.is_empty() {
                Vec::new()
            } else {
                check::targets(&repo, &DiffBase::Head, Some(&edited))?
            };
            let findings = check::find(&rules, &targets);
            let block = active(&findings, Mode::Block);
            if block.is_empty() {
                if state.stop_attempts > 0 {
                    state.stop_attempts = 0;
                    state.save(&sessions, session_id)?;
                }
                return Ok(());
            }
            if state.stop_attempts < repo.config.stop_max_attempts {
                state.stop_attempts += 1;
                state.save(&sessions, session_id)?;
                print_json(
                    &json!({ "decision": "block", "reason": check::agent_message(&block, true) }),
                );
            } else {
                // Let the turn end; record that it ended with violations for the human to see.
                state.stop_attempts = 0;
                state.save(&sessions, session_id)?;
                let unresolved: Vec<Finding> = block.into_iter().cloned().collect();
                log_event(&repo, &unresolved, "stop", Event::Unresolved)?;
                eprintln!(
                    "ingrain: turn ended with {} unresolved rule violations",
                    unresolved.len()
                );
            }
        }
    }
    Ok(())
}

fn active(findings: &[Finding], mode: Mode) -> Vec<&Finding> {
    findings
        .iter()
        .filter(|f| f.mode == mode && f.is_active())
        .collect()
}

fn log(repo: &Repo, findings: &[Finding], context: &str) -> Result<()> {
    let (overridden, hits): (Vec<Finding>, Vec<Finding>) = findings
        .iter()
        .cloned()
        .partition(|f| f.override_reason.is_some());
    log_event(repo, &hits, context, Event::Hit)?;
    log_event(repo, &overridden, context, Event::Override)
}

fn log_event(repo: &Repo, findings: &[Finding], context: &str, event: Event) -> Result<()> {
    let now = hitlog::now();
    let entries: Vec<Entry> = findings
        .iter()
        .map(|f| Entry {
            time: now,
            rule: f.rule.clone(),
            path: f.path.clone(),
            line: f.line,
            mode: f.mode,
            event: event.clone(),
            context: context.to_owned(),
            reason: f.override_reason.clone(),
            matched: f.matched.clone(),
        })
        .collect();
    hitlog::append(&repo.layout.hits_file(), &entries)
}

fn print_json(v: &Value) {
    println!("{v}");
}

fn to_json(findings: &[Finding]) -> Value {
    Value::Array(
        findings
            .iter()
            .map(|f| {
                json!({
                    "rule": f.rule,
                    "mode": f.mode.to_string(),
                    "path": f.path,
                    "line": f.line,
                    "message": f.message,
                    "matched": f.matched,
                    "overridden": f.override_reason,
                })
            })
            .collect(),
    )
}

/// GitHub workflow commands treat `%`, CR and LF specially.
fn escape_gh(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}
