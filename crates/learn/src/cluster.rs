//! Group corrections by what they removed.
//!
//! Each correction is reduced to its *anchors*: salient tokens that appear more often
//! in the agent's lines than in the human's replacement (`fetch` in
//! `fetch(url)` → `apiClient.get(url)`). Corrections sharing an anchor form a cluster.
//! A correction can sit in several clusters; backtesting decides which ones become rules.

use ingrain_core::Correction;
use ingrain_engine::{Token, lang, tokens};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A removed token, scoped to a language family.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Anchor {
    /// Rule language (family), e.g. `TypeScript` for both `.ts` and `.tsx`.
    pub lang: String,
    pub token: String,
    /// The token was called (`fetch(...)`, `x.fetch(...)`).
    pub callee: bool,
    /// tree-sitter kind of the token (`identifier`, `predefined_type`, a keyword...).
    pub kind: String,
}

impl Anchor {
    /// Keywords and operators: anonymous nodes that template rules can't target.
    pub fn is_keyword(&self) -> bool {
        !self.kind.contains("identifier") && self.kind != "predefined_type"
    }
}

/// What a correction removed and added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delta {
    pub removed: Vec<Token>,
    pub added: Vec<Token>,
}

#[derive(Debug, Clone)]
pub struct Cluster {
    pub anchor: Anchor,
    /// Indices into the corrections slice.
    pub members: Vec<usize>,
    /// Distinct fix events (commit, review comment, session) behind the members.
    pub support: usize,
    /// Most common tokens the humans added instead, most frequent first.
    pub replacements: Vec<String>,
}

/// Tokens on the changed lines of each side, diffed as multisets.
pub fn delta(c: &Correction) -> Option<Delta> {
    let language = lang::parse(&c.lang)?;
    let side = |snippet: &ingrain_core::Snippet| -> Vec<Token> {
        tokens::salient(language, &snippet.text)
            .into_iter()
            .filter(|t| snippet.changed.contains(t.line))
            .collect()
    };
    let before = side(&c.before);
    let after = side(&c.after);
    Some(Delta {
        removed: multiset_minus(&before, &after),
        added: multiset_minus(&after, &before),
    })
}

fn multiset_minus(a: &[Token], b: &[Token]) -> Vec<Token> {
    let mut counts: HashMap<&str, i32> = HashMap::new();
    for t in b {
        *counts.entry(t.text.as_str()).or_default() += 1;
    }
    a.iter()
        .filter(|t| {
            let n = counts.entry(t.text.as_str()).or_default();
            *n -= 1;
            *n < 0
        })
        .cloned()
        .collect()
}

/// Anchors of one correction, deduplicated by (token, callee).
pub fn anchors(c: &Correction) -> Vec<Anchor> {
    let Some(language) = lang::parse(&c.lang) else {
        return Vec::new();
    };
    let Some(d) = delta(c) else {
        return Vec::new();
    };
    let family = lang::name(lang::family(language));
    let mut seen = HashSet::new();
    d.removed
        .into_iter()
        .filter(|t| t.text.chars().count() >= 2)
        .filter(|t| seen.insert((t.text.clone(), t.is_callee)))
        .map(|t| Anchor {
            lang: family.clone(),
            token: t.text,
            callee: t.is_callee,
            kind: t.kind,
        })
        .collect()
}

/// Key for "the same fix event": corrections from one commit or comment count once.
pub fn source_key(c: &Correction) -> String {
    match &c.source {
        ingrain_core::Source::GitFixup { fix_commit, .. } => fix_commit.clone(),
        ingrain_core::Source::ReviewSuggestion { url } => url.clone(),
        ingrain_core::Source::Transcript { session } => format!("{session}:{}", c.time),
    }
}

