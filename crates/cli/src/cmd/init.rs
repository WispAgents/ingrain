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

    if !args.no_claude {
        report(
            "Claude Code hooks (.claude/settings.json)",
            hooks::install_claude(repo.git.root())?,
        );
    }
    if !args.no_git_hook {
        report(
            "git pre-commit hook",
            hooks::install_pre_commit(&repo.git.git_dir()?)?,
        );
    }
    println!(
        "\nnext: `ingrain mine` to capture corrections, then `ingrain propose` and `ingrain review`"
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
