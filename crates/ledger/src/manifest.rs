//! Dependency names declared by package manifests, so "adds dependency X" compares the
//! parsed set before and after a change instead of guessing from lines.

use serde_json::Value as Json;
use std::collections::BTreeSet;

/// Whether `path` is a manifest this module understands.
pub fn is_manifest(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "package.json" | "pyproject.toml" | "Cargo.toml" | "go.mod"
    ) || (name.ends_with(".txt")
        && (name.starts_with("requirements") || path.contains("requirements/")))
}

/// Normalized dependency names in a manifest. `None` if it doesn't parse.
pub fn deps(path: &str, text: &str) -> Option<BTreeSet<String>> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut out = BTreeSet::new();
    match name {
        "package.json" => {
            let v: Json = serde_json::from_str(text).ok()?;
            for key in [
                "dependencies",
                "devDependencies",
                "peerDependencies",
                "optionalDependencies",
            ] {
                if let Some(map) = v[key].as_object() {
                    out.extend(map.keys().map(|k| k.to_lowercase()));
                }
            }
        }
        "Cargo.toml" => {
            let v: toml::Value = toml::from_str(text).ok()?;
            let mut tables = vec![&v];
            if let Some(targets) = v.get("target").and_then(|t| t.as_table()) {
                tables.extend(targets.values());
            }
            for t in tables {
                for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(map) = t.get(key).and_then(|d| d.as_table()) {
                        out.extend(map.keys().map(|k| k.to_lowercase()));
                    }
                }
            }
        }
        "pyproject.toml" => {
            let v: toml::Value = toml::from_str(text).ok()?;
            let mut specs: Vec<&str> = Vec::new();
            specs.extend(strings(
                v.get("project").and_then(|p| p.get("dependencies")),
            ));
            for table in [
                v.get("project")
                    .and_then(|p| p.get("optional-dependencies")),
                v.get("dependency-groups"),
            ] {
                if let Some(groups) = table.and_then(|g| g.as_table()) {
                    for g in groups.values() {
                        specs.extend(strings(Some(g)));
                    }
                }
            }
            out.extend(specs.iter().filter_map(|s| python_name(s)));
            if let Some(poetry) = v.get("tool").and_then(|t| t.get("poetry")) {
                let mut tables: Vec<&toml::Value> = ["dependencies", "dev-dependencies"]
                    .iter()
                    .filter_map(|k| poetry.get(*k))
                    .collect();
                if let Some(groups) = poetry.get("group").and_then(|g| g.as_table()) {
                    tables.extend(groups.values().filter_map(|g| g.get("dependencies")));
                }
                for t in tables.into_iter().filter_map(|t| t.as_table()) {
                    out.extend(
                        t.keys()
                            .filter(|k| *k != "python")
                            .map(|k| normalize_python(k)),
                    );
                }
            }
        }
        "go.mod" => {
            let mut in_block = false;
            for line in text.lines() {
                let t = line.trim();
                if t.starts_with("require (") {
                    in_block = true;
                } else if in_block && t == ")" {
                    in_block = false;
                } else if let Some(rest) = t.strip_prefix("require ").or(in_block.then_some(t))
                    && let Some(module) = rest
                        .split_whitespace()
                        .next()
                        .filter(|m| !m.starts_with("//"))
                {
                    out.insert(module.to_lowercase());
                }
            }
        }
        _ => {
            for line in text.lines() {
                let t = line.trim();
                if t.is_empty() || t.starts_with('#') || t.starts_with('-') {
                    continue;
                }
                out.extend(python_name(t));
            }
        }
    }
    Some(out)
}

fn strings(a: Option<&toml::Value>) -> Vec<&str> {
    a.and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default()
}

/// The package name of a PEP 508 requirement (`redis[hiredis]>=5; python_version>'3.8'` → `redis`).
fn python_name(spec: &str) -> Option<String> {
    let end = spec
        .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.'))
        .unwrap_or(spec.len());
    let name = &spec[..end];
    (!name.is_empty()).then(|| normalize_python(name))
}

/// PEP 503: case-insensitive, runs of `-`, `_`, `.` are equivalent.
fn normalize_python(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        let c = if matches!(c, '-' | '_' | '.') { '-' } else { c };
        if !(c == '-' && out.ends_with('-')) {
            out.push(c);
        }
    }
    out
}

/// Whether a forbidden name matches a declared one (exact, after the manifest's normalization).
pub fn same(path: &str, declared: &str, forbidden: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let python = name == "pyproject.toml" || name.ends_with(".txt");
    if python {
        declared == normalize_python(forbidden)
    } else {
        declared == forbidden.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(path: &str, text: &str) -> Vec<String> {
        deps(path, text).unwrap().into_iter().collect()
    }

    #[test]
    fn parses_manifests() {
        assert_eq!(
            set(
                "package.json",
                r#"{"name":"redis","dependencies":{"axios":"1"},"devDependencies":{"vitest":"1"}}"#
            ),
            vec!["axios", "vitest"]
        );
        assert_eq!(
            set(
                "pyproject.toml",
                "[project]\ndependencies = [\"Redis[hiredis]>=5\", \"typing_extensions\"]\n"
            ),
            vec!["redis", "typing-extensions"]
        );
        assert_eq!(
            set(
                "Cargo.toml",
                "[dependencies]\nserde = \"1\"\n[dependencies.redis]\nversion = \"0.25\"\n[target.'cfg(unix)'.dependencies]\nnix = \"0.29\"\n"
            ),
            vec!["nix", "redis", "serde"]
        );
        assert_eq!(
            set(
                "go.mod",
                "module x\n\nrequire (\n\tgithub.com/a/b v1.0.0\n\tgithub.com/a/bc v1.0.0 // indirect\n)\nrequire golang.org/x/net v0.1.0\n"
            ),
            vec!["github.com/a/b", "github.com/a/bc", "golang.org/x/net"]
        );
        assert_eq!(
            set(
                "requirements/dev.txt",
                "-r base.txt\nredis==5\n# comment\nrequests>=2\n"
            ),
            vec!["redis", "requests"]
        );
        assert!(
            is_manifest("requirements/dev.txt")
                && is_manifest("requirements.txt")
                && !is_manifest("notes.txt")
        );
        assert!(deps("package.json", "{ broken").is_none());
        assert!(same(
            "pyproject.toml",
            "typing-extensions",
            "Typing_Extensions"
        ));
        assert!(!same("go.mod", "github.com/a/bc", "github.com/a/b"));
    }
}
