//! Which decisions apply to a file.
//!
//! A scope entry is either a path glob (`src/billing/**`, `*.py`, `api.ts`, `Makefile`, `.env`)
//! or a symbol or module name (`apiClient`, `ClaudeAgentOptions`). Globs match the path; `*`
//! stays within one directory and `**` crosses them. A bare file name matches that file in any
//! directory. A symbol matches a file whose path names it or whose content mentions it. An empty
//! scope or `**` is repo-wide.

use crate::record::Record;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

#[derive(Debug, Clone)]
enum Entry {
    Glob(GlobSet),
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
    if is_file_name(s) {
        // `Makefile`, `.env`, `Gemfile.lock`: that name in any directory (or, for `.github`,
        // everything under a directory of that name; for `README`, `README.md` too).
        let mut patterns = vec![format!("**/{s}")];
        if s.starts_with('.') {
            patterns.push(format!("**/{s}/**"));
        } else if !s.contains('.') {
            patterns.push(format!("**/{s}.*"));
        }
        return glob_set(&patterns).map(Entry::Glob);
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
        return glob_set(&[pattern]).map(Entry::Glob);
    }
    // `Query.commands`, `module::item`: the last segment is what shows up in code.
    let symbol = s.rsplit(['.', ':']).find(|p| !p.is_empty()).unwrap_or(s);
    Some(Entry::Symbol(symbol.trim_end_matches("()").to_owned()))
}

/// `*` stays within one path segment; `**` crosses directories.
fn glob_set(patterns: &[String]) -> Option<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for p in patterns {
        set.add(GlobBuilder::new(p).literal_separator(true).build().ok()?);
    }
    set.build().ok()
}

/// A bare file (or dot-directory) name: `.env`, `.github`, `Makefile`, `Dockerfile.dev`,
/// `Gemfile.lock`. Dotted names whose last part isn't a file extension (`Query.commands`,
/// `payments.client`) stay symbols / module names.
fn is_file_name(s: &str) -> bool {
    const NAMES: &[&str] = &[
        "Makefile",
        "GNUmakefile",
        "Dockerfile",
        "Containerfile",
        "README",
        "LICENSE",
        "CHANGELOG",
        "CODEOWNERS",
        "Gemfile",
        "Rakefile",
        "Procfile",
        "Justfile",
        "justfile",
        "Jenkinsfile",
        "Vagrantfile",
        "Brewfile",
        "Podfile",
        "BUILD",
        "WORKSPACE",
    ];
    const DOTTED: &[&str] = &[
        "Cargo.lock",
        "yarn.lock",
        "poetry.lock",
        "composer.lock",
        "Pipfile.lock",
        "flake.lock",
        "uv.lock",
        "mix.lock",
        "pdm.lock",
        "bun.lockb",
        "go.mod",
        "go.sum",
        "go.work",
    ];
    if s.contains('/') || s.contains('*') {
        return false;
    }
    let first = s.split('.').next().unwrap_or(s);
    s.starts_with('.') || NAMES.contains(&first) || DOTTED.contains(&s) || has_file_extension(s)
}

fn has_file_extension(s: &str) -> bool {
    const EXTS: &[&str] = &[
        "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "rs", "go", "java", "kt", "rb", "php", "cs",
        "swift", "c", "h", "cpp", "hpp", "md", "json", "yml", "yaml", "toml", "sql", "css", "html",
        "vue", "svelte", "ini", "cfg", "conf", "txt", "xml", "sh", "bash", "ps1", "gradle",
        "proto", "graphql", "tf", "scss", "less", "lua", "dart", "scala", "ex", "exs", "zig",
        "nix", "gemspec", "csv", "rst",
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
    fn file_names_are_paths_not_symbols() {
        let js = Some("const k = process.env.KEY; // see README, Makefile, Dockerfile");
        for (entry, hits, misses) in [
            (".env", &[".env", "sub/.env"][..], &["src/a.ts"][..]),
            (".github", &[".github/workflows/ci.yml"], &["src/a.ts"]),
            ("Makefile", &["Makefile", "sub/Makefile"], &["src/a.ts"]),
            (
                "Dockerfile",
                &["Dockerfile", "ops/Dockerfile.dev"],
                &["src/a.ts"],
            ),
            (
                "README",
                &["README", "README.md", "docs/README.md"],
                &["src/a.ts"],
            ),
            (
                "Gemfile.lock",
                &["Gemfile.lock", "app/Gemfile.lock"],
                &["Gemfile", "a.rb"],
            ),
            ("Cargo.lock", &["Cargo.lock"], &["src/a.ts"]),
        ] {
            let s = Scope::of(&rec("D-1", &[entry]));
            assert_eq!(s.symbols().count(), 0, "{entry} is a path");
            for p in hits {
                assert_eq!(s.matches(p, js), Some(Match::Glob), "{entry} ~ {p}");
            }
            for p in misses {
                assert_eq!(s.matches(p, js), None, "{entry} !~ {p}");
            }
        }
        // Dotted module names are still symbols.
        let s = Scope::of(&rec("D-2", &["payments.client"]));
        assert_eq!(s.symbols().collect::<Vec<_>>(), vec!["client"]);
    }

    #[test]
    fn star_stays_in_one_directory() {
        let s = Scope::of(&rec("D-1", &["src/billing/*.ts"]));
        assert!(s.matches("src/billing/a.ts", None).is_some());
        assert!(s.matches("src/billing/sub/a.ts", None).is_none());
        let s = Scope::of(&rec("D-2", &["src/**/*.ts"]));
        assert!(s.matches("src/a.ts", None).is_some());
        assert!(s.matches("src/billing/sub/a.ts", None).is_some());
        let s = Scope::of(&rec("D-3", &["src/billing"]));
        assert!(s.matches("src/billing/sub/deep/a.ts", None).is_some());
        let s = Scope::of(&rec("D-4", &["*.py"]));
        assert!(s.matches("a/b/c.py", None).is_some());
    }

    #[test]
    fn superseded_decisions_do_not_apply() {
        let mut r = rec("D-1", &["**"]);
        r.status = Status::Superseded;
        assert!(applicable(&[r], "a.ts", None).is_empty());
    }
}
