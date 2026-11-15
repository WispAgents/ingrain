use crate::cmd::Outcome;
use crate::repo::Repo;
use anyhow::bail;
use ingrain_ledger::record::{Record, Status, today};
use std::path::Path;
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// The accepted decision to replace.
    id: String,
    /// The new decision.
    #[arg(long)]
    decision: String,
    #[arg(long)]
    reason: String,
    /// New scope (default: the old decision's scope).
    #[arg(long)]
    scope: Vec<String>,
    /// Keep the old decision's deterministic checks.
    #[arg(long)]
    keep_checks: bool,
}

pub fn run(dir: &Path, args: Args) -> Outcome {
    let repo = Repo::open(dir)?;
    let old = repo.ledger.get(&args.id)?;
    if !old.is_live() {
        bail!(
            "{} is already superseded by {}",
            old.id,
            old.superseded_by.as_deref().unwrap_or("?")
        );
    }
    if args.reason.trim().is_empty() {
        bail!("a reason is required: why should {} change?", old.id);
    }
    let record = Record {
        id: repo
            .ledger
            .fresh_pending_id(&format!("{}\0{}", old.id, args.decision))?,
        decision: args.decision.trim().to_owned(),
        reason: args.reason.trim().to_owned(),
        rejected: vec![old.decision.clone()],
        scope: if args.scope.is_empty() {
            old.scope.clone()
        } else {
            args.scope
        },
        category: old.category,
        status: Status::Proposed,
        supersedes: Some(old.id.clone()),
        checks: if args.keep_checks {
            old.checks.clone()
        } else {
            Default::default()
        },
        date: today(),
        ..Default::default()
    };
    let path = repo.ledger.pending_path(&record.id)?;
    record.save(&path)?;
    println!(
        "proposed {} to replace {}: {}",
        record.id, old.id, record.decision
    );
    println!(
        "{} stays in force until a human accepts it (`ingrain decisions review`)",
        old.id
    );
    Ok(ExitCode::SUCCESS)
}
