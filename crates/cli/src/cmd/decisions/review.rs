use crate::cmd::Outcome;
use crate::repo::Repo;
use anyhow::{Context, Result, bail};
use ingrain_ledger::record::{Record, Rejection, Status};
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::ExitCode;

/// Accept a proposal: give it the next `D-` id, make it live, and retire what it supersedes.
/// The new record is written before the old one is retired, so a failure halfway never
/// leaves the ledger without the decision.
pub fn accept(repo: &Repo, pending_id: &str) -> Result<String> {
    let mut record = repo.ledger.get_pending(pending_id)?;
    let pending_path = repo.ledger.pending_path(pending_id)?;
    let old = match record.supersedes.clone() {
        Some(old_id) => {
            let old = repo.ledger.get(&old_id).with_context(|| {
                format!("{pending_id} supersedes {old_id}, which doesn't exist")
            })?;
            if !old.is_live() {
                bail!(
                    "{pending_id} supersedes {old_id}, which is already superseded by {}",
                    old.superseded_by.as_deref().unwrap_or("?")
                );
            }
            Some(old)
        }
        None => None,
    };
    let id = repo.ledger.next_id()?;
    record.id = id.clone();
    record.status = Status::Accepted;
    let path = repo.ledger.path(&id)?;
    record.save(&path)?;
    let mut staged = vec![path.clone()];
    if let Some(mut old) = old {
        let old_path = repo.ledger.path(&old.id)?;
        old.status = Status::Superseded;
        old.superseded_by = Some(id.clone());
        if let Err(e) = old.save(&old_path) {
            let _ = std::fs::remove_file(&path);
            return Err(e.context(format!("couldn't retire {}; {id} was not added", old.id)));
        }
        staged.push(old_path);
    }
    std::fs::remove_file(&pending_path)?;
    for p in staged {
        let rel = repo.git.relative(&p);
        repo.git.run(&["add", "--", &rel])?;
    }
    Ok(id)
}

pub fn reject(repo: &Repo, pending_id: &str, reason: &str) -> Result<()> {
    if reason.trim().is_empty() {
        bail!("a reason is required");
    }
    let record = repo.ledger.get_pending(pending_id)?;
    repo.ledger.add_rejection(Rejection {
        decision: record.decision,
        reason: reason.trim().to_owned(),
    })?;
    std::fs::remove_file(repo.ledger.pending_path(pending_id)?)?;
    Ok(())
}

pub fn accept_cmd(dir: &Path, ids: &[String]) -> Outcome {
    let repo = Repo::open(dir)?;
    for p in ids {
        let id = accept(&repo, p)?;
        println!("accepted {p} as {id} (staged)");
    }
    println!("commit decisions/ to share them with your team and CI");
    Ok(ExitCode::SUCCESS)
}

pub fn reject_cmd(dir: &Path, id: &str, reason: &str) -> Outcome {
    let repo = Repo::open(dir)?;
    reject(&repo, id, reason)?;
    println!("rejected {id}; it won't be proposed again");
    Ok(ExitCode::SUCCESS)
}

pub fn interactive(dir: &Path) -> Outcome {
    let repo = Repo::open(dir)?;
    let pending = repo.ledger.pending()?;
    if pending.is_empty() {
        println!("nothing to review");
        return Ok(ExitCode::SUCCESS);
    }
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    for (i, record) in pending.iter().enumerate() {
        println!("\n[{}/{}] {}", i + 1, pending.len(), card(&repo, record));
        loop {
            print!("[a]ccept  [e]dit  [r]eject  [s]kip  [q]uit > ");
            std::io::stdout().flush()?;
            let Some(answer) = read_line(&mut input)? else {
                return Ok(ExitCode::SUCCESS);
            };
            match answer.as_str() {
                "a" => match accept(&repo, &record.id) {
                    Ok(id) => {
                        println!("accepted as {id}");
                        break;
                    }
                    Err(e) => println!("{e:#}"),
                },
                "e" => match edit(&repo, &record.id) {
                    Ok(()) => println!("saved"),
                    Err(e) => println!("{e:#}"),
                },
                "r" => {
                    print!("reason > ");
                    std::io::stdout().flush()?;
                    let reason = read_line(&mut input)?.unwrap_or_default();
                    match reject(&repo, &record.id, &reason) {
                        Ok(()) => {
                            println!("rejected");
                            break;
                        }
                        Err(e) => println!("{e:#}"),
                    }
                }
                "s" => break,
                "q" => return Ok(ExitCode::SUCCESS),
                _ => {}
            }
        }
    }
    println!("\ncommit decisions/ to share accepted decisions");
    Ok(ExitCode::SUCCESS)
}

/// Text from GitHub or a model, safe to print: no control characters or escape sequences.
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}

/// The review card for one proposal.
pub fn card(repo: &Repo, r: &Record) -> String {
    clean(&card_text(repo, r))
}

fn card_text(repo: &Repo, r: &Record) -> String {
    let mut s = format!("{}  ({:?})\n  {}\n", r.id, r.category, r.decision);
    if !r.reason.is_empty() {
        s.push_str(&format!("  why: {}\n", r.reason));
    }
    if !r.rejected.is_empty() {
        s.push_str(&format!("  rejected: {}\n", r.rejected.join("; ")));
    }
    let scope = if r.scope.is_empty() {
        "repo-wide".to_owned()
    } else {
        r.scope.join(", ")
    };
    s.push_str(&format!("  scope: {scope}\n"));
    if let Some(old) = &r.supersedes {
        let text = repo
            .ledger
            .get(old)
            .map(|o| o.decision)
            .unwrap_or_else(|_| "(missing)".into());
        s.push_str(&format!("  replaces {old}: {text}\n"));
    }
    let c = &r.checks;
    if !c.is_empty() {
        s.push_str(&format!(
            "  checks: forbid imports {:?}, deps {:?}, patterns {:?}\n",
            c.forbid_imports,
            c.forbid_deps,
            c.forbid_patterns
                .iter()
                .map(|p| &p.pattern)
                .collect::<Vec<_>>()
        ));
    }
    for src in &r.sources {
        s.push_str(&format!("  source: {src}\n"));
    }
    if !r.author.is_empty() || !r.date.is_empty() {
        s.push_str(&format!("  by {} on {}\n", r.author, r.date));
    }
    s
}

fn read_line(input: &mut impl BufRead) -> Result<Option<String>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_owned()))
}

/// Edit the proposal in $EDITOR; it must still parse. The id is kept.
fn edit(repo: &Repo, id: &str) -> Result<()> {
    let path = repo.ledger.pending_path(id)?;
    ingrain_core::process::edit(&path)?;
    let mut r = Record::load(&path).context("the edited proposal doesn't parse")?;
    if r.id != id {
        r.id = id.to_owned();
        r.save(&path)?;
    }
    Ok(())
}
