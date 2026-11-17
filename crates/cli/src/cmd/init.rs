use super::Outcome;
use crate::hooks::{self, Outcome as Hook, Settings};
use crate::repo::Repo;
use anyhow::{Context, bail};
use ingrain_core::{Config, Store};
use ingrain_ledger::Ledger;
use std::path::Path;
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// Don't touch the Claude Code settings.
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
    match crate::repo::chosen() {
        Some(store) if store != Store::Project => personal(dir, store, args),
        _ => {
            // Even in a repository set up with `--local`.
            crate::repo::choose_store(Store::Project);
            project(dir, args)
        }
    }
}

/// `init --local` / `init --global`: a personal store, nothing written to the working tree.
fn personal(dir: &Path, store: Store, args: Args) -> Outcome {
    if args.agents_md {
        bail!("AGENTS.md is committed; `--agents-md` only works for the project store");
    }
    let git = ingrain_core::Git::open(dir).ok();
    let base = match (store, &git) {
        (Store::Global, _) => ingrain_core::layout::global_dir()
            .context("no home directory for the global store; set INGRAIN_HOME")?,
        (_, Some(git)) => ingrain_core::layout::local_dir(git),
        (_, None) => bail!("{} is not inside a git repository", dir.display()),
    };
    let corrections = base.join("corrections");
    std::fs::create_dir_all(corrections.join("rules"))?;
    std::fs::create_dir_all(corrections.join("pending"))?;
    let config = corrections.join("config.yml");
    if !config.exists() {
        Config::default().save(&config)?;
    }
    let ledger = Ledger::in_store(&base, store);
    std::fs::create_dir_all(ledger.pending_dir())?;
    println!("created {} ({} store)", base.display(), store.name());

    if !args.no_claude {
        match (store, &git) {
            (Store::Global, _) => match hooks::claude_home() {
                Some(home) => {
                    let path = home.join("settings.json");
                    report(
                        &format!("Claude Code hooks ({})", path.display()),
                        hooks::install_claude_in(&path, Settings::User)?,
                    );
                }
                None => println!("skipped Claude Code hooks: no home directory"),
            },
            (_, Some(git)) => {
                let path = git.root().join(".claude").join("settings.local.json");
                report(
                    "Claude Code hooks (.claude/settings.local.json)",
                    hooks::install_claude_in(&path, Settings::Local)?,
                );
                if hooks::exclude(
                    git,
                    ".claude/settings.local.json",
                    "/.claude/settings.local.json",
                )? {
                    println!("added .claude/settings.local.json to .git/info/exclude");
                }
            }
            (_, None) => {}
        }
    }
    if !args.no_git_hook {
        match &git {
            Some(git) => report(
                "git pre-commit hook (this repository)",
                hooks::install_pre_commit(&hooks::hooks_dir(git)?)?,
            ),
            None => println!("skipped the git pre-commit hook: not in a git repository"),
        }
    }
    let flag = format!("--{}", store.name());
    let reach = match store {
        Store::Global => "every repository on this machine",
        _ => "this repository, for you only",
    };
    println!(
        "
{} store: decisions and rules here apply to {reach} and are never committed.
next: `ingrain {flag} decisions add \"...\"` to record a decision (ids start with {}), and
      `ingrain {flag} rules propose` to keep learned rules here. Hooks and `ingrain check` read the
      project, local and global stores together.",
        store.name(),
        store.id_prefix()
    );
    Ok(ExitCode::SUCCESS)
}

fn project(dir: &Path, args: Args) -> Outcome {
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
            hooks::install_pre_commit(&hooks::hooks_dir(&repo.git)?)?,
        );
    }
    println!(
        "
next: `ingrain decisions extract --doc AGENTS.md` to propose decisions from your rule docs (or
      `ingrain decisions add \"...\"` to record one), then `ingrain decisions review`. Commit
      decisions/, .corrections/ and .claude/settings.json. Later, `ingrain rules mine` then
      `ingrain rules propose` learn rules from your corrections.
agents: see `ingrain --help`. Until the first commit with decisions, run `ingrain decisions accept`
      and the setup's `git add` / `git commit` as their own commands; the hooks let those through."
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
