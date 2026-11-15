//! Which decisions apply to a file.
//!
//! A scope entry is either a path glob (`src/billing/**`, `*.py`, `api.ts`) or a symbol
//! or module name (`apiClient`, `ClaudeAgentOptions`). Globs match the path. A symbol matches
//! a file whose path names it or whose content mentions it. An empty scope or `**` is repo-wide.

use crate::record::Record;
use globset::{Glob, GlobMatcher};

#[derive(Debug, Clone)]
enum Entry {
    Glob(GlobMatcher),
    Symbol(String),
    Everywhere,
}

/// How a decision matched; more specific matches are shown first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Match {
    Glob,
    Symbol,
    RepoWide,
}

#[derive(Debug, Clone)]
pub struct Scope {
    entries: Vec<Entry>,
}

impl Scope {
    pub fn of(record: &Record) -> Self {
        let mut entries: Vec<Entry> = record.scope.iter().filter_map(|s| parse(s)).collect();
        if entries.is_empty() {
            entries.push(Entry::Everywhere);
        }
        Self { entries }
    }

    /// Symbol entries, as matched in code (`Query.commands` → `commands`).
    pub fn symbols(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().filter_map(|e| match e {
            Entry::Symbol(s) => Some(s.as_str()),
            _ => None,
        })
    }

    pub fn is_repo_wide(&self) -> bool {
        self.entries.iter().any(|e| matches!(e, Entry::Everywhere))
    }

    /// The best match of this scope for a path (and, if known, its content).
    pub fn matches(&self, path: &str, content: Option<&str>) -> Option<Match> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                Entry::Everywhere => Some(Match::RepoWide),
                Entry::Glob(g) => g.is_match(path).then_some(Match::Glob),
                Entry::Symbol(s) => symbol_matches(s, path, content).then_some(Match::Symbol),
            })
            .min()
    }
}

fn parse(raw: &str) -> Option<Entry> {
    let s = raw.trim().trim_start_matches("./");
    if s.is_empty() {
        return None;
    }
    if s == "**" || s == "*" || s == "**/*" {
        return Some(Entry::Everywhere);
    }
    let looks_like_path = s.contains('/') || s.contains('*') || has_file_extension(s);
    if looks_like_path {
        // `src/billing` without a glob means everything below it, too.
        let pattern = if !s.contains('*') && !has_file_extension(s) {
            format!("{}/**", s.trim_end_matches('/'))
        } else if !s.contains('/') && !s.starts_with("**") {
            format!("**/{s}")
        } else {
            s.to_owned()
        };
        return Glob::new(&pattern)
            .ok()
            .map(|g| Entry::Glob(g.compile_matcher()));
    }
    // `Query.commands`, `module::item`: the last segment is what shows up in code.
    let symbol = s.rsplit(['.', ':']).find(|p| !p.is_empty()).unwrap_or(s);
    Some(Entry::Symbol(symbol.trim_end_matches("()").to_owned()))
}

fn has_file_extension(s: &str) -> bool {
    const EXTS: &[&str] = &[
        "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "rs", "go", "java", "kt", "rb", "php", "cs",
        "swift", "c", "h", "cpp", "hpp", "md", "json", "yml", "yaml", "toml", "sql", "css", "html",
        "vue", "svelte",
    ];
    s.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && EXTS.contains(&ext))
}

fn symbol_matches(symbol: &str, path: &str, content: Option<&str>) -> bool {
    let lower = symbol.to_lowercase();
    let in_path = path.split('/').any(|part| {
        part.split('.')
            .next()
            .is_some_and(|stem| stem.to_lowercase() == lower)
    });
    in_path || content.is_some_and(|c| contains_word(c, symbol))
}

fn contains_word(text: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// Live decisions that apply to `path`, most specific first.
pub fn applicable<'a>(
    records: &'a [Record],
    path: &str,
    content: Option<&str>,
) -> Vec<(&'a Record, Match)> {
    let mut out: Vec<(&Record, Match)> = records
        .iter()
        .filter(|r| r.is_live())
        .filter_map(|r| Scope::of(r).matches(path, content).map(|m| (r, m)))
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.id.cmp(&b.0.id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::Status;

    fn rec(id: &str, scope: &[&str]) -> Record {
        Record {
            id: id.into(),
            decision: "x".into(),
            scope: scope.iter().map(|s| s.to_string()).collect(),
            status: Status::Accepted,
            ..Default::default()
        }
    }

    #[test]
    fn globs_symbols_and_repo_wide() {
        let records = vec![
            rec("D-1", &["src/billing/**"]),
            rec("D-2", &["apiClient"]),
            rec("D-3", &[]),
            rec("D-4", &["src/claude_agent_sdk/types.py"]),
            rec("D-5", &["src/claude_agent_sdk"]),
            rec("D-6", &["*.py"]),
            rec("D-7", &["Query.commands"]),
        ];
        let ids = |path: &str, content: Option<&str>| -> Vec<String> {
            applicable(&records, path, content)
                .iter()
                .map(|(r, _)| r.id.clone())
                .collect()
        };
        assert_eq!(ids("src/billing/charge.ts", None), vec!["D-1", "D-3"]);
        assert_eq!(
            ids("src/ui/x.ts", Some("await apiClient.get(u)")),
            vec!["D-2", "D-3"]
        );
        assert_eq!(ids("src/ui/x.ts", Some("myApiClientX()")), vec!["D-3"]);
        assert_eq!(ids("src/lib/apiClient.ts", None), vec!["D-2", "D-3"]);
        assert_eq!(
            ids("src/claude_agent_sdk/types.py", None),
            vec!["D-4", "D-5", "D-6", "D-3"]
        );
        assert_eq!(
            ids("a/q.py", Some("self.commands = []")),
            vec!["D-6", "D-7", "D-3"]
        );
        assert!(Scope::of(&records[2]).is_repo_wide());
    }

    #[test]
    fn superseded_decisions_do_not_apply() {
        let mut r = rec("D-1", &["**"]);
        r.status = Status::Superseded;
        assert!(applicable(&[r], "a.ts", None).is_empty());
    }
}
