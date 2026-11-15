use crate::cmd::Outcome;
use crate::repo::Repo;
use ingrain_ledger::record::Status;
use ingrain_ledger::scope;
use serde_json::json;
use std::path::Path;
use std::process::ExitCode;

pub fn list(dir: &Path, all: bool) -> Outcome {
    let repo = Repo::open(dir)?;
    for r in repo.ledger.records()? {
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
    if all {
        for r in repo.ledger.pending()? {
            println!("{} [proposed]", r.line());
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn show(dir: &Path, id: &str) -> Outcome {
    let repo = Repo::open(dir)?;
    let record = repo
        .ledger
        .get(id)
        .or_else(|_| repo.ledger.get_pending(id))?;
    print!("{}", serde_yaml::to_string(&record)?);
    Ok(ExitCode::SUCCESS)
}

pub fn for_path(dir: &Path, path: &Path, as_json: bool) -> Outcome {
    let repo = Repo::open(dir)?;
    let rel = repo.git.relative(path);
    let records = repo.ledger.live()?;
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
