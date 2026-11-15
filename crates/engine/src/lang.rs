//! Language detection on top of ast-grep's registry.

use ast_grep_language::{Language, SupportLang};
use std::path::Path;

pub use ast_grep_language::SupportLang as Lang;

/// Language for a file path, from its extension.
pub fn detect(path: &str) -> Option<Lang> {
    SupportLang::from_path(Path::new(path))
}

/// Parse a language name as written in rule files (`TypeScript`, `ts`, `python`, ...).
pub fn parse(name: &str) -> Option<Lang> {
    name.parse().ok()
}

/// Canonical name used in rule files and records.
pub fn name(lang: Lang) -> String {
    lang.to_string()
}

/// Languages whose grammars accept each other's patterns well enough that a rule
/// written for one should also run on the other.
pub fn siblings(lang: Lang) -> &'static [Lang] {
    match lang {
        SupportLang::TypeScript => &[SupportLang::Tsx],
        SupportLang::Tsx => &[SupportLang::TypeScript],
        _ => &[],
    }
}

/// Rule family: languages that share rules and clusters.
pub fn family(lang: Lang) -> Lang {
    match lang {
        SupportLang::Tsx => SupportLang::TypeScript,
        other => other,
    }
}

/// Programming languages worth mining. Data and markup formats are skipped:
/// corrections there are content edits, not code patterns.
pub fn is_code(lang: Lang) -> bool {
    !matches!(
        lang,
        SupportLang::Json
            | SupportLang::Yaml
            | SupportLang::Html
            | SupportLang::Css
            | SupportLang::Hcl
            | SupportLang::Nix
            | SupportLang::Markdown
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_languages() {
        assert_eq!(detect("src/a.ts"), Some(SupportLang::TypeScript));
        assert_eq!(detect("src/a.tsx"), Some(SupportLang::Tsx));
        assert_eq!(detect("x.py"), Some(SupportLang::Python));
        assert_eq!(detect("README"), None);
        assert_eq!(family(SupportLang::Tsx), SupportLang::TypeScript);
        assert!(!is_code(SupportLang::Json));
        assert!(is_code(SupportLang::Python));
        assert_eq!(
            parse(&name(SupportLang::TypeScript)),
            Some(SupportLang::TypeScript)
        );
    }
}
