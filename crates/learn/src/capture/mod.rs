//! Sources of corrections: git history, PR review suggestions, agent transcripts.

pub mod git;
pub mod github;
pub mod snippet;
pub mod transcript;

use ingrain_core::Correction;
use std::collections::HashSet;

/// Merge corrections from several sources, dropping duplicates: the same agent
/// lines corrected through more than one source (a review suggestion and the commit
/// that applied it, maybe with edits). The earliest one is kept, so a time-split
/// evaluation can't learn from one copy and "prevent" its twin.
pub fn merge(sources: Vec<Vec<Correction>>) -> Vec<Correction> {
    let mut all: Vec<Correction> = sources.into_iter().flatten().collect();
    all.sort_by(|a, b| a.time.cmp(&b.time).then(a.id.cmp(&b.id)));
    let mut seen = HashSet::new();
    all.retain(|c| seen.insert((c.path.clone(), normalize(&c.before.changed_text()))));
    all
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::tests::correction;

    #[test]
    fn merge_keeps_the_earliest_twin() {
        let mut review = correction(
            "r",
            "TypeScript",
            "x = fetch(a);",
            "x = api.get(a);",
            "pr#1",
        );
        review.time = 5;
        let mut commit = correction("g", "TypeScript", "x = fetch(a);", "x = api.get(a) ;", "c1");
        commit.time = 9;
        let merged = merge(vec![vec![commit], vec![review]]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "r");
    }
}
