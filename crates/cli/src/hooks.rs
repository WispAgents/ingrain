//! Installing ingrain into Claude Code settings and git hooks.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::Path;

/// `.claude/settings.json` is shared with the team: teammates without ingrain get a no-op.
fn guarded(event: &str) -> String {
    format!("command -v ingrain >/dev/null 2>&1 && ingrain hook {event} || true")
}

/// The Claude Code hook events ingrain answers, with their tool matcher.
const EVENTS: &[(&str, Option<&str>, &str)] = &[
    ("SessionStart", None, "session-start"),
    ("PreToolUse", Some("Edit|Write|MultiEdit"), "pre-tool-use"),
    ("PostToolUse", Some("Edit|Write|MultiEdit"), "post-tool-use"),
    ("Stop", None, "stop"),
];

/// Hook commands from v0.1 ingrain and the separate decision-ledger tool, replaced on install.
/// Only these exact forms match, so a user's own hooks are never touched.
fn is_legacy(command: &str) -> bool {
    const LEGACY: &[&str] = &[
        "ingrain check --hook post-edit",
        "ingrain check --hook stop",
        "decisions hook session-start",
        "decisions hook pre-tool-use",
        "decisions hook stop",
    ];
    LEGACY.iter().any(|legacy| {
        command.match_indices(legacy).any(|(i, _)| {
            let before = command[..i].chars().next_back();
            let after = command[i + legacy.len()..].chars().next();
            let boundary = |c: Option<char>| {
                c.is_none_or(|c| {
                    !(c.is_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.')
                })
            };
            boundary(before) && boundary(after)
        })
    })
}

/// True when Claude Code settings run an ingrain hook, current (`ingrain hook …`) or legacy.
pub fn runs_ingrain(settings: &Value) -> bool {
    let Some(events) = settings["hooks"].as_object() else {
        return false;
    };
    events
        .values()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|group| group["hooks"].as_array())
        .flatten()
        .filter_map(|h| h["command"].as_str())
        .any(|c| c.contains("ingrain hook ") || is_legacy(c))
}

const PRE_COMMIT_MARKER: &str = "# ingrain";

/// What an install step did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Installed,
    AlreadyPresent,
    /// Left alone; the string says what the user should do.
    Skipped(String),
}

/// Merge ingrain's hooks into `.claude/settings.json`, keeping everything else.
pub fn install_claude(root: &Path) -> Result<Outcome> {
    let path = root.join(".claude").join("settings.json");
    let mut settings: Value = if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON", path.display()))?
    } else {
        json!({})
    };
    let Some(obj) = settings.as_object_mut() else {
        return Ok(Outcome::Skipped(format!(
            "{} is not a JSON object",
            path.display()
        )));
    };
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let mut changed = remove_legacy(hooks)?;
    for (event, matcher, name) in EVENTS {
        changed |= add_hook(hooks, event, *matcher, &guarded(name))?;
    }
    if !changed {
        return Ok(Outcome::AlreadyPresent);
    }
    std::fs::create_dir_all(path.parent().expect("has parent"))?;
    std::fs::write(&path, serde_json::to_string_pretty(&settings)? + "\n")?;
    Ok(Outcome::Installed)
}

/// Remove legacy commands, and the groups that held only them. Groups that were already empty
/// are left alone.
fn remove_legacy(hooks: &mut Value) -> Result<bool> {
    let Some(events) = hooks.as_object_mut() else {
        return Ok(false);
    };
    let mut changed = false;
    for list in events.values_mut().filter_map(Value::as_array_mut) {
        let mut emptied = Vec::new();
        for (i, group) in list.iter_mut().enumerate() {
            if let Some(hs) = group["hooks"].as_array_mut() {
                let before = hs.len();
                hs.retain(|h| !h["command"].as_str().is_some_and(is_legacy));
                if hs.len() != before {
                    changed = true;
                    if hs.is_empty() {
                        emptied.push(i);
                    }
                }
            }
        }
        for i in emptied.into_iter().rev() {
            list.remove(i);
        }
    }
    Ok(changed)
}

