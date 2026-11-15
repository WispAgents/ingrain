//! Turn a cluster into a rule: template candidates first, then an LLM.

pub mod llm;
pub mod template;

use crate::backtest::Score;
use crate::cluster::Cluster;
use anyhow::{Context, Result};
use ingrain_core::Correction;
use ingrain_engine::CompiledRule;
use llm::Llm;
use serde_yaml::{Mapping, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Template,
    Llm,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Template => "template",
            Origin::Llm => "llm",
        }
    }
}

/// A candidate rule document.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub yaml: String,
    pub origin: Origin,
    /// Short description of what it matches, for logs.
    pub label: String,
}

/// Build rule YAML from parts.
pub fn rule_yaml(id: &str, lang: &str, rule: Value, message: &str) -> String {
    let mut doc = Mapping::new();
    doc.insert("id".into(), id.into());
    doc.insert("language".into(), lang.into());
    doc.insert("severity".into(), "warning".into());
    doc.insert("rule".into(), rule);
    doc.insert("message".into(), message.into());
    serde_yaml::to_string(&doc).expect("mapping serializes")
}

/// Ask the model for a rule, retrying with backtest feedback.
pub fn with_llm(
    llm: &dyn Llm,
    cluster: &Cluster,
    members: &[&Correction],
    id: &str,
    attempts: usize,
    evaluate: &dyn Fn(&CompiledRule) -> Score,
    accept: &dyn Fn(&Score) -> bool,
) -> Result<Option<(Candidate, CompiledRule, Score)>> {
    let mut feedback = String::new();
    for _ in 0..attempts {
        let reply = llm.complete(&prompt(cluster, members, id, &feedback))?;
        let yaml = match clean(&reply, id, &cluster.anchor.lang) {
            Ok(y) => y,
            Err(e) => {
                feedback = format!("Your previous reply was not a usable rule: {e:#}.");
                continue;
            }
        };
        let compiled = match CompiledRule::compile(&yaml) {
            Ok(c) => c,
            Err(e) => {
                feedback =
                    format!("Your previous rule:\n```yaml\n{yaml}```\nfailed to compile: {e:#}");
                continue;
            }
        };
        let score = evaluate(&compiled);
        if accept(&score) {
            let candidate = Candidate {
                yaml,
                origin: Origin::Llm,
                label: format!("llm rule for `{}`", cluster.anchor.token),
            };
            return Ok(Some((candidate, compiled, score)));
        }
        feedback = describe_failure(&yaml, &score, members.len());
    }
    Ok(None)
}

fn describe_failure(yaml: &str, s: &Score, n: usize) -> String {
    let b = &s.backtest;
    let mut problems = Vec::new();
    if (b.before_hits as usize) < n.div_ceil(2) {
        problems.push(format!(
            "it matched only {} of {} BEFORE examples on the marked lines",
            b.before_hits, n
        ));
    }
    if b.after_hits > 0 {
        problems.push(format!(
            "it matched {} AFTER examples, which are the corrected code",
            b.after_hits
        ));
    }
    if let Some(p) = s.precision() {
        problems.push(format!(
            "on other agent code it fired {} times on lines humans kept and {} times on lines they corrected (precision {:.0}%)",
            s.false_hits,
            s.true_hits,
            p * 100.0
        ));
    }
    format!(
        "Your previous rule:\n```yaml\n{yaml}```\nwas not good enough: {}. Write a better rule.",
        problems.join("; ")
    )
}

fn prompt(cluster: &Cluster, members: &[&Correction], id: &str, feedback: &str) -> String {
    let a = &cluster.anchor;
    let mut examples = String::new();
    for (i, c) in members.iter().take(6).enumerate() {
        examples.push_str(&format!(
            "### Example {} ({})\nBEFORE (agent code; corrected lines start with `>>`):\n```\n{}\n```\nAFTER (human replacement for the marked lines):\n```\n{}\n```\n\n",
            i + 1,
            c.path,
            marked(&c.before),
            c.after.changed_text()
        ));
    }
    format!(
        "You write ast-grep rules (YAML) that flag code an AI coding agent wrote and a human later corrected.\n\n\
         Language: {lang}\n\
         These corrections all removed `{token}`. Each shows the agent's code and what the human replaced the marked lines with.\n\n\
         {examples}\
         Write ONE ast-grep rule that:\n\
         - matches the problem in the marked BEFORE lines of most examples,\n\
         - does NOT match the AFTER code,\n\
         - is specific enough not to fire on ordinary correct code (prefer a precise `pattern`; use `inside`, `has` or `not` only when needed),\n\
         - has `id: {id}`, `language: {lang}` and `severity: warning`,\n\
         - has a one-sentence `message` telling the agent what to write instead, based on the AFTER code.\n\
         Do not add `fix`, `files`, `ignores`, `metadata` or `note`. Reply with only the YAML in a ```yaml block.\n\
         {feedback}",
        lang = a.lang,
        token = a.token,
    )
}

fn marked(s: &ingrain_core::Snippet) -> String {
    s.text
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if s.changed.contains(i as u32 + 1) {
                format!(">> {l}")
            } else {
                format!("   {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the YAML block and force the fields we own.
fn clean(reply: &str, id: &str, lang: &str) -> Result<String> {
    let body = match reply.find("```yaml") {
        Some(start) => {
            let rest = &reply[start + 7..];
            &rest[..rest.find("```").unwrap_or(rest.len())]
        }
        None => reply,
    };
    let mut doc: Mapping = serde_yaml::from_str(body).context("not a YAML mapping")?;
    for key in ["fix", "files", "ignores", "metadata", "note", "rewriters"] {
        doc.remove(key);
    }
    doc.insert("id".into(), id.into());
    doc.insert("language".into(), lang.into());
    if !doc.contains_key("rule") {
        anyhow::bail!("the rule has no `rule` key");
    }
    Ok(serde_yaml::to_string(&doc)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_reply() {
        let reply = "Here:\n```yaml\nid: wrong\nlanguage: Python\nrule:\n  pattern: print($$$A)\nmessage: use logging\nfix: logging.info($$$A)\n```\n";
        let yaml = clean(reply, "avoid-print", "Python").unwrap();
        assert!(yaml.contains("id: avoid-print"));
        assert!(!yaml.contains("fix"));
        CompiledRule::compile(&yaml).unwrap();
        assert!(clean("no yaml here: [", "x", "Python").is_err());
    }
}
