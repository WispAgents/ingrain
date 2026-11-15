//! `ingrain decisions …`: the decision ledger in `decisions/`.

pub mod add;
pub mod extract;
pub mod list;
pub mod review;
pub mod stale;
pub mod supersede;

use crate::cmd::Outcome;
use anyhow::{Context, Result, bail};
use clap::Subcommand;
use ingrain_ledger::record::{Category, PatternCheck};
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum Command {
    /// Record a decision (accepted, or `--propose` for review).
    Add(add::Args),
    /// Propose decisions from rule docs, commits, GitHub PRs/reviews/issues and Claude Code sessions.
    Extract(extract::Args),
    /// Go through proposed decisions one by one: accept, edit, reject or skip.
    Review,
    /// Accept proposals; a superseding proposal retires the decision it replaces.
    Accept {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Reject a proposal so extraction doesn't bring it back.
    Reject {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Propose a replacement for an accepted decision.
    Supersede(supersede::Args),
    /// List decisions (`--all` includes superseded ones and proposals).
    List {
        #[arg(long)]
        all: bool,
    },
    /// Show one decision in full.
    Show { id: String },
    /// Decisions that apply to a file.
    For {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Decisions whose scope no longer exists, that are often overridden, or that are broken.
    Stale,
}

pub fn run(dir: &Path, command: Command) -> Outcome {
    match command {
        Command::Add(args) => add::run(dir, args),
        Command::Extract(args) => extract::run(dir, args),
        Command::Review => review::interactive(dir),
        Command::Accept { ids } => review::accept_cmd(dir, &ids),
        Command::Reject { id, reason } => review::reject_cmd(dir, &id, &reason),
        Command::Supersede(args) => supersede::run(dir, args),
        Command::List { all } => list::list(dir, all),
        Command::Show { id } => list::show(dir, &id),
        Command::For { path, json } => list::for_path(dir, &path, json),
        Command::Stale => stale::run(dir),
    }
}

/// `--category` values.
pub fn parse_category(s: &str) -> Result<Category> {
    Ok(match s {
        "convention" => Category::Convention,
        "constraint" => Category::Constraint,
        "api_change" | "api-change" => Category::ApiChange,
        "architecture" => Category::Architecture,
        "scope" => Category::Scope,
        other => bail!(
            "unknown category `{other}` (convention, constraint, api_change, architecture, scope)"
        ),
    })
}

/// `--forbid-pattern Language:pattern`, e.g. `TypeScript:fetch($$$A)`.
pub fn parse_pattern(s: &str) -> Result<PatternCheck> {
    let (language, pattern) = s
        .split_once(':')
        .context("--forbid-pattern takes `Language:pattern`, e.g. `TypeScript:fetch($$$A)`")?;
    if ingrain_engine::lang::parse(language).is_none() {
        bail!("unknown language `{language}`");
    }
    Ok(PatternCheck {
        language: language.to_owned(),
        pattern: pattern.to_owned(),
    })
}