/// Cluster corrections by anchor. Only clusters with `support >= min_support` are returned,
/// largest first.
pub fn cluster(corrections: &[Correction], min_support: usize) -> Vec<Cluster> {
    let mut by_anchor: BTreeMap<Anchor, Vec<usize>> = BTreeMap::new();
    let mut deltas: HashMap<usize, Delta> = HashMap::new();
    for (i, c) in corrections.iter().enumerate() {
        for a in anchors(c) {
            by_anchor.entry(a).or_default().push(i);
        }
        if let Some(d) = delta(c) {
            deltas.insert(i, d);
        }
    }
    let mut clusters: Vec<Cluster> = by_anchor
        .into_iter()
        .filter_map(|(anchor, members)| {
            let support = members
                .iter()
                .map(|&i| source_key(&corrections[i]))
                .collect::<HashSet<_>>()
                .len();
            if support < min_support {
                return None;
            }
            let replacements = top_added(&members, &deltas, &anchor.token);
            Some(Cluster {
                anchor,
                members,
                support,
                replacements,
            })
        })
        .collect();
    clusters.sort_by(|a, b| b.support.cmp(&a.support).then(a.anchor.cmp(&b.anchor)));
    clusters
}

fn top_added(members: &[usize], deltas: &HashMap<usize, Delta>, anchor: &str) -> Vec<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for i in members {
        let Some(d) = deltas.get(i) else { continue };
        let unique: HashSet<&str> = d.added.iter().map(|t| t.text.as_str()).collect();
        for t in unique {
            if t != anchor && t.chars().count() >= 2 {
                *counts.entry(t).or_default() += 1;
            }
        }
    }
    let mut v: Vec<(&str, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    // Keep tokens that appear in at least half the members.
    let needed = members.len().div_ceil(2);
    v.into_iter()
        .filter(|(_, n)| *n >= needed)
        .take(3)
        .map(|(t, _)| t.to_owned())
        .collect()
}

/// Corpus-level numbers used to judge whether corrections are patterns at all.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PatternStats {
    pub corrections: usize,
    /// Corrections that removed at least one salient token.
    pub with_anchor: usize,
    /// Corrections in at least one cluster with `support >= min_support`.
    pub repeated: usize,
}

pub fn pattern_stats(corrections: &[Correction], min_support: usize) -> PatternStats {
    let with_anchor = corrections
        .iter()
        .filter(|c| !anchors(c).is_empty())
        .count();
    let repeated: HashSet<usize> = cluster(corrections, min_support)
        .into_iter()
        .flat_map(|c| c.members)
        .collect();
    PatternStats {
        corrections: corrections.len(),
        with_anchor,
        repeated: repeated.len(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ingrain_core::{LineRange, Snippet, Source};

    pub fn correction(id: &str, lang: &str, before: &str, after: &str, fix: &str) -> Correction {
        let snip = |t: &str| Snippet {
            text: t.into(),
            changed: LineRange::new(1, t.lines().count() as u32),
            first_line: 1,
        };
        Correction {
            id: id.into(),
            path: "src/a.ts".into(),
            lang: lang.into(),
            before: snip(before),
            after: snip(after),
            source: Source::GitFixup {
                agent_commit: "a".into(),
                fix_commit: fix.into(),
            },
            agent_time: 0,
            time: 10,
            note: None,
        }
    }

    #[test]
    fn anchors_are_removed_tokens() {
        let c = correction(
            "1",
            "TypeScript",
            "const r = await fetch(url);",
            "const r = await apiClient.get(url);",
            "f1",
        );
        let a = anchors(&c);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].token, "fetch");
        assert!(a[0].callee);
    }

    #[test]
    fn clusters_by_anchor_and_counts_distinct_fixes() {
        let mk = |id: &str, fix: &str| {
            correction(
                id,
                "TypeScript",
                &format!("const {id} = await fetch(u{id});"),
                &format!("const {id} = await apiClient.get(u{id});"),
                fix,
            )
        };
        let cs = vec![mk("a", "f1"), mk("b", "f1"), mk("c", "f2"), mk("d", "f3")];
        let clusters = cluster(&cs, 3);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].anchor.token, "fetch");
        assert_eq!(clusters[0].support, 3);
        assert_eq!(clusters[0].members.len(), 4);
        assert_eq!(clusters[0].replacements, vec!["apiClient", "get"]);
        assert!(cluster(&cs, 4).is_empty());
        let stats = pattern_stats(&cs, 3);
        assert_eq!(stats.repeated, 4);
    }
}
