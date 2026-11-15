use crate::cmd::Outcome;
use crate::repo::Repo;
use ingrain_core::{AgentHunk, Correction, store};
use ingrain_learn::eval::{self, EvalOptions};
use ingrain_learn::propose::ProposeOptions;
use ingrain_learn::synth::llm;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// Share of corrections (oldest first) used to learn rules.
    #[arg(long, default_value_t = 0.6)]
    split: f64,
    /// Model backend for clusters templates can't express: auto, claude, api or none.
    #[arg(long, default_value = "none")]
    llm: String,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, default_value_t = 10)]
    llm_clusters: usize,
    /// Write the full report as JSON.
    #[arg(long)]
    json: Option<PathBuf>,
}

pub fn run(dir: &Path, args: Args) -> Outcome {
    let repo = Repo::open(dir)?;
    let corrections: Vec<Correction> = store::read(&repo.layout.corrections_file())?;
    let hunks: Vec<AgentHunk> = store::read(&repo.layout.agent_hunks_file())?;
    let backend = llm::backend(&args.llm, args.model.as_deref())?;
    let report = eval::evaluate(
        &corrections,
        &hunks,
        EvalOptions {
            split: args.split,
            propose: ProposeOptions {
                min_support: repo.config.min_support as usize,
                min_precision: repo.config.min_precision,
                llm: backend.as_deref(),
                llm_clusters: args.llm_clusters,
                ..Default::default()
            },
            max_samples: 200,
        },
    )?;

    let pct = |a: usize, b: usize| {
        if b == 0 {
            0.0
        } else {
            100.0 * a as f64 / b as f64
        }
    };
    println!(
        "corrections: {} · agent hunks: {}",
        report.corrections, report.agent_hunks
    );
    println!(
        "patterns: {:.0}% removed a salient token · {:.0}% sit in a cluster of {}+ fixes",
        pct(report.with_anchor, report.corrections),
        pct(report.repeated, report.corrections),
        repo.config.min_support
    );
    println!(
        "split: learn on {} corrections, test on {} (and {} agent hunks)",
        report.train_corrections, report.test_corrections, report.test_agent_hunks
    );
    println!("rules learned: {}", report.rules.len());
    for r in &report.rules {
        println!(
            "  {:<34} {:<8} train {:>3} · test prevented {:>3} · hits {} true / {} false",
            r.id, r.origin, r.train_caught, r.test_prevented, r.test_true_hits, r.test_false_hits
        );
    }
    println!(
        "prevented: {}/{} later corrections ({:.1}%)",
        report.prevented,
        report.test_corrections,
        report.recall * 100.0
    );
    match report.precision {
        Some(p) => println!(
            "precision on later agent code: {:.0}% ({} true, {} false)",
            p * 100.0,
            report.test_true_hits,
            report.test_false_hits
        ),
        None => println!("precision on later agent code: no hits"),
    }
    println!(
        "scan time per hunk: p50 {:.2} ms · p95 {:.2} ms",
        report.scan_ms_p50, report.scan_ms_p95
    );
    if let Some(path) = args.json {
        std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
        println!("report: {}", path.display());
    }
    Ok(ExitCode::SUCCESS)
}
