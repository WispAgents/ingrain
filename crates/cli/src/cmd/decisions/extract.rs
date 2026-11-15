use crate::cmd::Outcome;
use crate::repo::Repo;
use anyhow::Result;
use ingrain_learn::synth::llm;
use ingrain_ledger::extract::{self, source};
use ingrain_ledger::record::{now, parse_since};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// Only texts after this date (YYYY-MM-DD). Default: 90 days before the first run.
    #[arg(long)]
    since: Option<String>,
    /// GitHub repo as `owner/name` (default: detected from the remote; skipped if there is none).
    #[arg(long)]
    github: Option<String>,
    #[arg(long)]
    no_github: bool,
    #[arg(long)]
    no_git: bool,
    #[arg(long)]
    no_sessions: bool,
    /// Also read a doc that states project rules, e.g. AGENTS.md (repeatable). Read once.
    #[arg(long)]
    doc: Vec<String>,
    /// Claude Code projects directory (default ~/.claude/projects).
    #[arg(long)]
    sessions_dir: Option<PathBuf>,
    /// Pages of 100 GitHub items per stream.
    #[arg(long, default_value_t = 10)]
    pages: u32,
    /// Read at most this many texts per run (oldest unread first).
    #[arg(long, default_value_t = 400)]
    max_texts: usize,
    /// auto, claude, api or none.
    #[arg(long, default_value = "auto")]
    llm: String,
    #[arg(long)]
    model: Option<String>,
    /// Count the texts and stop before calling the model.
    #[arg(long)]
    dry_run: bool,
}

/// What earlier runs read: `.git/decisions/seen.json`. Texts are tracked by id, so nothing is
/// skipped when a run reads only part of the backlog, and nothing is read twice.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Seen {
    /// Oldest time looked at (fixed on the first run).
    floor: i64,
    done: BTreeSet<String>,
    /// Texts whose batch failed, and how often. Given up on after `MAX_FAILURES`.
    #[serde(default)]
    failures: BTreeMap<String, u32>,
}

const MAX_FAILURES: u32 = 3;

pub fn run(dir: &Path, args: Args) -> Outcome {
    let repo = Repo::open(dir)?;
    let seen_path = repo.state_dir().join("decisions-seen.json");
    let mut seen: Seen = std::fs::read_to_string(&seen_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if seen.floor == 0 {
        seen.floor = now() - 90 * 86_400;
    }
    let floor = match &args.since {
        Some(d) => parse_since(d)?,
        None => seen.floor,
    };

    let mut texts = source::docs(&repo.git, &args.doc)?;
    if !texts.is_empty() {
        println!("docs: {} sections", texts.len());
    }
    if !args.no_git {
        let found = source::commits(&repo.git, floor, 20_000)?;
        println!("commits: {}", found.len());
        texts.extend(found);
    }
    if !args.no_github {
        let name = match &args.github {
            Some(n) => Some(n.clone()),
            None => source::github_name(repo.git.root()).ok(),
        };
        match name {
            Some(name) => {
                let (found, truncated) = source::github(&name, floor, args.pages)?;
                println!(
                    "github {name}: {} PRs, reviews, issues and comments by maintainers",
                    found.len()
                );
                if truncated {
                    println!(
                        "github: stopped at {} pages per stream; older items weren't read (raise --pages)",
                        args.pages
                    );
                }
                texts.extend(found);
            }
            None => println!("github: no GitHub remote found, skipped (pass --github owner/name)"),
        }
    }
    if !args.no_sessions {
        let dir = args.sessions_dir.clone().or_else(default_sessions_dir);
        if let Some(dir) = dir {
            let root = std::fs::canonicalize(repo.git.root())
                .unwrap_or_else(|_| repo.git.root().to_path_buf());
            let found = source::sessions(&dir, &root, floor);
            println!("claude code sessions: {} messages you typed", found.len());
            texts.extend(found);
        }
    }

    texts.retain(|t| source::worth_reading(t) && !seen.done.contains(&t.id));
    // Oldest first: a run that reads only part of the backlog continues where it stopped.
    // Rule docs first: they state the most decisions per text.
    texts.sort_by_key(|t| (t.kind != "doc", t.time));
    let backlog = texts.len();
    texts.truncate(args.max_texts);
    println!(
        "{} new texts worth reading{}",
        texts.len(),
        if backlog > texts.len() {
            format!(" ({backlog} in the backlog; run again for the rest)")
        } else {
            String::new()
        }
    );
    if args.dry_run || texts.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    let Some(model) = llm::backend(&args.llm, args.model.as_deref())? else {
        println!(
            "no model backend (install Claude Code or set ANTHROPIC_API_KEY); nothing extracted"
        );
        return Ok(ExitCode::SUCCESS);
    };
    eprintln!("extracting with {} …", model.describe());
    let mut known = repo.ledger.records()?;
    known.extend(repo.ledger.pending()?);
    let out = extract::extract(
        model.as_ref(),
        &texts,
        &known,
        &repo.ledger.rejections()?,
        8,
        4,
    );
    for p in &out.proposals {
        p.save(&repo.ledger.pending_path(&p.id)?)?;
    }
    seen.done.extend(out.processed.iter().cloned());
    let mut gave_up = 0;
    for id in &out.failed {
        let n = seen.failures.entry(id.clone()).or_default();
        *n += 1;
        if *n >= MAX_FAILURES {
            seen.done.insert(id.clone());
            gave_up += 1;
        }
    }
    save(&seen_path, &seen)?;

    println!(
        "{} proposals from {} texts in {} calls ({} failed, {} duplicates skipped)",
        out.proposals.len(),
        out.texts,
        out.calls,
        out.failed_calls,
        out.duplicates
    );
    if gave_up > 0 {
        println!("gave up on {gave_up} texts after {MAX_FAILURES} failed attempts");
    }
    for p in &out.proposals {
        let sup = p
            .supersedes
            .as_deref()
            .map(|s| format!(" (replaces {s})"))
            .unwrap_or_default();
        println!("  {}  {}{sup}", p.id, p.decision);
    }
    if !out.proposals.is_empty() {
        println!("\nnext: `ingrain decisions review`");
    }
    Ok(ExitCode::SUCCESS)
}

fn save(path: &Path, seen: &Seen) -> Result<()> {
    std::fs::create_dir_all(path.parent().expect("has parent"))?;
    std::fs::write(path, serde_json::to_string(seen)?)?;
    Ok(())
}

fn default_sessions_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let dir = PathBuf::from(home).join(".claude").join("projects");
    dir.is_dir().then_some(dir)
}
