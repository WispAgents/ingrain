//! Time-split evaluation: learn rules from corrections before a cutoff, then
//! measure how many later corrections they would have prevented, and how often
//! they fire on later agent code that humans kept.

use crate::backtest;
use crate::cluster;
use crate::propose::{self, ProposeOptions, ProposeStats};
use anyhow::Result;
use ingrain_core::{AgentHunk, Correction};
use ingrain_engine::{Parsed, lang};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Default, Serialize)]
pub struct EvalReport {
    pub corrections: usize,
    pub agent_hunks: usize,
    /// Stage 1: corrections that removed a salient token / sit in a repeated cluster (all data).
    pub with_anchor: usize,
    pub repeated: usize,
    pub cutoff: i64,
    pub train_corrections: usize,
    pub test_corrections: usize,
    pub test_agent_hunks: usize,
    pub rules: Vec<RuleReport>,
    /// Test corrections a frozen rule flags on the agent's lines (and not on the fix).
    pub prevented: usize,
    pub recall: f64,
    pub test_true_hits: u32,
    pub test_false_hits: u32,
    pub precision: Option<f64>,
    /// Milliseconds to parse one agent hunk and run every rule on it.
    pub scan_ms_p50: f64,
    pub scan_ms_p95: f64,
    pub propose: ProposeStatsReport,
    /// Test hits for hand-judging.
    pub samples: Vec<HitSample>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RuleReport {
    pub id: String,
    pub origin: String,
    pub anchor: String,
    pub yaml: String,
    pub train_caught: usize,
    pub test_prevented: usize,
    pub test_true_hits: u32,
    pub test_false_hits: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProposeStatsReport {
    pub clusters: usize,
    pub template_rules: usize,
    pub llm_tried: usize,
    pub llm_rules: usize,
    pub llm_errors: usize,
}

impl From<ProposeStats> for ProposeStatsReport {
    fn from(s: ProposeStats) -> Self {
        Self {
            clusters: s.clusters,
            template_rules: s.template_rules,
            llm_tried: s.llm_tried,
            llm_rules: s.llm_rules,
            llm_errors: s.llm_errors,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HitSample {
    pub rule: String,
    pub commit: String,
    pub path: String,
    pub line: String,
    /// The line was later corrected by a human.
    pub corrected: bool,
}

pub struct EvalOptions<'a> {
    /// Share of corrections (by time) used for training.
    pub split: f64,
    pub propose: ProposeOptions<'a>,
    pub max_samples: usize,
}

pub fn evaluate(
    corrections: &[Correction],
    hunks: &[AgentHunk],
    opts: EvalOptions<'_>,
) -> Result<EvalReport> {
    let min_support = opts.propose.min_support;
    let stats = cluster::pattern_stats(corrections, min_support);
    let mut report = EvalReport {
        corrections: corrections.len(),
        agent_hunks: hunks.len(),
        with_anchor: stats.with_anchor,
        repeated: stats.repeated,
        ..Default::default()
    };
    let mut times: Vec<i64> = corrections.iter().map(|c| c.time).collect();
    times.sort_unstable();
    if times.len() < 2 {
        return Ok(report);
    }
    let idx = ((times.len() as f64 * opts.split) as usize).clamp(1, times.len() - 1);
    let cutoff = times[idx];
    report.cutoff = cutoff;

    let mut popts = opts.propose;
    popts.known_before = cutoff;
    let (proposals, pstats) = propose::propose(corrections, hunks, &popts)?;
    report.propose = pstats.into();
    report.train_corrections = corrections.iter().filter(|c| c.time < cutoff).count();
    let test: Vec<&Correction> = corrections.iter().filter(|c| c.time >= cutoff).collect();
    report.test_corrections = test.len();

    let mut per_rule: HashMap<String, RuleReport> = proposals
        .iter()
        .map(|p| {
            (
                p.rule.id.clone(),
                RuleReport {
                    id: p.rule.id.clone(),
                    origin: p.rule.meta.origin.clone(),
                    anchor: p.anchor.token.clone(),
                    yaml: p.rule.to_yaml(),
                    train_caught: p.caught.len(),
                    ..Default::default()
                },
            )
        })
        .collect();

    for c in &test {
        let mut prevented = false;
        for p in &proposals {
            let flags_before = backtest::flags(&p.compiled, &c.path, &c.before);
            if flags_before && !backtest::flags(&p.compiled, &c.path, &c.after) {
                per_rule
                    .get_mut(&p.rule.id)
                    .expect("rule report")
                    .test_prevented += 1;
                prevented = true;
            }
        }
        report.prevented += usize::from(prevented);
    }
    report.recall = ratio(report.prevented, report.test_corrections);

    let test_hunks: Vec<&AgentHunk> = hunks.iter().filter(|h| h.time >= cutoff).collect();
    report.test_agent_hunks = test_hunks.len();
    let all = backtest::by_id(corrections);
    let known = |t: i64| t >= cutoff;
    // Relevance is per rule, so the cache is keyed by rule id + correction id.
    let mut relevance: HashMap<String, HashMap<String, bool>> = HashMap::new();
    let mut durations = Vec::new();
    for (n, h) in test_hunks.iter().enumerate() {
        let Some(language) = lang::detect(&h.path) else {
            continue;
        };
        let applicable: Vec<_> = proposals
            .iter()
            .filter(|p| p.compiled.applies_to(&h.path))
            .collect();
        let measure = n < 300;
        if applicable.is_empty() && !measure {
            continue;
        }
        let started = Instant::now();
        let parsed = Parsed::new(language, &h.snippet.text);
        let judged: Vec<(&String, backtest::Judged)> = applicable
            .iter()
            .map(|p| {
                let relevant = relevance.entry(p.rule.id.clone()).or_default();
                let j = backtest::judge(&p.compiled, &parsed, h, &all, &known, relevant);
                (&p.rule.id, j)
            })
            .collect();
        if measure {
            durations.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let lines: Vec<&str> = h.snippet.text.lines().collect();
        for (rule, j) in judged {
            let r = per_rule.get_mut(rule).expect("rule report");
            r.test_true_hits += j.true_hits();
            r.test_false_hits += j.false_hits();
            report.test_true_hits += j.true_hits();
            report.test_false_hits += j.false_hits();
            for (line, is_true) in j.hits {
                if report.samples.len() >= opts.max_samples {
                    break;
                }
                report.samples.push(HitSample {
                    rule: rule.clone(),
                    commit: h.commit.chars().take(10).collect(),
                    path: h.path.clone(),
                    line: lines
                        .get(line as usize - 1)
                        .map(|l| l.trim().chars().take(160).collect())
                        .unwrap_or_default(),
                    corrected: is_true,
                });
            }
        }
    }
    let total = report.test_true_hits + report.test_false_hits;
    report.precision = (total > 0).then(|| f64::from(report.test_true_hits) / f64::from(total));
    durations.sort_by(f64::total_cmp);
    report.scan_ms_p50 = percentile(&durations, 0.5);
    report.scan_ms_p95 = percentile(&durations, 0.95);

    let mut rules: Vec<RuleReport> = per_rule.into_values().collect();
    rules.sort_by(|a, b| {
        b.test_prevented
            .cmp(&a.test_prevented)
            .then(a.id.cmp(&b.id))
    });
    report.rules = rules;
    Ok(report)
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { a as f64 / b as f64 }
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[i]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::tests::correction;

    #[test]
    fn rules_learned_before_cutoff_prevent_later_corrections() {
        let cs: Vec<Correction> = (0..8)
            .map(|i| {
                let mut c = correction(
                    &format!("c{i}"),
                    "TypeScript",
                    &format!("const r{i} = await fetch(u{i});"),
                    &format!("const r{i} = await apiClient.get(u{i});"),
                    &format!("fix{i}"),
                );
                c.time = i;
                c
            })
            .collect();
        let report = evaluate(
            &cs,
            &[],
            EvalOptions {
                split: 0.5,
                propose: ProposeOptions::default(),
                max_samples: 10,
            },
        )
        .unwrap();
        assert_eq!(report.train_corrections, 4);
        assert_eq!(report.test_corrections, 4);
        assert_eq!(report.prevented, 4);
        assert!((report.recall - 1.0).abs() < f64::EPSILON);
    }
}
