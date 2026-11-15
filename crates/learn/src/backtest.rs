//! Score a candidate rule against history.

use ingrain_core::{AgentHunk, Backtest, Correction, LineRange, Snippet};
use ingrain_engine::{CompiledRule, Parsed, lang};
use std::collections::HashSet;

/// Backtest result plus the numbers derived from it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Score {
    pub backtest: Backtest,
    /// Correction indices (into the cluster's slice) whose agent code the rule flags.
    pub caught: Vec<usize>,
    /// Hits on lines humans later corrected.
    pub true_hits: u32,
    /// Hits on agent lines humans kept.
    pub false_hits: u32,
}

impl Score {
    /// Share of hits on agent code that landed on corrected lines.
    /// `None` when the rule never fired on agent code.
    pub fn precision(&self) -> Option<f64> {
        let total = self.true_hits + self.false_hits;
        (total > 0).then(|| f64::from(self.true_hits) / f64::from(total))
    }
    pub fn after_rate(&self) -> f64 {
        let b = &self.backtest;
        if b.afters == 0 {
            0.0
        } else {
            f64::from(b.after_hits) / f64::from(b.afters)
        }
    }
}

/// True if the rule fires on the snippet's changed lines.
pub fn flags(rule: &CompiledRule, path: &str, snippet: &Snippet) -> bool {
    if snippet.changed.is_empty() || !rule.applies_to(path) {
        return false;
    }
    let Some(language) = lang::detect(path) else {
        return false;
    };
    rule.scan_text(language, &snippet.text)
        .iter()
        .any(|h| LineRange::new(h.start_line, h.end_line).overlaps(&snippet.changed))
}

/// Score `rule` on a cluster's corrections and on the agent-hunk population.
///
/// `prefilter` is a token the matched code must contain; hunks without it are
/// skipped without parsing. Only hunks written in `[since, known_before)` count:
/// code from before humans first made this correction was fine at the time (think
/// of a renamed API), and `known_before` keeps a time-split evaluation from seeing
/// the future.
pub fn score(
    rule: &CompiledRule,
    corrections: &[&Correction],
    hunks: &[AgentHunk],
    prefilter: Option<&str>,
    since: i64,
    known_before: i64,
) -> Score {
    let mut s = Score::default();
    for (i, c) in corrections.iter().enumerate() {
        s.backtest.befores += 1;
        if flags(rule, &c.path, &c.before) {
            s.backtest.before_hits += 1;
            s.caught.push(i);
        }
        if !c.after.changed.is_empty() {
            s.backtest.afters += 1;
            if flags(rule, &c.path, &c.after) {
                s.backtest.after_hits += 1;
            }
        }
    }

    for h in hunks
        .iter()
        .filter(|h| h.time >= since && h.time < known_before)
    {
        if prefilter.is_some_and(|p| !h.snippet.text.contains(p)) || !rule.applies_to(&h.path) {
            continue;
        }
        let Some(language) = lang::detect(&h.path) else {
            continue;
        };
        s.backtest.kept += 1;
        let parsed = Parsed::new(language, &h.snippet.text);
        let corrected: Vec<&LineRange> = h.corrected_before(known_before).collect();
        let mut hit_lines = HashSet::new();
        for hit in rule.scan(&parsed) {
            let range = LineRange::new(hit.start_line, hit.end_line);
            if !range.overlaps(&h.snippet.changed) || !hit_lines.insert(hit.start_line) {
                continue;
            }
            if corrected.iter().any(|c| c.overlaps(&range)) {
                s.true_hits += 1;
            } else {
                s.false_hits += 1;
                s.backtest.kept_hits += 1;
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::tests::correction;
    use ingrain_core::CorrectedLines;

    const RULE: &str =
        "id: no-fetch\nlanguage: TypeScript\nrule:\n  pattern: fetch($$$A)\nmessage: m\n";

    fn hunk(text: &str, corrected: Option<LineRange>, time: i64) -> AgentHunk {
        AgentHunk {
            commit: "a".into(),
            path: "src/x.ts".into(),
            lang: "TypeScript".into(),
            time,
            snippet: Snippet {
                text: text.into(),
                changed: LineRange::new(1, text.lines().count() as u32),
                first_line: 1,
            },
            corrected: corrected
                .map(|lines| CorrectedLines {
                    lines,
                    time: 5,
                    correction: "c".into(),
                })
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn scores_corrections_and_hunks() {
        let rule = CompiledRule::compile(RULE).unwrap();
        let c = correction(
            "1",
            "TypeScript",
            "await fetch(u);",
            "await api.get(u);",
            "f",
        );
        let hunks = vec![
            hunk("a();\nawait fetch(u);", Some(LineRange::new(2, 2)), 1),
            hunk("await fetch(v);", None, 1),
            hunk("nothing here", None, 1),
        ];
        let s = score(&rule, &[&c], &hunks, Some("fetch"), 0, i64::MAX);
        assert_eq!(s.backtest.before_hits, 1);
        assert_eq!(s.backtest.after_hits, 0);
        assert_eq!(s.true_hits, 1);
        assert_eq!(s.false_hits, 1);
        assert_eq!(s.precision(), Some(0.5));
        assert_eq!(
            s.backtest.kept, 2,
            "prefilter skipped the hunk without `fetch`"
        );
    }

    #[test]
    fn hides_the_future() {
        let rule = CompiledRule::compile(RULE).unwrap();
        let hunks = vec![hunk("await fetch(u);", Some(LineRange::new(1, 1)), 1)];
        // The correction happened at t=5; at t=3 the line looks kept.
        let s = score(&rule, &[], &hunks, None, 0, 3);
        assert_eq!((s.true_hits, s.false_hits), (0, 1));
        // Code written before the window doesn't count at all.
        let s = score(&rule, &[], &hunks, None, 2, i64::MAX);
        assert_eq!((s.true_hits, s.false_hits), (0, 0));
    }
}
