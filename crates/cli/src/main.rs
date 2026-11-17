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
Just for the user, nothing committed: `ingrain init --local` (this repo, .git/ingrain/local) or
`ingrain init --global` (every repo, ~/.config/ingrain); then `ingrain --local|--global decisions
add --propose ...`. A person accepts; there is no first-setup exception for personal stores.
Guide: https://github.com/WispAgents/ingrain/blob/main/skills/ingrain-setup/SKILL.md";

#[derive(Parser)]
#[command(name = "ingrain", version, about, long_about = None, after_help = AGENT_HELP)]
struct Cli {
    /// Repository to work on (defaults to the current directory).
    #[arg(long, global = true, value_name = "DIR")]
    repo: Option<std::path::PathBuf>,
    /// Keep decisions and rules for this repository only, inside `.git/`: never committed.
    #[arg(long, global = true, conflicts_with = "global")]
    local: bool,
    /// Keep decisions and rules for every repository on this machine (`~/.config/ingrain`,
    /// or `$INGRAIN_HOME`).
    #[arg(long, global = true)]
    global: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create .corrections/ and decisions/, and install the Claude Code and git hooks
    /// (`--local`: for you only, nothing committed; `--global`: for every repository).
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
    /// Open the desktop viewer: browse, review and trace decisions and rules (`--global`: only
    /// your personal rules).
    View(cmd::view::Args),
    /// Claude Code hook entry point (reads the hook JSON on stdin).
    Hook {
        event: cmd::hook::Event,
        /// Set by the Claude Code plugin; does nothing when the repo's own settings run ingrain.
        #[arg(long)]
        from_plugin: bool,
    },
}

/// Hidden flag that prints the CLI reference as Markdown, for `docs/book/src/reference/cli.md`.
const MARKDOWN_HELP_FLAG: &str = "--markdown-help";

/// The CLI reference as Markdown. The book's sidebar already lists the commands, so the overview
/// is left out, the per-command "Options:" headings become bold text so they stay out of it, the
/// preformatted agent help is fenced so its alignment survives, and empty defaults are dropped.
fn markdown_help() -> String {
    let options = clap_markdown::MarkdownOptions::new()
        .title("CLI reference".to_string())
        .show_footer(false)
        .show_table_of_contents(false);
    clap_markdown::help_markdown_custom::<Cli>(&options)
        .replace("###### **", "**")
        .replace("\n\n  Default value: ``\n", "\n")
        .replace(AGENT_HELP, &format!("```text\n{AGENT_HELP}\n```"))
}

fn main() -> ExitCode {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|a| a == MARKDOWN_HELP_FLAG)
    {
        print!("{}", markdown_help());
        return ExitCode::SUCCESS;
    }
    let cli = Cli::parse();
    let dir = cli.repo.unwrap_or_else(|| std::path::PathBuf::from("."));
    if cli.local {
        repo::choose_store(ingrain_core::Store::Local);
    } else if cli.global {
        repo::choose_store(ingrain_core::Store::Global);
    }
    let result = match cli.command {
        Command::Init(args) => cmd::init::run(&dir, args),
        Command::Check(args) => cmd::check::run(&dir, args),
        Command::Decisions { command } => cmd::decisions::run(&dir, command),
        Command::Hook { event, from_plugin } => cmd::hook::run(&dir, event, from_plugin),
        Command::Rules { command } => cmd::rules::run(&dir, command),
        Command::View(args) => cmd::view::run(&dir, cli.global, args),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ingrain: {e:#}");
            ExitCode::from(2)
        }
    }
}
