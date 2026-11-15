//! Running accepted rules on added lines. Shared by the hook, pre-commit and CI paths.

use crate::repo::Repo;
use anyhow::Result;
use ingrain_core::{DiffBase, Example, LineRange, Mode, RuleFile};
use ingrain_engine::{CompiledRule, Parsed, lang};

/// A file to check and the lines that count.
pub struct Target {
    pub path: String,
    pub added: Vec<LineRange>,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub rule: String,
    pub mode: Mode,
    pub path: String,
    pub line: u32,
    pub message: String,
    pub matched: String,
    pub example: Option<Example>,
    /// Set when the line carries `ingrain-ignore <rule>: <reason>`.
    pub override_reason: Option<String>,
    /// The line names the rule in an ignore comment but gives no reason.
    pub missing_reason: bool,
}

impl Finding {
    /// Counts against the agent / the build.
    pub fn is_active(&self) -> bool {
        self.override_reason.is_none()
    }
}

/// Files with added lines relative to `base`, optionally limited to `only`.
pub fn targets(repo: &Repo, base: &DiffBase, only: Option<&[String]>) -> Result<Vec<Target>> {
    let files: Vec<(String, Vec<LineRange>)> = match only {
        Some(paths) => paths
            .iter()
            .map(|p| Ok((p.clone(), repo.git.added_lines_for(base, p)?)))
            .collect::<Result<_>>()?,
        None => repo.git.added_lines(base)?,
    };
    let mut out = Vec::new();
    for (path, added) in files {
        if added.is_empty() || lang::detect(&path).is_none() {
            continue;
        }
        let content = match base {
            DiffBase::Staged => repo.git.show("", &path),
            _ => std::fs::read_to_string(repo.git.root().join(&path)).ok(),
        };
        if let Some(content) = content {
            out.push(Target {
                path,
                added,
                content,
            });
        }
    }
    Ok(out)
}

/// Every hit of every applicable rule on added lines.
pub fn find(rules: &[(RuleFile, CompiledRule)], targets: &[Target]) -> Vec<Finding> {
    let mut out = Vec::new();
    for t in targets {
        let applicable: Vec<&(RuleFile, CompiledRule)> = rules
            .iter()
            .filter(|(_, c)| c.applies_to(&t.path))
            .collect();
        if applicable.is_empty() {
            continue;
        }
        let Some(language) = lang::detect(&t.path) else {
            continue;
        };
        let parsed = Parsed::new(language, &t.content);
        let lines: Vec<&str> = t.content.lines().collect();
        for (file, compiled) in applicable {
            for hit in compiled.scan(&parsed) {
                let range = LineRange::new(hit.start_line, hit.end_line);
                if !t.added.iter().any(|a| a.overlaps(&range)) {
                    continue;
                }
                let (override_reason, missing_reason) =
                    ignore_comment(&lines, hit.start_line, &file.id);
                out.push(Finding {
                    rule: file.id.clone(),
                    mode: file.meta.mode,
                    path: t.path.clone(),
                    line: hit.start_line,
                    message: hit.message,
                    matched: hit.matched,
                    example: file.meta.example.clone(),
                    override_reason,
                    missing_reason,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.rule.cmp(&b.rule))
    });
    out
}

/// Look for `ingrain-ignore <rule>: <reason>` on the hit line or the line above.
fn ignore_comment(lines: &[&str], line: u32, rule: &str) -> (Option<String>, bool) {
    let idx = line as usize - 1;
    let candidates = [idx.checked_sub(1), Some(idx)];
    for i in candidates.into_iter().flatten() {
        let Some(text) = lines.get(i) else { continue };
        let Some(pos) = text.find("ingrain-ignore") else {
            continue;
        };
        let rest = text[pos + "ingrain-ignore".len()..].trim_start();
        let Some(after_id) = rest.strip_prefix(rule) else {
            continue;
        };
        if after_id.starts_with(|c: char| c.is_alphanumeric() || c == '-') {
            continue; // a longer rule id
        }
        let reason = after_id
            .trim_start()
            .strip_prefix(':')
            .map(|r| {
                r.trim()
                    .trim_end_matches("*/")
                    .trim_end_matches("-->")
                    .trim()
            })
            .filter(|r| !r.is_empty());
        return match reason {
            Some(r) => (Some(r.to_owned()), false),
            None => (None, true),
        };
    }
    (None, false)
}

/// Text sent to the agent for the findings it must act on.
pub fn agent_message(findings: &[&Finding], blocking: bool) -> String {
    let mut msg = format!(
        "ingrain: {} learned rule{} {} in code you just wrote. These rules come from corrections humans made to agent code in this repo.\n",
        findings.len(),
        if findings.len() == 1 { "" } else { "s" },
        if findings.len() == 1 { "fires" } else { "fire" }
    );
    for f in findings {
        msg.push_str(&format!(
            "\n- {} at {}:{}: {}\n",
            f.rule, f.path, f.line, f.message
        ));
        msg.push_str(&format!("  matched: {}\n", f.matched));
        if let Some(ex) = &f.example {
            msg.push_str("  past fix:\n");
            for l in ex.before.lines() {
                msg.push_str(&format!("    - {l}\n"));
            }
            for l in ex.after.lines() {
                msg.push_str(&format!("    + {l}\n"));
            }
        }
        if f.missing_reason {
            msg.push_str("  (an ingrain-ignore comment names this rule but gives no reason, so it doesn't count)\n");
        }
    }
    if blocking {
        msg.push_str(
            "\nFix these before finishing. If the code is right as written, keep it and add a comment \
             `ingrain-ignore <rule-id>: <reason>` on that line.\n",
        );
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_comments_need_a_reason() {
        let lines = vec![
            "// ingrain-ignore avoid-fetch-call: streaming endpoint",
            "await fetch(u);",
            "await fetch(v); // ingrain-ignore avoid-fetch-call",
            "await fetch(w); // ingrain-ignore avoid-fetch-call-2: other rule",
        ];
        assert_eq!(
            ignore_comment(&lines, 2, "avoid-fetch-call"),
            (Some("streaming endpoint".into()), false)
        );
        assert_eq!(ignore_comment(&lines, 3, "avoid-fetch-call"), (None, true));
        assert_eq!(ignore_comment(&lines, 4, "avoid-fetch-call"), (None, true));
    }

    fn rule(mode: Mode) -> (RuleFile, CompiledRule) {
        let yaml = "id: avoid-fetch-call\nlanguage: TypeScript\nrule:\n  pattern: fetch($$$A)\nmessage: use apiClient\n";
        let mut file = RuleFile::parse(yaml, Default::default()).unwrap();
        file.set_mode(mode);
        let compiled = CompiledRule::compile(&file.to_yaml()).unwrap();
        (file, compiled)
    }

    #[test]
    fn only_added_lines_count() {
        let rules = vec![rule(Mode::Block)];
        let t = Target {
            path: "src/a.ts".into(),
            added: vec![LineRange::new(2, 2)],
            content: "await fetch(old);\nawait fetch(new);\n".into(),
        };
        let found = find(&rules, &[t]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 2);
        assert_eq!(found[0].mode, Mode::Block);
        let msg = agent_message(&found.iter().collect::<Vec<_>>(), true);
        assert!(msg.contains("avoid-fetch-call at src/a.ts:2"));
    }
}
