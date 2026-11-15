use crate::cmd::Outcome;
use crate::repo::Repo;
use anyhow::{Context, Result};
use ingrain_core::{Correction, store};
use ingrain_learn::agents::Classifier;
use ingrain_learn::capture::{self, git::GitMineOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[derive(clap::Args)]
pub struct Args {
    /// Also read ```suggestion review comments from GitHub (`owner/name`, or
    /// detected from the origin remote when given without a value).
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    github: Option<String>,
    /// Pages of 100 review comments to read, newest first.
    #[arg(long, default_value_t = 20)]
    review_pages: u32,
    /// Fetch `refs/pull/*/head` first, so fix-ups inside PRs survive squash merges.
    #[arg(long)]
    pr_refs: bool,
    /// Read Claude Code transcripts from this projects directory (default ~/.claude/projects).
    #[arg(long)]
    transcripts: Option<PathBuf>,
    /// Skip transcripts.
    #[arg(long)]
    no_transcripts: bool,
    /// Only commits after this date (`git log --since`).
    #[arg(long)]
    since: Option<String>,
    /// Only the newest N commits.
    #[arg(long)]
    max_commits: Option<usize>,
}

pub fn run(dir: &Path, args: Args) -> Outcome {
    let repo = Repo::open(dir)?;
    let mut revs = vec!["HEAD".to_owned()];
    if args.pr_refs {
        eprintln!("fetching pull request refs…");
        repo.git
            .run(&[
                "fetch",
                "--quiet",
                "origin",
                "+refs/pull/*/head:refs/ingrain/pull/*",
            ])
            .context("fetching refs/pull/*/head from origin")?;
        revs.push("--glob=refs/ingrain/pull/*".into());
    }
    let opts = GitMineOptions {
        window_days: repo.config.window_days,
        revs,
        since: args.since.clone(),
        max_commits: args.max_commits,
        ignore: repo.config.ignore.clone(),
        extra_markers: repo.config.agent_markers.clone(),
        ..Default::default()
    };
    eprintln!("mining git history…");
    let mined = capture::git::mine(&repo.git, &opts)?;
    let s = &mined.stats;
    println!(
        "git: {} commits ({} agent, {} human, {} bot), {} human commits skipped (refactors, formatter runs), {} hunks blamed",
        s.commits,
        s.agent_commits,
        s.human_commits,
        s.bot_commits,
        s.human_commits_skipped,
        s.hunks_blamed
    );
    println!(
        "git: {} corrections, {} agent hunks",
        s.corrections, s.agent_hunks
    );

    let mut sources: Vec<Vec<Correction>> = vec![mined.corrections];
    if let Some(name) = &args.github {
        let name = if name.is_empty() {
            detect_github(repo.git.root())?
        } else {
            name.clone()
        };
        eprintln!("reading review comments from {name}…");
        let comments = capture::github::fetch(&name, args.review_pages)?;
        let found = capture::github::corrections(
            &repo.git,
            &comments,
            &Classifier::new(&repo.config.agent_markers),
        );
        println!(
            "github: {} review comments, {} suggestions on agent code",
            comments.len(),
            found.len()
        );
        sources.push(found);
    }
    if !args.no_transcripts {
        let dir = args.transcripts.clone().or_else(default_projects_dir);
        if let Some(dir) = dir {
            let found = transcripts(&dir, repo.git.root())?;
            println!("transcripts: {} corrections", found.len());
            sources.push(found);
        }
    }

    let corrections = capture::merge(sources);
    store::write(&repo.layout.corrections_file(), &corrections)?;
    store::write(&repo.layout.agent_hunks_file(), &mined.agent_hunks)?;
    println!(
        "saved {} corrections to {}",
        corrections.len(),
        repo.layout.corrections_file().display()
    );
    Ok(ExitCode::SUCCESS)
}

fn transcripts(projects: &Path, root: &Path) -> Result<Vec<Correction>> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut out = Vec::new();
    for file in capture::transcript::session_files(projects, &root) {
        match capture::transcript::mine_file(&file, &root) {
            Ok(found) => out.extend(found),
            Err(e) => eprintln!("ingrain: skipping {}: {e:#}", file.display()),
        }
    }
    Ok(out)
}

fn default_projects_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let dir = PathBuf::from(home).join(".claude").join("projects");
    dir.is_dir().then_some(dir)
}

fn detect_github(root: &Path) -> Result<String> {
    let out = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ])
        .current_dir(root)
        .output()
        .context("failed to run `gh`")?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    anyhow::ensure!(
        out.status.success() && !name.is_empty(),
        "couldn't detect the GitHub repo; pass --github owner/name"
    );
    Ok(name)
}
