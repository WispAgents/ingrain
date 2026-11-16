//! `ingrain`: keep coding agents in line with what your team decided. Decisions you record and
//! lint rules learned from your corrections are shown and enforced where the agent edits,
//! before commit and in CI.

mod agents_md;
mod bash;
mod check;
mod cmd;
mod hitlog;
mod hooks;
mod overrides;
mod repo;
mod session;

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
    /// Create .corrections/ and decisions/, and install the Claude Code and git hooks.
    Init(cmd::init::Args),
    /// Check added lines against learned rules and decisions (pre-commit, CI).
    Check(cmd::check::Args),
    /// The decision ledger: record, extract, review, supersede and list decisions.
    Decisions {
        #[command(subcommand)]
        command: cmd::decisions::Command,
    },
    /// Learned lint rules: mine corrections, propose, review, promote and evaluate rules.
    Rules {
        #[command(subcommand)]
        command: cmd::rules::Command,
    },
    /// Claude Code hook entry point (reads the hook JSON on stdin).
    Hook {
        event: cmd::hook::Event,
        /// Set by the Claude Code plugin; does nothing when the repo's own settings run ingrain.
        #[arg(long)]
        from_plugin: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let dir = cli.repo.unwrap_or_else(|| std::path::PathBuf::from("."));
    let result = match cli.command {
        Command::Init(args) => cmd::init::run(&dir, args),
        Command::Check(args) => cmd::check::run(&dir, args),
        Command::Decisions { command } => cmd::decisions::run(&dir, command),
        Command::Hook { event, from_plugin } => cmd::hook::run(&dir, event, from_plugin),
        Command::Rules { command } => cmd::rules::run(&dir, command),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ingrain: {e:#}");
            ExitCode::from(2)
        }
    }
}
