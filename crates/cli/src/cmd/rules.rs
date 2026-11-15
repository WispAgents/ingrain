use super::Outcome;
use crate::hitlog::{self, RuleStats};
use crate::repo::Repo;
use anyhow::Context;
use ingrain_core::Mode;
use std::path::Path;
use std::process::ExitCode;

pub fn list(dir: &Path) -> Outcome {
    let repo = Repo::open(dir)?;
    let stats = hitlog::stats(&repo.layout.hits_file())?;
    let accepted = repo.accepted()?;
    println!("accepted ({}):", accepted.len());
    for r in &accepted {
        let s = stats.get(&r.id).cloned().unwrap_or_default();
        println!(
            "  {:<34} {:<6} {:>4} hits  {:>3} overrides  {}",
            r.id, r.meta.mode, s.hits, s.overrides, r.message
        );
    }
    let pending = repo.pending()?;
    println!("pending ({}):", pending.len());
    for r in &pending {
        println!("  {:<34} {}", r.id, r.message);
    }
    Ok(ExitCode::SUCCESS)
}

pub fn set_mode(dir: &Path, id: &str, mode: Mode) -> Outcome {
    let repo = Repo::open(dir)?;
    let mut rule = repo.find_accepted(id)?;
    let path = rule.path.clone();
    rule.set_mode(mode);
    rule.save(&path)?;
    println!("{id} → {mode}");
    Ok(ExitCode::SUCCESS)
}

/// A suggested mode change.
#[derive(Debug, PartialEq)]
pub struct Change {
    pub id: String,
    pub from: Mode,
    pub to: Mode,
    pub why: String,
}

pub fn suggest(
    mode: Mode,
    s: &RuleStats,
    min_hits: u32,
    max_override_rate: f64,
) -> Option<(Mode, String)> {
    let rate = s.override_rate();
    let seen = s.hits + s.overrides;
    if seen >= min_hits && rate > max_override_rate {
        return mode.prev().map(|m| {
            (
                m,
                format!("{:.0}% of {seen} hits were overridden", rate * 100.0),
            )
        });
    }
    if s.hits >= min_hits && rate <= max_override_rate {
        return mode.next().map(|m| {
            (
                m,
                format!("{} hits, {:.0}% overridden", s.hits, rate * 100.0),
            )
        });
    }
    None
}

pub fn promote(dir: &Path, apply: bool) -> Outcome {
    let repo = Repo::open(dir)?;
    let stats = hitlog::stats(&repo.layout.hits_file())?;
    let mut changes = Vec::new();
    for rule in repo.accepted()? {
        let s = stats.get(&rule.id).cloned().unwrap_or_default();
        if let Some((to, why)) = suggest(
            rule.meta.mode,
            &s,
            repo.config.promote_min_hits,
            repo.config.promote_max_override_rate,
        ) {
            changes.push(Change {
                id: rule.id.clone(),
                from: rule.meta.mode,
                to,
                why,
            });
        }
    }
    if changes.is_empty() {
        println!("no mode changes suggested yet");
        return Ok(ExitCode::SUCCESS);
    }
    for c in &changes {
        println!("{:<34} {} → {}  ({})", c.id, c.from, c.to, c.why);
        if apply {
            let mut rule = repo.find_accepted(&c.id)?;
            let path = rule.path.clone();
            rule.set_mode(c.to);
            rule.save(&path).context("saving rule")?;
        }
    }
    if apply {
        println!("applied; commit .corrections/rules/ so CI uses the new modes");
    } else {
        println!("run `ingrain promote --apply` to apply");
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_ladder() {
        let clean = RuleStats {
            hits: 6,
            overrides: 0,
            ..Default::default()
        };
        assert_eq!(suggest(Mode::Shadow, &clean, 5, 0.1).unwrap().0, Mode::Warn);
        assert_eq!(suggest(Mode::Warn, &clean, 5, 0.1).unwrap().0, Mode::Block);
        assert!(suggest(Mode::Block, &clean, 5, 0.1).is_none());
        let noisy = RuleStats {
            hits: 4,
            overrides: 3,
            ..Default::default()
        };
        assert_eq!(suggest(Mode::Block, &noisy, 5, 0.1).unwrap().0, Mode::Warn);
        let few = RuleStats {
            hits: 2,
            ..Default::default()
        };
        assert!(suggest(Mode::Shadow, &few, 5, 0.1).is_none());
    }
}
