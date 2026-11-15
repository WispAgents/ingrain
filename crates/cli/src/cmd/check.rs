//! `ingrain check`: learned rules and decision checks on added lines (pre-commit, CI, by hand).

use super::Outcome;
use crate::check::{self, Finding};
use crate::hitlog::{self, Entry, Event};
use crate::overrides;
use crate::repo::Repo;
use anyhow::{Result, bail};
use ingrain_core::{DiffBase, Mode};
use ingrain_ledger::{Checker, Record, Violation};
use serde_json::{Value, json};
use std::path::Path;
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// Files to check (default: every file with added lines).
    paths: Vec<String>,
    /// Check staged changes (pre-commit).
    #[arg(long, conflicts_with = "diff")]
    staged: bool,
    /// Check added lines in a revision range, e.g. `origin/main...HEAD` (CI).
    #[arg(long, value_name = "RANGE")]
    diff: Option<String>,
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Text,
    Json,
    Github,
}

pub fn run(dir: &Path, args: Args) -> Outcome {
    let repo = Repo::open(dir)?;
    let rules = repo.compiled_rules()?;
    let (base, context) = if args.staged {
        (DiffBase::Staged, "staged")
    } else if let Some(range) = &args.diff {
        if range.starts_with('-') {
            bail!("`--diff` takes a revision range, not an option: {range}");
        }
        (DiffBase::Range(range.clone()), "ci")
    } else {
        (DiffBase::Head, "manual")
    };
    // Pre-commit and CI judge a change by the decisions it started from, so one change can't
    // retire a decision and break it. By hand, the working tree's ledger counts.
    let decisions = match (&base, base.before_rev(&repo.git)) {
        (DiffBase::Head, _) | (_, None) => repo.ledger.live()?,
        (_, Some(rev)) => repo.ledger.live_at(&repo.git, &rev)?,
    };
    let ledger_changes = if base == DiffBase::Head {
        Vec::new()
    } else {
        ledger_changes(&repo, &base)
    };

    let only: Vec<String> = args
        .paths
        .iter()
        .map(|p| repo.git.relative(Path::new(p)))
        .collect();
    let targets = check::targets(&repo, &base, (!only.is_empty()).then_some(only.as_slice()))?;
    let findings = check::find(&rules, &targets);
    log(&repo, &findings, context)?;
    let violations: Vec<Violation> = {
        let checker = Checker::new(&decisions);
        repo.decision_targets(&targets, &base)
            .iter()
            .flat_map(|t| checker.check(t))
            .collect()
    };
    overrides::log(&repo, &violations)?;

    let visible: Vec<&Finding> = findings.iter().filter(|f| f.mode != Mode::Shadow).collect();
    let broken: Vec<&Violation> = violations
        .iter()
        .filter(|v| v.overridden.is_none())
        .collect();
    match args.format {
        Format::Json => {
            let out = json!({
                "rules": findings.iter().map(finding_json).collect::<Vec<_>>(),
                "decisions": violations.iter().map(violation_json).collect::<Vec<_>>(),
                "ledger_changes": ledger_changes,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Format::Github => {
            for f in visible.iter().filter(|f| f.is_active()) {
                let level = if f.mode == Mode::Block {
                    "error"
                } else {
                    "warning"
                };
                println!(
                    "::{level} file={},line={},title={}::{}",
                    escape_property(&f.path),
                    f.line,
                    escape_property(&format!("ingrain rule {}", f.rule)),
                    escape(&f.message)
                );
            }
            for v in &broken {
                println!(
                    "::error file={},line={},title={}::{}",
                    escape_property(&v.path),
                    v.line,
                    escape_property(&format!("decision {}", v.decision)),
                    escape(&format!(
                        "{}. {}",
                        v.what,
                        decision_line(&decisions, &v.decision)
                    ))
                );
            }
            for f in &ledger_changes {
                println!(
                    "::notice file={},title={}::{}",
                    escape_property(f),
                    escape_property("decision ledger changed"),
                    escape(
                        "This change edits the decision ledger; it was checked against the decisions it started from."
                    )
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
            for v in &violations {
                let tag = if v.overridden.is_some() {
                    " [overridden]"
                } else {
                    ""
                };
                println!(
                    "{}:{}  {} {}{tag}: {}",
                    v.path, v.line, v.decision, v.what, v.code
                );
            }
            for f in &ledger_changes {
                println!(
                    "note: this change edits {f}; it was checked against the decisions it started from"
                );
            }
            let shadow = findings.len() - visible.len();
            if shadow > 0 {
                println!("({shadow} shadow-mode rule hits logged, not shown)");
            }
            if visible.is_empty() && violations.is_empty() {
                println!(
                    "nothing broken ({} files, {} rules, {} decisions with checks)",
                    targets.len(),
                    rules.len(),
                    decisions.iter().filter(|r| !r.checks.is_empty()).count()
                );
            }
        }
    }
    let blocking = visible
        .iter()
        .any(|f| f.is_active() && f.mode == Mode::Block)
        || !broken.is_empty();
    Ok(if blocking {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Ledger files a diff changes, other than proposals.
fn ledger_changes(repo: &Repo, base: &DiffBase) -> Vec<String> {
    repo.git
        .diff(base, Some(ingrain_ledger::record::Ledger::DIR))
        .map(|files| {
            files
                .into_iter()
                .filter_map(|f| f.new_path.or(f.old_path))
                .filter(|p| !p.contains("/pending/"))
                .collect()
        })
        .unwrap_or_default()
}

pub fn decision_line(records: &[Record], id: &str) -> String {
    records
        .iter()
        .find(|r| r.id == id)
        .map(Record::line)
        .unwrap_or_else(|| id.to_owned())
}

/// Rule findings in a given mode that aren't overridden.
pub fn active(findings: &[Finding], mode: Mode) -> Vec<&Finding> {
    findings
        .iter()
        .filter(|f| f.mode == mode && f.is_active())
        .collect()
}

/// Log rule hits and overrides to the hit log.
pub fn log(repo: &Repo, findings: &[Finding], context: &str) -> Result<()> {
    let (overridden, hits): (Vec<Finding>, Vec<Finding>) = findings
        .iter()
        .cloned()
        .partition(|f| f.override_reason.is_some());
    log_event(repo, &hits, context, Event::Hit)?;
    log_event(repo, &overridden, context, Event::Override)
}

pub fn log_event(repo: &Repo, findings: &[Finding], context: &str, event: Event) -> Result<()> {
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

fn finding_json(f: &Finding) -> Value {
    json!({
        "rule": f.rule,
        "mode": f.mode.to_string(),
        "path": f.path,
        "line": f.line,
        "message": f.message,
        "matched": f.matched,
        "overridden": f.override_reason,
    })
}

fn violation_json(v: &Violation) -> Value {
    json!({
        "decision": v.decision,
        "path": v.path,
        "line": v.line,
        "what": v.what,
        "code": v.code,
        "overridden": v.overridden,
    })
}

/// GitHub workflow commands treat `%`, CR and LF specially.
fn escape(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Properties (`file=`, `title=`) also need `,` and `:` escaped.
fn escape_property(s: &str) -> String {
    escape(s).replace(',', "%2C").replace(':', "%3A")
}
