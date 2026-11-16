//! Deterministic checks of changed code against live decisions.
//!
//! Only added lines count, so existing code never fails a check. A line (or the line above)
//! carrying `decisions-ignore D-0012: <reason>` overrides that decision there; a reason is required.

use crate::manifest;
use crate::record::Record;
use crate::scope::Scope;
use ingrain_core::LineRange;
use ingrain_engine::lang;
use ingrain_engine::{CompiledRule, Parsed};
use std::collections::HashMap;

/// One file as it will be after the change.
pub struct Target {
    pub path: String,
    pub content: String,
    /// Added lines (1-based). `None` means the whole file counts as added.
    pub added: Option<Vec<LineRange>>,
    /// The file before the change (`None` for a new file). Used to compare manifests.
    pub before: Option<String>,
}

impl Target {
    fn is_added(&self, line: u32) -> bool {
        self.added
            .as_ref()
            .is_none_or(|r| r.iter().any(|r| r.contains(line)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub decision: String,
    pub path: String,
    pub line: u32,
    /// What broke the decision, e.g. "imports `axios`".
    pub what: String,
    pub code: String,
    /// Set when the line carries a `decisions-ignore` comment with a reason.
    pub overridden: Option<String>,
}

/// Compiled checks for a set of decisions, reusable across files.
pub struct Checker<'a> {
    records: Vec<&'a Record>,
    patterns: HashMap<String, Vec<CompiledRule>>,
}

impl<'a> Checker<'a> {
    /// Live decisions with checks. Patterns that don't compile are reported and skipped.
    pub fn new(records: &'a [Record]) -> Self {
        let records: Vec<&Record> = records
            .iter()
            .filter(|r| r.is_live() && !r.checks.is_empty())
            .collect();
        let mut patterns = HashMap::new();
        for r in &records {
            let compiled: Vec<CompiledRule> = r
                .checks
                .forbid_patterns
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    let yaml = format!(
                        "id: {}-{i}\nlanguage: {}\nrule:\n  pattern: {}\nmessage: x\n",
                        r.id,
                        serde_json::to_string(&p.language).expect("string serializes"),
                        serde_json::to_string(&p.pattern).expect("string serializes")
                    );
                    CompiledRule::compile(&yaml)
                        .map_err(|e| {
                            eprintln!("decisions: {} pattern `{}` skipped: {e:#}", r.id, p.pattern)
                        })
                        .ok()
                })
                .collect();
            patterns.insert(r.id.clone(), compiled);
        }
        Self { records, patterns }
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn check(&self, target: &Target) -> Vec<Violation> {
        let lines: Vec<&str> = target.content.lines().collect();
        let language = lang::detect(&target.path);
        let parsed = language.map(|l| Parsed::new(l, &target.content));
        let new_deps = added_deps(target);
        let mut out = Vec::new();
        for r in &self.records {
            // Dependency decisions are about the project, so they apply to every manifest.
            for dep in &new_deps {
                if let Some(forbidden) = r
                    .checks
                    .forbid_deps
                    .iter()
                    .find(|f| manifest::same(&target.path, dep, f))
                {
                    let line = lines
                        .iter()
                        .position(|l| l.to_lowercase().contains(dep))
                        .map_or(1, |i| i as u32 + 1);
                    out.push(violation(
                        r,
                        target,
                        &lines,
                        line,
                        format!("adds dependency `{forbidden}`"),
                    ));
                }
            }
            if Scope::of(r)
                .matches(&target.path, Some(&target.content))
                .is_none()
            {
                continue;
            }
            if let Some(syntax) = language.and_then(ImportSyntax::of) {
                for (i, line) in lines.iter().enumerate() {
                    let n = i as u32 + 1;
                    if !target.is_added(n) {
                        continue;
                    }
                    let Some(module) = syntax.module(line) else {
                        continue;
                    };
                    for forbidden in &r.checks.forbid_imports {
                        if module_matches(&module, forbidden) {
                            out.push(violation(
                                r,
                                target,
                                &lines,
                                n,
                                format!("imports `{module}`"),
                            ));
                        }
                    }
                }
            }
            if let Some(parsed) = &parsed {
                for rule in self.patterns.get(&r.id).into_iter().flatten() {
                    if !rule.applies_to(&target.path) {
                        continue;
                    }
                    for hit in rule.scan(parsed) {
                        let added = (hit.start_line..=hit.end_line).any(|l| target.is_added(l));
                        if added {
                            out.push(violation(
                                r,
                                target,
                                &lines,
                                hit.start_line,
                                format!("matches `{}`", hit.matched),
                            ));
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then(a.line.cmp(&b.line))
                .then(a.decision.cmp(&b.decision))
        });
        out.dedup_by(|a, b| a.decision == b.decision && a.path == b.path && a.line == b.line);
        out
    }
}

fn violation(r: &Record, t: &Target, lines: &[&str], line: u32, what: String) -> Violation {
    Violation {
        decision: r.id.clone(),
        path: t.path.clone(),
        line,
        what,
        code: lines
            .get(line as usize - 1)
            .map(|l| l.trim().chars().take(160).collect())
            .unwrap_or_default(),
        overridden: ignore_reason(lines, line, &r.id),
    }
}

/// `decisions-ignore <id>: <reason>` on the line, or on the line above when that line is only
/// the comment (a trailing comment on the line above covers that line, not this one).
fn ignore_reason(lines: &[&str], line: u32, id: &str) -> Option<String> {
    let idx = line as usize - 1;
    let above = idx
        .checked_sub(1)
        .filter(|&i| lines.get(i).is_some_and(|l| is_comment_only(l)));
    for i in [above, Some(idx)].into_iter().flatten() {
        let Some(text) = lines.get(i) else { continue };
        for (pos, _) in text.match_indices("decisions-ignore") {
            let rest = text[pos + "decisions-ignore".len()..].trim_start();
            let Some(after) = rest.strip_prefix(id) else {
                continue;
            };
            if after.starts_with(|c: char| c.is_alphanumeric()) {
                continue;
            }
            let reason = after.trim_start().strip_prefix(':').map(|r| {
                r.split("decisions-ignore")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_end_matches(';')
                    .trim_end_matches("*/")
                    .trim_end_matches("-->")
                    .trim()
                    .to_owned()
            });
            if let Some(r) = reason.filter(|r| !r.is_empty()) {
                return Some(r);
            }
        }
    }
    None
}

/// A line holding nothing but a comment.
fn is_comment_only(line: &str) -> bool {
    const MARKERS: &[&str] = &["//", "#", "--", "/*", "<!--", ";", "{/*"];
    let t = line.trim_start();
    MARKERS.iter().any(|m| t.starts_with(m))
}

/// Dependencies the change adds to a manifest (empty for other files or unparsable ones).
fn added_deps(t: &Target) -> Vec<String> {
    if !manifest::is_manifest(&t.path) {
        return Vec::new();
    }
    let Some(after) = manifest::deps(&t.path, &t.content) else {
        return Vec::new();
    };
    let before = t
        .before
        .as_deref()
        .and_then(|b| manifest::deps(&t.path, b))
        .unwrap_or_default();
    after.difference(&before).cloned().collect()
}

/// Import syntax by language family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportSyntax {
    JavaScript,
    Python,
    Go,
    Rust,
}

impl ImportSyntax {
    fn of(lang: lang::Lang) -> Option<Self> {
        use ast_grep_language::SupportLang as L;
        match lang {
            L::TypeScript | L::Tsx | L::JavaScript => Some(Self::JavaScript),
            L::Python => Some(Self::Python),
            L::Go => Some(Self::Go),
            L::Rust => Some(Self::Rust),
            _ => None,
        }
    }

    /// The module an (added) line imports.
    fn module(self, line: &str) -> Option<String> {
        let t = line.trim();
        match self {
            Self::JavaScript => {
                // `import x from 'm'`, `export * from 'm'`, `} from 'm'` (multi-line), `import 'm'`,
                // `require('m')`, `import('m')`.
                let statement = ["import", "export", "}", "from "]
                    .iter()
                    .any(|p| t.starts_with(p));
                if statement
                    && let Some(pos) = t
                        .find(" from ")
                        .or_else(|| t.starts_with("from ").then_some(0))
                {
                    return quoted(&t[pos..]);
                }
                if t.starts_with("import '") || t.starts_with("import \"") {
                    return quoted(t);
                }
                let pos = t.find("require(").or_else(|| t.find("import("))?;
                quoted(&t[pos..])
            }
            Self::Python => {
                if let Some(rest) = t.strip_prefix("from ") {
                    return rest.split_whitespace().next().map(str::to_owned);
                }
                let rest = t.strip_prefix("import ")?;
                rest.split([',', ' '])
                    .next()
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .map(str::to_owned)
            }
            Self::Go => {
                // `import "m"`, `import alias "m"`, or a line inside an import block.
                let body = t.strip_prefix("import ").unwrap_or(t);
                let words: Vec<&str> = body.split_whitespace().collect();
                let looks_like_spec = matches!(words.len(), 1 | 2)
                    && body.ends_with('"')
                    && body.matches('"').count() == 2;
                looks_like_spec.then(|| quoted(body)).flatten()
            }
            Self::Rust => {
                let rest = t
                    .strip_prefix("use ")
                    .or_else(|| t.strip_prefix("pub use "))?;
                // A real `use` item ends the path with `;` or opens a `{` group.
                if !(rest.ends_with(';') || rest.contains('{')) {
                    return None;
                }
                let path = rest
                    .trim_end_matches(';')
                    .split(['{', ' '])
                    .next()?
                    .trim_end_matches("::");
                path.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
                    .then(|| path.to_owned())
            }
        }
    }
}

fn quoted(s: &str) -> Option<String> {
    let start = s.find(['\'', '"', '`'])?;
    let q = s[start..].chars().next()?;
    let rest = &s[start + 1..];
    Some(rest[..rest.find(q)?].to_owned())
}

/// `requests` forbids `requests` and `requests.adapters`; `api/` forbids `../api/client`
/// but not `../rapi/client`. Matching is on whole segments.
fn module_matches(module: &str, forbidden: &str) -> bool {
    let forbidden = forbidden.trim();
    if module == forbidden
        || [".", "/", "::"]
            .iter()
            .any(|sep| module.starts_with(&format!("{forbidden}{sep}")))
    {
        return true;
    }
    let want: Vec<&str> = forbidden
        .trim_matches('/')
        .trim_start_matches("./")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let have: Vec<&str> = module.split('/').collect();
    forbidden.contains('/')
        && !want.is_empty()
        && have.windows(want.len()).any(|w| w == want.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{Checks, PatternCheck, Status};

    fn rec(checks: Checks, scope: &[&str]) -> Record {
        Record {
            id: "D-0007".into(),
            decision: "x".into(),
            scope: scope.iter().map(|s| s.to_string()).collect(),
            status: Status::Accepted,
            checks,
            ..Default::default()
        }
    }

    fn target(path: &str, content: &str, added: Option<Vec<LineRange>>) -> Target {
        Target {
            path: path.into(),
            content: content.into(),
            added,
            before: None,
        }
    }

    #[test]
    fn imports_by_language() {
        use ast_grep_language::SupportLang as L;
        let js = ImportSyntax::of(L::TypeScript).unwrap();
        assert_eq!(
            js.module("import axios from 'axios';").as_deref(),
            Some("axios")
        );
        assert_eq!(js.module("} from \"axios\";").as_deref(), Some("axios"));
        assert_eq!(
            js.module("const x = require('node-fetch')").as_deref(),
            Some("node-fetch")
        );
        assert_eq!(
            js.module("  \"axios\",").as_deref(),
            None,
            "a string in a list is not an import"
        );
        let py = ImportSyntax::of(L::Python).unwrap();
        assert_eq!(
            py.module("from requests.adapters import X").as_deref(),
            Some("requests.adapters")
        );
        assert_eq!(py.module("import os, sys").as_deref(), Some("os"));
        let go = ImportSyntax::of(L::Go).unwrap();
        assert_eq!(go.module("\t\"net/http\"").as_deref(), Some("net/http"));
        assert_eq!(
            go.module("\tpb \"example.com/proto\"").as_deref(),
            Some("example.com/proto")
        );
        let rs = ImportSyntax::of(L::Rust).unwrap();
        assert_eq!(
            rs.module("use reqwest::Client;").as_deref(),
            Some("reqwest::Client")
        );
        assert_eq!(rs.module("use axios only in legacy code"), None);
        assert!(ImportSyntax::of(L::Json).is_none());
        assert!(module_matches("requests.adapters", "requests"));
        assert!(!module_matches("requests_toolbelt", "requests"));
        assert!(module_matches("../api/client", "api/"));
        assert!(!module_matches("../rapi/client", "api/"));
        assert!(!module_matches(
            "@aws-sdk/client-s3-extra",
            "@aws-sdk/client-s3"
        ));
    }

    #[test]
    fn checks_only_added_lines_in_scope() {
        let records = vec![rec(
            Checks {
                forbid_imports: vec!["axios".into()],
                forbid_patterns: vec![PatternCheck {
                    language: "TypeScript".into(),
                    pattern: "fetch($$$A)".into(),
                }],
                ..Default::default()
            },
            &["src/**"],
        )];
        let checker = Checker::new(&records);
        let content = "import axios from 'axios';\nconst a = fetch(u);\nconst b = fetch(v);\n";
        let found = checker.check(&target(
            "src/a.ts",
            content,
            Some(vec![LineRange::new(3, 3)]),
        ));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].line, 3);
        let all = checker.check(&target("src/a.ts", content, None));
        assert_eq!(all.len(), 3);
        assert!(
            checker
                .check(&target("test/a.ts", content, None))
                .is_empty()
        );
    }

    #[test]
    fn deps_apply_to_manifests_regardless_of_scope() {
        let records = vec![rec(
            Checks {
                forbid_deps: vec!["axios".into(), "redis".into()],
                ..Default::default()
            },
            &["src/**"],
        )];
        let checker = Checker::new(&records);
        let pkg =
            "{\n  \"name\": \"redis\",\n  \"dependencies\": {\n    \"axios\": \"^1.0.0\"\n  }\n}\n";
        assert_eq!(
            checker.check(&target("package.json", pkg, None)).len(),
            1,
            "the package name isn't a dependency"
        );
        let mut had = target("package.json", pkg, None);
        had.before = Some(pkg.into());
        assert!(
            checker.check(&had).is_empty(),
            "already a dependency before the change"
        );
        let req = "redis==5.0\nredis-om==1\n";
        assert_eq!(
            checker.check(&target("requirements.txt", req, None)).len(),
            1
        );
        let py = "[project]\ndependencies = [\"redis>=5\"]\n";
        assert_eq!(checker.check(&target("pyproject.toml", py, None)).len(), 1);
    }

    #[test]
    fn overrides_need_a_reason() {
        let records = vec![rec(
            Checks {
                forbid_imports: vec!["axios".into()],
                ..Default::default()
            },
            &[],
        )];
        let checker = Checker::new(&records);
        let with = "// decisions-ignore D-0007: legacy upload path\nimport axios from 'axios';\n";
        let without = "import axios from 'axios'; // decisions-ignore D-0007\n";
        assert_eq!(
            checker.check(&target("a.ts", with, None))[0]
                .overridden
                .as_deref(),
            Some("legacy upload path")
        );
        assert_eq!(
            checker.check(&target("a.ts", without, None))[0].overridden,
            None
        );
    }

    #[test]
    fn trailing_override_on_the_line_above_does_not_cover_a_new_line() {
        let records = vec![rec(
            Checks {
                forbid_imports: vec!["axios".into()],
                ..Default::default()
            },
            &[],
        )];
        let checker = Checker::new(&records);
        let after = "import a from 'axios'; // decisions-ignore D-0007: legacy upload\nimport b from 'axios';\n";
        let v = checker.check(&target("a.ts", after, Some(vec![LineRange::new(2, 2)])));
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].line, 2);
        assert_eq!(v[0].overridden, None);
        for comment in [
            "# decisions-ignore D-0007: ok",
            "  /* decisions-ignore D-0007: ok */",
        ] {
            let src = format!("{comment}\nimport b from 'axios';\n");
            assert_eq!(
                checker.check(&target("a.ts", &src, None))[0]
                    .overridden
                    .as_deref(),
                Some("ok")
            );
        }
    }
}
