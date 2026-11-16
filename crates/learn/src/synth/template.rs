//! Deterministic rule candidates: banned calls and banned identifiers.

use super::{Candidate, Origin, rule_yaml};
use crate::cluster::Cluster;
use ingrain_core::Correction;
use std::collections::HashMap;

/// Candidates for a cluster, most specific first.
pub fn candidates(cluster: &Cluster, corrections: &[&Correction], id: &str) -> Vec<Candidate> {
    let a = &cluster.anchor;
    let message = message(cluster);
    let mut out = Vec::new();
    let mut push = |rule: serde_yaml::Value, label: String| {
        out.push(Candidate {
            yaml: rule_yaml(id, &a.lang, rule, &message),
            origin: Origin::Template,
            label,
        });
    };

    if a.callee {
        let receivers = receivers(&a.token, corrections);
        let total: usize = receivers.values().sum();
        if let Some((recv, n)) = receivers
            .iter()
            .filter(|(r, _)| !r.is_empty() && r.as_str() != EXPR)
            .max_by_key(|(_, n)| **n)
            && *n * 3 >= total * 2
        {
            push(
                pattern(&format!("{recv}.{}($$$ARGS)", a.token)),
                format!("{recv}.{}(...)", a.token),
            );
        }
        let bare = receivers.get("").copied().unwrap_or(0);
        if bare * 2 >= total {
            push(
                pattern(&format!("{}($$$ARGS)", a.token)),
                format!("{}(...)", a.token),
            );
        }
        push(
            pattern(&format!("$OBJ.{}($$$ARGS)", a.token)),
            format!(".{}(...)", a.token),
        );
    } else if !a.is_keyword() && is_plain_identifier(&a.token) {
        let mut rule = serde_yaml::Mapping::new();
        rule.insert("kind".into(), a.kind.clone().into());
        rule.insert("regex".into(), format!("^{}$", escape(&a.token)).into());
        push(serde_yaml::Value::Mapping(rule), format!("`{}`", a.token));
    }
    out
}

fn pattern(p: &str) -> serde_yaml::Value {
    let mut rule = serde_yaml::Mapping::new();
    rule.insert("pattern".into(), p.into());
    serde_yaml::Value::Mapping(rule)
}

/// How the token is called on the corrected lines: receiver name → count.
/// The empty string counts bare calls (`fetch(...)`).
fn receivers(token: &str, corrections: &[&Correction]) -> HashMap<String, usize> {
    let mut out: HashMap<String, usize> = HashMap::new();
    let needle = format!("{token}(");
    for c in corrections {
        let text = c.before.changed_text();
        let mut from = 0;
        while let Some(pos) = text[from..].find(&needle) {
            let at = from + pos;
            from = at + needle.len();
            let prev = text[..at].chars().last();
            if prev.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
                continue; // part of a longer name
            }
            let recv = if prev == Some('.') {
                let head = &text[..at - 1];
                let start = head
                    .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$'))
                    .map_or(0, |i| i + 1);
                head[start..].to_owned()
            } else {
                String::new()
            };
            // `foo().bar(` has no named receiver; it only supports the generic member form.
            let key = if prev == Some('.') && recv.is_empty() {
                EXPR.to_owned()
            } else {
                recv
            };
            *out.entry(key).or_default() += 1;
        }
    }
    out
}

/// Receiver key for calls on an unnamed expression.
const EXPR: &str = "<expr>";

fn is_plain_identifier(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

fn escape(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_alphanumeric() || c == '_' {
                vec![c]
            } else {
                vec!['\\', c]
            }
        })
        .collect()
}

/// A one-line, actionable message from the cluster.
pub fn message(cluster: &Cluster) -> String {
    let a = &cluster.anchor;
    let what = if a.callee {
        format!("`{}(...)`", a.token)
    } else {
        format!("`{}`", a.token)
    };
    let n = cluster.support;
    let corrections = if n == 1 {
        "1 past correction".to_owned()
    } else {
        format!("{n} past corrections")
    };
    if cluster.replacements.is_empty() {
        format!("Avoid {what} here: humans removed it in {corrections} of agent code.")
    } else {
        let with: Vec<String> = cluster
            .replacements
            .iter()
            .map(|r| format!("`{r}`"))
            .collect();
        format!(
            "Avoid {what} here: in {corrections} humans replaced it with {}.",
            with.join(" / ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{self, tests::correction};
    use ingrain_engine::CompiledRule;

    #[test]
    fn call_candidates_compile_and_prefer_receiver() {
        let cs: Vec<Correction> = (0..3)
            .map(|i| {
                correction(
                    &i.to_string(),
                    "TypeScript",
                    &format!("console.log(v{i});"),
                    &format!("logger.debug(v{i});"),
                    &format!("f{i}"),
                )
            })
            .collect();
        let clusters = cluster::cluster(&cs, 3);
        let log = clusters.iter().find(|c| c.anchor.token == "log").unwrap();
        let refs: Vec<&Correction> = cs.iter().collect();
        let cands = candidates(log, &refs, "avoid-log-call");
        assert_eq!(cands[0].label, "console.log(...)");
        for c in &cands {
            CompiledRule::compile(&c.yaml).unwrap();
        }
        assert!(
            cands[0].yaml.contains("logger"),
            "message names the replacement"
        );
    }

    #[test]
    fn identifier_candidate() {
        let cs: Vec<Correction> = (0..3)
            .map(|i| {
                correction(
                    &i.to_string(),
                    "TypeScript",
                    &format!("let v{i}: any = 1;"),
                    &format!("let v{i}: number = 1;"),
                    &format!("f{i}"),
                )
            })
            .collect();
        let clusters = cluster::cluster(&cs, 3);
        let any = clusters.iter().find(|c| c.anchor.token == "any").unwrap();
        let refs: Vec<&Correction> = cs.iter().collect();
        let cands = candidates(any, &refs, "avoid-any");
        assert_eq!(cands.len(), 1);
        let rule = CompiledRule::compile(&cands[0].yaml).unwrap();
        assert_eq!(
            rule.scan_text(ingrain_engine::Lang::TypeScript, "let x: any = 2;")
                .len(),
            1
        );
        assert_eq!(
            rule.scan_text(ingrain_engine::Lang::TypeScript, "let x: number = 2;")
                .len(),
            0
        );
    }
}
