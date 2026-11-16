use crate::cmd::Outcome;
use crate::hitlog::{self, RuleStats};
use crate::repo::Repo;
use anyhow::Context;
use ingrain_core::{Correction, Mode, store};
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

/// Leaving shadow needs this many hits confirmed by a human correction...
pub const SHADOW_MIN_CONFIRMED: u32 = 3;
/// ...and at least this share of all its hits confirmed.
pub const SHADOW_MIN_CONFIRMED_RATE: f64 = 0.5;

#[derive(Debug, PartialEq)]
pub enum Suggestion {
    Move(Mode, String),
    /// Enough hits to move, but no evidence they are right; the reason why.
    Hold(String),
}

/// What the hit log says about a rule's mode.
///
/// Overrides are the signal in warn and block: the agent sees the message and either
/// fixes the code or overrides with a reason. In shadow the agent sees nothing, so
/// overrides stay at zero and hit volume alone would promote the noisiest rule first.
/// Leaving shadow therefore needs hits that humans confirmed by correcting the flagged
/// code later (`RuleStats::confirmed`, from the mined corrections): at least
/// `SHADOW_MIN_CONFIRMED` of them, and `SHADOW_MIN_CONFIRMED_RATE` of all hits.
pub fn suggest(
    mode: Mode,
    s: &RuleStats,
    min_hits: u32,
    max_override_rate: f64,
) -> Option<Suggestion> {
    let rate = s.override_rate();
    let seen = s.hits + s.overrides;
    if seen >= min_hits && rate > max_override_rate {
        return mode.prev().map(|m| {
            Suggestion::Move(
                m,
                format!("{:.0}% of {seen} hits were overridden", rate * 100.0),
            )
        });
    }
    if s.hits >= min_hits && rate <= max_override_rate {
        let next = mode.next()?;
        if mode == Mode::Shadow {
            let confirmed_rate = f64::from(s.confirmed) / f64::from(s.hits);
            if s.confirmed < SHADOW_MIN_CONFIRMED || confirmed_rate < SHADOW_MIN_CONFIRMED_RATE {
                return Some(Suggestion::Hold(format!(
                    "{} hits but only {} later corrected by a human (need {} and {:.0}%); \
                     shadow hits are invisible to the agent, so volume says nothing about correctness",
                    s.hits,
                    s.confirmed,
                    SHADOW_MIN_CONFIRMED,
                    SHADOW_MIN_CONFIRMED_RATE * 100.0
                )));
            }
            return Some(Suggestion::Move(
                next,
                format!(
                    "{} hits, {} later corrected by a human ({:.0}%)",
                    s.hits,
                    s.confirmed,
                    confirmed_rate * 100.0
                ),
            ));
        }
        return Some(Suggestion::Move(
            next,
            format!("{} hits, {:.0}% overridden", s.hits, rate * 100.0),
        ));
    }
    None
}

pub fn promote(dir: &Path, apply: bool) -> Outcome {
    let repo = Repo::open(dir)?;
    let corrections: Vec<Correction> = store::read(&repo.layout.corrections_file())?;
    let stats = hitlog::stats_confirmed(&repo.layout.hits_file(), &corrections)?;
    let mut changes = Vec::new();
    let mut held = Vec::new();
    for rule in repo.accepted()? {
        let s = stats.get(&rule.id).cloned().unwrap_or_default();
        match suggest(
            rule.meta.mode,
            &s,
            repo.config.promote_min_hits,
            repo.config.promote_max_override_rate,
        ) {
            Some(Suggestion::Move(to, why)) => changes.push(Change {
                id: rule.id.clone(),
                from: rule.meta.mode,
                to,
                why,
            }),
            Some(Suggestion::Hold(why)) => held.push((rule.id.clone(), why)),
            None => {}
        }
    }
    for (id, why) in &held {
        println!("{id:<34} stays in shadow  ({why})");
    }
    if !held.is_empty() {
        println!(
            "run `ingrain rules mine` to pick up recent fixes, or move a rule by hand: `ingrain rules mode <id> warn`"
        );
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
        println!("run `ingrain rules promote --apply` to apply");
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
        let to = |s: Option<Suggestion>| match s {
            Some(Suggestion::Move(m, _)) => Some(m),
            _ => None,
        };
        assert_eq!(to(suggest(Mode::Warn, &clean, 5, 0.1)), Some(Mode::Block));
        assert!(suggest(Mode::Block, &clean, 5, 0.1).is_none());
        let noisy = RuleStats {
            hits: 4,
            overrides: 3,
            ..Default::default()
        };
        assert_eq!(to(suggest(Mode::Block, &noisy, 5, 0.1)), Some(Mode::Warn));
        let few = RuleStats {
            hits: 2,
            ..Default::default()
        };
        assert!(suggest(Mode::Shadow, &few, 5, 0.1).is_none());
    }

    #[test]
    fn leaving_shadow_needs_confirmed_hits() {
        let stats = |hits, confirmed| RuleStats {
            hits,
            confirmed,
            ..Default::default()
        };
        // Many hits, no overrides (the agent never sees shadow hits), nothing confirmed.
        assert!(matches!(
            suggest(Mode::Shadow, &stats(40, 0), 5, 0.1),
            Some(Suggestion::Hold(_))
        ));
        // Confirmed, but a small share of a noisy rule's hits.
        assert!(matches!(
            suggest(Mode::Shadow, &stats(40, 5), 5, 0.1),
            Some(Suggestion::Hold(_))
        ));
        // A high share, but too few to trust.
        assert!(matches!(
            suggest(Mode::Shadow, &stats(5, 2), 5, 0.1),
            Some(Suggestion::Hold(_))
        ));
        assert!(matches!(
            suggest(Mode::Shadow, &stats(6, 3), 5, 0.1),
            Some(Suggestion::Move(Mode::Warn, _))
        ));
    }
}
