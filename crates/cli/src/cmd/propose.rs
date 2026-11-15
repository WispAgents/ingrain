use super::Outcome;
use crate::repo::Repo;
use anyhow::Result;
use ingrain_core::{AgentHunk, Correction, store};
use ingrain_engine::{CompiledRule, lang};
use ingrain_learn::propose::{self, ProposeOptions};
use ingrain_learn::synth::llm;
use std::collections::HashSet;
use std::path::Path;
use std::process::ExitCode;

#[derive(clap::Args)]
pub struct Args {
    /// Model backend for clusters templates can't express: auto, claude, api or none.
    #[arg(long, default_value = "auto")]
    llm: String,
    /// Model id (default claude-opus-5-5).
    #[arg(long)]
    model: Option<String>,
    /// Clusters sent to the model at most.
    #[arg(long, default_value_t = 10)]
    llm_clusters: usize,
    /// Print every cluster and how each candidate rule scored.
    #[arg(long)]
    explain: bool,
}

pub fn run(dir: &Path, args: Args) -> Outcome {
    let repo = Repo::open(dir)?;
    let corrections: Vec<Correction> = store::read(&repo.layout.corrections_file())?;
    let hunks: Vec<AgentHunk> = store::read(&repo.layout.agent_hunks_file())?;
    if corrections.is_empty() {
        println!("no corrections yet; run `ingrain mine` first");
        return Ok(ExitCode::SUCCESS);
    }
    let existing: Vec<CompiledRule> = repo.compiled_rules()?.into_iter().map(|(_, c)| c).collect();
    let backend = llm::backend(&args.llm, args.model.as_deref())?;
    if let Some(b) = &backend {
        eprintln!("rule writing falls back to {}", b.describe());
    }
    let opts = ProposeOptions {
        min_support: repo.config.min_support as usize,
        min_precision: repo.config.min_precision,
        llm: backend.as_deref(),
        llm_clusters: args.llm_clusters,
        taken_ids: taken_ids(&repo)?,
        existing: &existing,
        ..Default::default()
    };
    let (mut proposals, stats) = propose::propose(&corrections, &hunks, &opts)?;
    // A rejected rule must not come back under another id.
    let rejected: Vec<serde_yaml::Value> = super::review::rejected(&repo)?
        .iter()
        .filter_map(|r| rule_body(&r.rule))
        .collect();
    proposals.retain(|p| rule_body(&p.rule.to_yaml()).is_none_or(|b| !rejected.contains(&b)));
    println!(
        "{} corrections, {} clusters with {}+ fixes → {} rules from templates, {} from the model ({} tried, {} errors)",
        stats.corrections,
        stats.clusters,
        opts.min_support,
        stats.template_rules,
        stats.llm_rules,
        stats.llm_tried,
        stats.llm_errors
    );
    if args.explain {
        print_explain(&stats.explain);
    }
    for mut p in proposals {
        let head_hits = head_hits(&repo, &p.compiled, &p.anchor.token)?;
        if let Some(b) = p.rule.meta.backtest.as_mut() {
            b.head_hits = head_hits;
        }
        let path = repo
            .layout
            .rule_path(&repo.layout.pending_dir(), &p.rule.id);
        p.rule.save(&path)?;
        let precision = p
            .score
            .precision()
            .map_or("n/a".to_owned(), |v| format!("{:.0}%", v * 100.0));
        println!(
            "  {:<32} catches {:>3} corrections · precision {:>4} · {} hits in current code · {}",
            p.rule.id,
            p.caught.len(),
            precision,
            head_hits,
            p.rule.meta.origin
        );
    }
    println!("\nnext: `ingrain review`");
    Ok(ExitCode::SUCCESS)
}

fn print_explain(clusters: &[ingrain_learn::propose::ClusterExplain]) {
    for c in clusters {
        println!(
            "cluster `{}` ({}) · {} fixes, {} corrections",
            c.anchor, c.lang, c.support, c.members
        );
        if c.candidates.is_empty() {
            println!("    no template can express this (keyword or operator); needs the model");
        }
        for k in &c.candidates {
            println!(
                "    {:<28} catches {}/{} · flags {} fixed versions · {} true / {} false hits on agent code · {}",
                k.label,
                k.before_hits,
                k.befores,
                k.after_hits,
                k.true_hits,
                k.false_hits,
                if k.accepted { "ACCEPTED" } else { "rejected" }
            );
        }
    }
}

/// The `rule:` block of a rule document, for comparing rules regardless of id or message.
fn rule_body(yaml: &str) -> Option<serde_yaml::Value> {
    let doc: serde_yaml::Mapping = serde_yaml::from_str(yaml).ok()?;
    let mut body = serde_yaml::Mapping::new();
    for key in ["language", "rule"] {
        body.insert(key.into(), doc.get(key)?.clone());
    }
    Some(serde_yaml::Value::Mapping(body))
}

fn taken_ids(repo: &Repo) -> Result<HashSet<String>> {
    let mut ids: HashSet<String> = repo.accepted()?.into_iter().map(|r| r.id).collect();
    ids.extend(repo.pending()?.into_iter().map(|r| r.id));
    ids.extend(super::review::rejected(repo)?.into_iter().map(|r| r.id));
    Ok(ids)
}

/// Matches in tracked files today. Files are prefiltered with `git grep` on the anchor token.
fn head_hits(repo: &Repo, rule: &CompiledRule, token: &str) -> Result<u32> {
    let Some(listing) = repo.git.try_run(&["grep", "-l", "-F", "-e", token]) else {
        return Ok(0); // git grep exits 1 when nothing matches
    };
    let mut n = 0;
    for path in listing.lines().filter(|p| rule.applies_to(p)) {
        let (Some(language), Ok(text)) = (
            lang::detect(path),
            std::fs::read_to_string(repo.git.root().join(path)),
        ) else {
            continue;
        };
        n += rule.scan_text(language, &text).len() as u32;
    }
    Ok(n)
}
