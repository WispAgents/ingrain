use crate::cmd::Outcome;
use crate::overrides::{self, Override};
use crate::repo::Repo;
use ingrain_ledger::scope::Scope;
use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

/// Overrides at or above this count flag a decision for a second look.
const OFTEN_OVERRIDDEN: usize = 3;

pub fn run(dir: &Path) -> Outcome {
    let repo = Repo::open(dir)?;
    let records = repo.ledger.records()?;
    let files: Vec<String> = repo
        .git
        .run(&["ls-files"])?
        .lines()
        .map(str::to_owned)
        .collect();
    let overrides: Vec<Override> = overrides::read(&repo);
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    for o in &overrides {
        if seen.insert((o.decision.as_str(), o.path.as_str(), o.code.as_str())) {
            *counts.entry(o.decision.as_str()).or_default() += 1;
        }
    }

    let mut issues = 0;
    for r in records.iter().filter(|r| r.is_live()) {
        let scope = Scope::of(r);
        if !scope.is_repo_wide() {
            // Path match only (fast); symbols fall back to `git grep`.
            let by_path = files.iter().any(|f| scope.matches(f, None).is_some());
            let by_symbol = !by_path
                && scope.symbols().any(|s| {
                    repo.git
                        .try_run(&["grep", "-q", "-w", "-F", "-e", s])
                        .is_some()
                });
            if !by_path && !by_symbol {
                println!(
                    "{}  scope matches no file in the repo ({}); retire or update it",
                    r.id,
                    r.scope.join(", ")
                );
                issues += 1;
            }
        }
        if let Some(n) = counts
            .get(r.id.as_str())
            .filter(|n| **n >= OFTEN_OVERRIDDEN)
        {
            println!("{}  overridden {n} times; maybe it no longer holds", r.id);
            issues += 1;
        }
        if let Some(old) = &r.supersedes
            && records
                .iter()
                .find(|x| &x.id == old)
                .is_none_or(|x| x.superseded_by.as_deref() != Some(r.id.as_str()))
        {
            println!(
                "{}  says it supersedes {old}, but {old} doesn't point back",
                r.id
            );
            issues += 1;
        }
    }
    if issues == 0 {
        println!(
            "no stale decisions ({} live)",
            records.iter().filter(|r| r.is_live()).count()
        );
    }
    Ok(ExitCode::SUCCESS)
}
