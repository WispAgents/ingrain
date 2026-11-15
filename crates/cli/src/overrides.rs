//! The decision override log, `.git/ingrain/decision-overrides.jsonl`: every
//! `decisions-ignore D-NNNN: <reason>` seen by a check. `ingrain decisions stale` reads it.

use crate::repo::Repo;
use anyhow::Result;
use ingrain_ledger::Violation;
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Debug, Serialize, Deserialize)]
pub struct Override {
    pub time: i64,
    pub decision: String,
    pub path: String,
    pub code: String,
    pub reason: String,
}

pub fn file(repo: &Repo) -> std::path::PathBuf {
    repo.state_dir().join("decision-overrides.jsonl")
}

pub fn log(repo: &Repo, found: &[Violation]) -> Result<()> {
    let now = ingrain_ledger::record::now();
    let entries: Vec<Override> = found
        .iter()
        .filter_map(|v| {
            v.overridden.as_ref().map(|reason| Override {
                time: now,
                decision: v.decision.clone(),
                path: v.path.clone(),
                code: v.code.clone(),
                reason: reason.clone(),
            })
        })
        .collect();
    if entries.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(repo.state_dir())?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file(repo))?;
    for e in entries {
        writeln!(f, "{}", serde_json::to_string(&e)?)?;
    }
    Ok(())
}

pub fn read(repo: &Repo) -> Vec<Override> {
    std::fs::read_to_string(file(repo))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
