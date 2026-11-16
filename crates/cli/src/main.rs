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

/// For a coding agent asked to "use ingrain" or "set up ingrain", which may not have the skills.
const AGENT_HELP: &str = "\
Setting up a repo (for coding agents):
  1. ingrain init --agents-md          hooks, pre-commit, and an AGENTS.md section for hookless agents
  2. ingrain decisions extract --doc AGENTS.md --no-git --no-github --no-sessions
                                       propose decisions from the rule docs (add --doc for each)
  3. Add a `checks:` block (forbid_deps, forbid_imports, forbid_patterns) to each proposal in
     decisions/pending/ that can be checked mechanically; without checks a decision never blocks
  4. Show the user the proposals, then `ingrain decisions accept <P-ids>` for the ones they approve
     and `ingrain decisions reject <P-id> --reason ...` for the rest
  5. On GitHub, add .github/workflows/ingrain.yml using WispAgents/ingrain/action
  6. Commit decisions/, .corrections/, .claude/settings.json and AGENTS.md on a branch
Until that first commit, ingrain's hooks let the agent accept and commit, when each runs as its
own command (no `cd`, variables or other programs in it); after it, agents only propose
(`ingrain decisions add --propose`, `ingrain decisions supersede`) and a person accepts.
Guide: https://github.com/WispAgents/ingrain/blob/main/skills/ingrain-setup/SKILL.md";

#[derive(Parser)]
#[command(name = "ingrain", version, about, long_about = None, after_help = AGENT_HELP)]
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
