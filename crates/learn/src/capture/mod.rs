//! Sources of corrections: git history, PR review suggestions, agent transcripts.

pub mod git;
pub mod github;
pub mod snippet;
pub mod transcript;

use ingrain_core::Correction;
use std::collections::HashSet;

/// Merge corrections from several sources, dropping duplicates (the same
/// before/after change seen through more than one source).
pub fn merge(sources: Vec<Vec<Correction>>) -> Vec<Correction> {
    let mut seen = HashSet::new();
    let mut out: Vec<Correction> = sources
        .into_iter()
        .flatten()
        .filter(|c| {
            let key = (
                c.path.clone(),
                normalize(&c.before.changed_text()),
                normalize(&c.after.changed_text()),
            );
            seen.insert(key)
        })
        .collect();
    out.sort_by(|a, b| a.time.cmp(&b.time).then(a.id.cmp(&b.id)));
    out
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
