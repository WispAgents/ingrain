//! Compile ast-grep rule YAML and scan source text with it.

use crate::lang::{self, Lang};
use anyhow::{Context, Result, anyhow, bail};
use ast_grep_config::{GlobalRules, RuleConfig, from_yaml_string};
use ast_grep_core::AstGrep;
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde_yaml::{Mapping, Value};

/// One rule match. Lines are 1-based and inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub rule: String,
    pub start_line: u32,
    pub end_line: u32,
    pub message: String,
    /// First line of the matched code, trimmed.
    pub matched: String,
}

/// A parsed source file, reusable across rules.
pub struct Parsed {
    lang: Lang,
    grep: AstGrep<StrDoc<Lang>>,
}

impl Parsed {
    pub fn new(lang: Lang, source: &str) -> Self {
        Self {
            lang,
            grep: lang.ast_grep(source),
        }
    }
    pub fn lang(&self) -> Lang {
        self.lang
    }
    pub(crate) fn grep(&self) -> &AstGrep<StrDoc<Lang>> {
        &self.grep
    }
}

/// A rule compiled for its own language and any sibling languages
/// (a TypeScript rule also runs on `.tsx` files).
pub struct CompiledRule {
    pub id: String,
    pub lang: Lang,
    variants: Vec<(Lang, RuleConfig<Lang>)>,
    files: Option<GlobSet>,
    ignores: Option<GlobSet>,
}

impl CompiledRule {
    /// Compile one rule document. Errors explain what ast-grep rejected,
    /// which is fed back to the LLM during synthesis.
    pub fn compile(yaml: &str) -> Result<Self> {
        let doc: Mapping = serde_yaml::from_str(yaml).context("rule is not a YAML mapping")?;
        let lang_name = doc
            .get("language")
            .and_then(Value::as_str)
            .context("rule has no `language`")?;
        let lang =
            lang::parse(lang_name).with_context(|| format!("unknown language `{lang_name}`"))?;
        let id = doc
            .get("id")
            .and_then(Value::as_str)
            .context("rule has no `id`")?
            .to_owned();

        let mut variants = vec![(lang, compile_one(yaml)?)];
        for &sibling in lang::siblings(lang) {
            let mut twin = doc.clone();
            twin.insert(Value::from("language"), Value::from(lang::name(sibling)));
            // A pattern that doesn't parse in the sibling grammar just doesn't run there.
            if let Ok(config) = compile_one(&serde_yaml::to_string(&twin)?) {
                variants.push((sibling, config));
            }
        }

        Ok(Self {
            id,
            lang,
            variants,
            files: globs(&doc, "files")?,
            ignores: globs(&doc, "ignores")?,
        })
    }

    /// Whether the rule's language and `files` / `ignores` globs cover this path.
    pub fn applies_to(&self, path: &str) -> bool {
        let Some(file_lang) = lang::detect(path) else {
            return false;
        };
        if !self.variants.iter().any(|(l, _)| *l == file_lang) {
            return false;
        }
        if let Some(files) = &self.files
            && !files.is_match(path)
        {
            return false;
        }
        !self.ignores.as_ref().is_some_and(|i| i.is_match(path))
    }

    /// Scan already-parsed source. Returns nothing if the rule has no variant for its language.
    pub fn scan(&self, parsed: &Parsed) -> Vec<Hit> {
        let Some((_, config)) = self.variants.iter().find(|(l, _)| *l == parsed.lang()) else {
            return Vec::new();
        };
        parsed
            .grep()
            .root()
            .find_all(&config.matcher)
            .map(|m| {
                let text = m.text();
                let first = text.lines().next().unwrap_or("").trim();
                Hit {
                    rule: self.id.clone(),
                    start_line: m.start_pos().line() as u32 + 1,
                    end_line: m.end_pos().line() as u32 + 1,
                    message: config.get_message(&m),
                    matched: truncate(first, 120),
                }
            })
            .collect()
    }

    /// Convenience: parse and scan.
    pub fn scan_text(&self, lang: Lang, source: &str) -> Vec<Hit> {
        self.scan(&Parsed::new(lang, source))
    }
}

fn compile_one(yaml: &str) -> Result<RuleConfig<Lang>> {
    let mut configs: Vec<RuleConfig<Lang>> =
        from_yaml_string(yaml, &GlobalRules::default()).map_err(|e| anyhow!("{e}"))?;
    match configs.len() {
        1 => Ok(configs.remove(0)),
        0 => bail!("no rule in document"),
        n => bail!("expected one rule per file, found {n}"),
    }
}

fn globs(doc: &Mapping, key: &str) -> Result<Option<GlobSet>> {
    let Some(list) = doc.get(key).and_then(Value::as_sequence) else {
        return Ok(None);
    };
    let mut builder = GlobSetBuilder::new();
    for item in list {
        let pattern = item
            .as_str()
            .with_context(|| format!("`{key}` entries must be strings"))?;
        builder.add(Glob::new(pattern).with_context(|| format!("bad glob `{pattern}`"))?);
    }
    Ok(Some(builder.build()?))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast_grep_language::SupportLang;

    const RULE: &str = r#"
id: no-raw-fetch
language: TypeScript
rule:
  pattern: fetch($$$ARGS)
message: Use apiClient instead of fetch
files: ["src/**"]
ignores: ["src/lib/api.ts"]
"#;

    #[test]
    fn scans_and_reports_lines() {
        let rule = CompiledRule::compile(RULE).unwrap();
        let src = "const a = 1;\nconst r = await fetch(`/x`, { method: 'GET' });\n";
        let hits = rule.scan_text(SupportLang::TypeScript, src);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].start_line, 2);
        assert_eq!(hits[0].message, "Use apiClient instead of fetch");
    }

    #[test]
    fn runs_on_sibling_language_and_respects_globs() {
        let rule = CompiledRule::compile(RULE).unwrap();
        assert!(rule.applies_to("src/a.ts"));
        assert!(rule.applies_to("src/view.tsx"));
        assert!(!rule.applies_to("src/lib/api.ts"));
        assert!(!rule.applies_to("test/a.ts"));
        assert!(!rule.applies_to("src/a.py"));
        let hits = rule.scan_text(SupportLang::Tsx, "const x = <div>{fetch(u)}</div>;");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn rejects_invalid_rules() {
        assert!(CompiledRule::compile("id: x\nlanguage: TypeScript\n").is_err());
        assert!(CompiledRule::compile("id: x\nlanguage: Klingon\nrule: {pattern: a}").is_err());
    }
}
