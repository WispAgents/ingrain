use super::Outcome;
use crate::hooks::{self, Outcome as Hook};
use crate::repo::Repo;
use std::path::Path;
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// Don't touch .claude/settings.json.
    #[arg(long)]
    no_claude: bool,
    /// Don't install the git pre-commit hook.
    #[arg(long)]
    no_git_hook: bool,
    /// Also add an ingrain section to AGENTS.md, for agents that read it but run no hooks
    /// (Codex, Cursor, Copilot…).
    #[arg(long)]
    agents_md: bool,
}

pub fn run(dir: &Path, args: Args) -> Outcome {
    let repo = Repo::open(dir)?;
    let l = &repo.layout;
    std::fs::create_dir_all(l.rules_dir())?;
    std::fs::create_dir_all(l.pending_dir())?;
    if !l.config_file().exists() {
        repo.config.save(&l.config_file())?;
    }
    let gitignore = l.corrections_dir().join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(
            &gitignore,
            "# Proposals stay local until accepted.\npending/\n",
        )?;
    }
    println!("created {}", l.corrections_dir().display());
    std::fs::create_dir_all(repo.ledger.pending_dir())?;
    let ledger_ignore = repo.ledger.dir.join(".gitignore");
    if !ledger_ignore.exists() {
        std::fs::write(
            &ledger_ignore,
            "# Proposed decisions stay local until a person accepts them.\npending/\n",
        )?;
    }
    println!("created {}", repo.ledger.dir.display());

    if !args.no_claude {
        report(
            "Claude Code hooks (.claude/settings.json)",
            hooks::install_claude(repo.git.root())?,
        );
    }
    if args.agents_md {
        let path = repo.git.root().join("AGENTS.md");
        if crate::agents_md::write(&path)? {
            println!("updated the ingrain section in {}", path.display());
        } else {
            println!("AGENTS.md: ingrain section already up to date");
        }
    }
    if !args.no_git_hook {
        report(
            "git pre-commit hook",
            hooks::install_pre_commit(repo.git.git_dir())?,
        );
    }
    println!(
        "
next: `ingrain decisions add \"...\"` to record a decision, `ingrain decisions extract --doc AGENTS.md`
      to propose some, or `ingrain rules mine` then `ingrain rules propose` to learn rules from your corrections"
    );
    Ok(ExitCode::SUCCESS)
}

fn report(what: &str, outcome: Hook) {
    match outcome {
        Hook::Installed => println!("installed {what}"),
        Hook::AlreadyPresent => println!("{what}: already installed"),
        Hook::Skipped(why) => println!("skipped {what}: {why}"),
    }
}
