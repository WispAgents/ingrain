//! `ingrain`: learn lint rules from the corrections humans make to coding-agent
//! output, and enforce them where the agent edits, before commit and in CI.

mod check;
mod cmd;
mod hitlog;
mod hooks;
mod repo;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "ingrain", version, about, long_about = None)]
struct Cli {
    /// Repository to work on (defaults to the current directory).
    #[arg(long, global = true, value_name = "DIR")]
    repo: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create .corrections/ and install the Claude Code and git hooks.
    Init(cmd::init::Args),
    /// Capture corrections from git history, PR review suggestions and agent transcripts.
    Mine(cmd::mine::Args),
    /// Cluster corrections and write backtested rule proposals to .corrections/pending/.
    Propose(cmd::propose::Args),
    /// Go through pending proposals one by one: accept, edit, reject or skip.
    Review,
    /// Accept pending proposals (moves them to .corrections/rules/ and stages them).
    Accept {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Reject a pending proposal so it is never proposed again.
    Reject {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// List accepted and pending rules with their mode and hit counts.
    Rules,
    /// Set a rule's mode by hand.
    Mode {
        id: String,
        mode: ingrain_core::Mode,
    },
    /// Suggest (or apply) promotions and demotions from the hit log.
    Promote {
        #[arg(long)]
        apply: bool,
    },
    /// Check added lines against accepted rules (hooks, pre-commit, CI).
    Check(cmd::check::Args),
    /// Time-split evaluation: learn from older corrections, score on newer ones.
    Eval(cmd::eval::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let dir = cli.repo.unwrap_or_else(|| std::path::PathBuf::from("."));
    let result = match cli.command {
        Command::Init(args) => cmd::init::run(&dir, args),
        Command::Mine(args) => cmd::mine::run(&dir, args),
        Command::Propose(args) => cmd::propose::run(&dir, args),
        Command::Review => cmd::review::interactive(&dir),
        Command::Accept { ids } => cmd::review::accept(&dir, &ids),
        Command::Reject { id, reason } => cmd::review::reject(&dir, &id, &reason),
        Command::Rules => cmd::rules::list(&dir),
        Command::Mode { id, mode } => cmd::rules::set_mode(&dir, &id, mode),
        Command::Promote { apply } => cmd::rules::promote(&dir, apply),
        Command::Check(args) => cmd::check::run(&dir, args),
        Command::Eval(args) => cmd::eval::run(&dir, args),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ingrain: {e:#}");
            ExitCode::from(2)
        }
    }
}
