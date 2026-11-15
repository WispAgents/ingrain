//! The ingrain block in AGENTS.md, for agents that read AGENTS.md but run no hooks (Codex, Cursor,
//! Copilot, …). It lives between markers, so `ingrain init --agents-md` can update it in place.

use anyhow::Result;
use std::path::Path;

use ingrain_ledger::extract::source::{AGENTS_BLOCK_END as END, AGENTS_BLOCK_START as START};

const BODY: &str = r#"## Decisions and learned rules (ingrain)

This repo keeps rules agents must follow, managed by [ingrain](https://github.com/WispAgents/ingrain):
accepted decisions in `decisions/` and lint rules learned from past corrections in `.corrections/rules/`.

- Before editing a file, run `ingrain decisions for <path>` and follow what it lists.
- Before finishing, run `ingrain check` and fix what it reports. If a flagged line is right as written,
  keep it and add `decisions-ignore <id>: <reason>` or `ingrain-ignore <rule>: <reason>` on that line.
- Never edit `decisions/*.yml` or `.corrections/rules/*.yml`. When the user states a lasting decision,
  propose it: `ingrain decisions add --propose "<decision>" --reason "<why>" --scope <glob>`.
  To change one: `ingrain decisions supersede <id> --decision "<new>" --reason "<why>"`. A person accepts.
"#;

/// The full marked block.
pub fn block() -> String {
    format!("{START}\n{BODY}{END}\n")
}

/// Insert or refresh the block in `path`. Returns true if the file changed.
pub fn write(path: &Path) -> Result<bool> {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    let next = match (current.find(START), current.find(END)) {
        (Some(a), Some(b)) if a < b => format!(
            "{}{}{}",
            &current[..a],
            block().trim_end(),
            &current[b + END.len()..]
        ),
        _ if current.trim().is_empty() => format!("# AGENTS.md\n\n{}", block()),
        _ => format!("{}\n\n{}", current.trim_end(), block()),
    };
    if next == current {
        return Ok(false);
    }
    std::fs::write(path, next)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_once_and_refreshes_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "# Project\n\nUse tabs.\n").unwrap();
        assert!(write(&path).unwrap());
        assert!(!write(&path).unwrap(), "idempotent");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Project\n\nUse tabs.\n\n<!-- ingrain:start -->"));
        assert_eq!(text.matches(START).count(), 1);
        // An outdated block is replaced, the rest kept.
        std::fs::write(&path, format!("{START}\nold\n{END}\n\nmore\n")).unwrap();
        assert!(write(&path).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("ingrain decisions for")
                && text.ends_with("\n\nmore\n")
                && !text.contains("old")
        );
        // A missing file gets a fresh one.
        let fresh = dir.path().join("new/AGENTS.md");
        std::fs::create_dir_all(fresh.parent().unwrap()).unwrap();
        assert!(write(&fresh).unwrap());
    }
}