fn add_hook(hooks: &mut Value, event: &str, matcher: Option<&str>, command: &str) -> Result<bool> {
    let hooks = hooks
        .as_object_mut()
        .context("`hooks` in settings.json is not an object")?;
    let list = hooks.entry(event).or_insert_with(|| json!([]));
    let list = list
        .as_array_mut()
        .with_context(|| format!("`hooks.{event}` is not an array"))?;
    let present = list.iter().any(|group| {
        group["hooks"]
            .as_array()
            .is_some_and(|hs| hs.iter().any(|h| h["command"] == command))
    });
    if present {
        return Ok(false);
    }
    let mut group = json!({ "hooks": [{ "type": "command", "command": command }] });
    if let Some(m) = matcher {
        group["matcher"] = json!(m);
    }
    list.push(group);
    Ok(true)
}

/// Write `.git/hooks/pre-commit`, unless another pre-commit hook already exists.
pub fn install_pre_commit(git_dir: &Path) -> Result<Outcome> {
    let path = git_dir.join("hooks").join("pre-commit");
    if path.exists() {
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if existing.contains(PRE_COMMIT_MARKER) {
            return Ok(Outcome::AlreadyPresent);
        }
        return Ok(Outcome::Skipped(format!(
            "{} already exists; add `ingrain check --staged` to it",
            path.display()
        )));
    }
    std::fs::create_dir_all(path.parent().expect("has parent"))?;
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n{PRE_COMMIT_MARKER}: block commits that break learned rules\n\
             # Teammates without ingrain installed are not blocked.\n\
             command -v ingrain >/dev/null 2>&1 || exit 0\n\
             exec ingrain check --staged\n"
        ),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(Outcome::Installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_into_existing_settings() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}]}}"#,
        )
        .unwrap();
        assert_eq!(install_claude(dir.path()).unwrap(), Outcome::Installed);
        assert_eq!(install_claude(dir.path()).unwrap(), Outcome::AlreadyPresent);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v["model"], "x");
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(
            v["hooks"]["PostToolUse"][0]["matcher"],
            "Edit|Write|MultiEdit"
        );
    }

    #[test]
    fn replaces_legacy_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join(".claude/settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"hooks":{"PostToolUse":[{"matcher":"Edit","hooks":[{"type":"command","command":"command -v ingrain >/dev/null 2>&1 && ingrain check --hook post-edit || true"}]}],
               "PreToolUse":[{"hooks":[{"type":"command","command":"command -v decisions >/dev/null 2>&1 && decisions hook pre-tool-use || true"}]}]}}"#,
        )
        .unwrap();
        assert_eq!(install_claude(dir.path()).unwrap(), Outcome::Installed);
        let text = std::fs::read_to_string(&settings).unwrap();
        assert!(
            !text.contains("check --hook") && !text.contains("decisions hook"),
            "{text}"
        );
        let v: Value = serde_json::from_str(&text).unwrap();
        for event in ["SessionStart", "PreToolUse", "PostToolUse", "Stop"] {
            assert_eq!(v["hooks"][event].as_array().unwrap().len(), 1, "{event}");
        }
    }

    #[test]
    fn legacy_match_is_exact() {
        assert!(is_legacy(
            "command -v ingrain >/dev/null 2>&1 && ingrain check --hook post-edit || true"
        ));
        assert!(is_legacy("decisions hook stop"));
        assert!(!is_legacy("./scripts/sync-decisions hook pre-push"));
        assert!(!is_legacy("my-decisions hook stop"));
        assert!(!is_legacy("ingrain hook stop"));
    }

    #[test]
    fn pre_commit_does_not_clobber() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(install_pre_commit(dir.path()).unwrap(), Outcome::Installed);
        assert_eq!(
            install_pre_commit(dir.path()).unwrap(),
            Outcome::AlreadyPresent
        );
        std::fs::write(dir.path().join("hooks/pre-commit"), "#!/bin/sh\nnpm test\n").unwrap();
        assert!(matches!(
            install_pre_commit(dir.path()).unwrap(),
            Outcome::Skipped(_)
        ));
    }
}
