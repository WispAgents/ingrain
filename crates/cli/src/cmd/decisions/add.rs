use super::{parse_category, parse_pattern};
use crate::cmd::Outcome;
use crate::repo::Repo;
use anyhow::{Context, Result, bail};
use ingrain_engine::CompiledRule;
use ingrain_ledger::record::{Checks, Record, Status, today};
use std::path::Path;
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// The decision, as one imperative sentence.
    decision: String,
    #[arg(long, default_value = "")]
    reason: String,
    /// Path globs or symbol names it applies to (repeatable). Omit for repo-wide.
    #[arg(long)]
    scope: Vec<String>,
    #[arg(long, default_value = "convention")]
    category: String,
    /// An alternative that was rejected (repeatable).
    #[arg(long)]
    rejected: Vec<String>,
    /// Module that must not be imported in scope (repeatable).
    #[arg(long)]
    forbid_import: Vec<String>,
    /// Package that must not be added as a dependency (repeatable).
    #[arg(long)]
    forbid_dep: Vec<String>,
    /// ast-grep pattern that must not appear in scope, as `Language:pattern` (repeatable).
    #[arg(long)]
    forbid_pattern: Vec<String>,
    /// Link to where it was decided (repeatable).
    #[arg(long)]
    source: Vec<String>,
    /// Write it as a proposal for a human to accept (what agents should use).
    #[arg(long)]
    propose: bool,
}

pub fn run(dir: &Path, args: Args) -> Outcome {
    let repo = Repo::open(dir)?;
    if args.decision.trim().is_empty() {
        bail!("the decision is empty");
    }
    let patterns = args
        .forbid_pattern
        .iter()
        .map(|p| parse_pattern(p))
        .collect::<Result<Vec<_>>>()?;
    for p in &patterns {
        let yaml = format!(
            "id: t\nlanguage: {}\nrule:\n  pattern: {}\nmessage: x\n",
            p.language,
            serde_json::to_string(&p.pattern)?
        );
        CompiledRule::compile(&yaml)
            .with_context(|| format!("pattern `{}` doesn't compile", p.pattern))?;
    }
    let mut record = Record {
        id: String::new(),
        decision: args.decision.trim().to_owned(),
        reason: args.reason.trim().to_owned(),
        rejected: args.rejected,
        scope: args.scope,
        category: parse_category(&args.category)?,
        status: Status::Accepted,
        checks: Checks {
            forbid_imports: args.forbid_import,
            forbid_deps: args.forbid_dep,
            forbid_patterns: patterns,
        },
        sources: args.source,
        author: author(&repo),
        date: today(),
        ..Default::default()
    };
    if args.propose {
        record.status = Status::Proposed;
        record.id = repo.ledger.fresh_pending_id(&record.decision)?;
        let path = repo.ledger.pending_path(&record.id)?;
        record.save(&path)?;
        println!(
            "proposed {} → {} (a human runs `ingrain decisions review` to accept it)",
            record.id,
            path.display()
        );
    } else {
        record.id = repo.ledger.next_id()?;
        let path = repo.ledger.path(&record.id)?;
        record.save(&path)?;
        let staged = if repo.stage(&path)? { " (staged)" } else { "" };
        println!("added {} → {}{staged}", record.id, path.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn author(repo: &Repo) -> String {
    repo.git
        .try_run(&["config", "user.name"])
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}
