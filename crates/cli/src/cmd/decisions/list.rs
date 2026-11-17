use crate::cmd::Outcome;
use crate::repo::Repo;
use ingrain_ledger::record::Status;
use ingrain_ledger::scope;
use serde_json::json;
use std::path::Path;
use std::process::ExitCode;

pub fn list(dir: &Path, all: bool) -> Outcome {
    let repo = Repo::open(dir)?;
    let records = repo.all_records()?;
    let mut pending = Vec::new();
    if all {
        for ledger in repo.ledgers() {
            pending.extend(ledger.pending()?);
        }
    }
    for r in records {
        match r.status {
            Status::Accepted => println!("{}", r.line()),
            Status::Superseded if all => println!(
                "{} [superseded by {}]",
                r.line(),
                r.superseded_by.as_deref().unwrap_or("?")
            ),
            _ => {}
        }
    }
    for r in pending {
        println!("{} [proposed]", r.line());
    }
    Ok(ExitCode::SUCCESS)
}

pub fn show(dir: &Path, id: &str) -> Outcome {
    let repo = Repo::open(dir)?;
    let ledger = repo.ledger_of(id);
    let record = ledger.get(id).or_else(|_| ledger.get_pending(id))?;
    print!("{}", serde_yaml::to_string(&record)?);
    Ok(ExitCode::SUCCESS)
}

pub fn for_path(dir: &Path, path: &Path, as_json: bool) -> Outcome {
    let repo = Repo::open(dir)?;
    let rel = repo.git.relative(path);
    let records = repo.decisions()?;
    let content = repo.read(&rel);
    let found = scope::applicable(&records, &rel, content.as_deref());
    if as_json {
        let items: Vec<_> = found.iter().map(|(r, m)| json!({ "id": r.id, "decision": r.decision, "reason": r.reason, "match": format!("{m:?}") })).collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
    } else if found.is_empty() {
        println!("no decisions apply to {rel}");
    } else {
        for (r, _) in found {
            println!("{}", r.line());
        }
    }
    Ok(ExitCode::SUCCESS)
}
