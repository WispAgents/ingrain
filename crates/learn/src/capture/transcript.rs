//! Mine corrections from Claude Code session transcripts (`~/.claude/projects/*/*.jsonl`).
//!
//! The signal: the agent edits a file, the human replies in chat, and the agent's
//! next edit to that file rewrites what it just wrote. The rewrite is the correction
//! and the human's message is its note.

use super::snippet;
use anyhow::Result;
use ingrain_core::{Correction, Source};
use ingrain_engine::lang;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Session files for a repository: Claude Code stores them under a directory
/// named after the working directory with `/` and `.` replaced by `-`.
pub fn session_files(projects_dir: &Path, repo_root: &Path) -> Vec<PathBuf> {
    let key = encode(repo_root);
    let Ok(entries) = std::fs::read_dir(projects_dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != key && !name.starts_with(&format!("{key}-")) {
            continue;
        }
        if let Ok(inner) = std::fs::read_dir(entry.path()) {
            files.extend(
                inner
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "jsonl")),
            );
        }
    }
    files.sort();
    files
}

fn encode(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| {
            if c == '/' || c == '.' || c == '\\' || c == ':' {
                '-'
            } else {
                c
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
enum Event {
    /// The agent wrote `new` into `path`, replacing `old` (empty for a full write).
    Edit {
        path: String,
        old: String,
        new: String,
        time: i64,
    },
    /// A message the human typed.
    Human(String),
}

/// Corrections in one session file. Paths are made relative to `repo_root`.
pub fn mine_file(path: &Path, repo_root: &Path) -> Result<Vec<Correction>> {
    let text = std::fs::read_to_string(path)?;
    let session = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let events: Vec<Event> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .flat_map(|v| events(&v))
        .collect();
    Ok(corrections(&events, repo_root, &session))
}

fn events(record: &Value) -> Vec<Event> {
    let time = record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(super::github::parse_time)
        .unwrap_or(0);
    let message = record.get("message");
    let content = message.and_then(|m| m.get("content"));
    match record.get("type").and_then(Value::as_str) {
        Some("user") => human_text(content).map(Event::Human).into_iter().collect(),
        Some("assistant") => content
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
            .flat_map(|b| tool_edits(b, time))
            .collect(),
        _ => Vec::new(),
    }
}

/// Text the human typed. Tool results and harness-injected messages are skipped.
fn human_text(content: Option<&Value>) -> Option<String> {
    let text = match content? {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            if blocks
                .iter()
                .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            {
                return None;
            }
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => return None,
    };
    let trimmed = text.trim();
    let injected = trimmed.starts_with('<') || trimmed.starts_with("Caveat:");
    (!trimmed.is_empty() && !injected).then(|| trimmed.to_owned())
}

fn tool_edits(block: &Value, time: i64) -> Vec<Event> {
    let input = block.get("input");
    let s = |v: Option<&Value>, k: &str| {
        v.and_then(|i| i.get(k))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let Some(path) = s(input, "file_path") else {
        return Vec::new();
    };
    match block.get("name").and_then(Value::as_str) {
        Some("Edit") => match (s(input, "old_string"), s(input, "new_string")) {
            (Some(old), Some(new)) => vec![Event::Edit {
                path,
                old,
                new,
                time,
            }],
            _ => Vec::new(),
        },
        Some("MultiEdit") => input
            .and_then(|i| i.get("edits"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some(Event::Edit {
                    path: path.clone(),
                    old: s(Some(e), "old_string")?,
                    new: s(Some(e), "new_string")?,
                    time,
                })
            })
            .collect(),
        Some("Write") => s(input, "content")
            .map(|new| Event::Edit {
                path,
                old: String::new(),
                new,
                time,
            })
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn corrections(events: &[Event], repo_root: &Path, session: &str) -> Vec<Correction> {
    let mut out = Vec::new();
    for (i, event) in events.iter().enumerate() {
        let Event::Edit {
            path,
            new: written,
            time: written_at,
            ..
        } = event
        else {
            continue;
        };
        // Look ahead: a human message, then the next edit of the same file.
        let mut note: Option<&str> = None;
        for later in &events[i + 1..] {
            match later {
                Event::Human(text) => {
                    note.get_or_insert(text);
                }
                Event::Edit {
                    path: p,
                    old,
                    new,
                    time,
                } if p == path => {
                    let rewrites_it = !old.trim().is_empty() && written.contains(old.as_str());
                    if let (Some(note), true) = (note, rewrites_it)
                        && old.trim() != new.trim()
                        && let Some(c) =
                            build(path, old, new, note, *written_at, *time, repo_root, session)
                    {
                        out.push(c);
                    }
                    break;
                }
                Event::Edit { .. } => {}
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn build(
    abs_path: &str,
    old: &str,
    new: &str,
    note: &str,
    agent_time: i64,
    time: i64,
    repo_root: &Path,
    session: &str,
) -> Option<Correction> {
    let rel = Path::new(abs_path)
        .strip_prefix(repo_root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    let language = lang::detect(&rel).filter(|l| lang::is_code(*l))?;
    let before = snippet::whole(old);
    let after = snippet::whole(new);
    let source = Source::Transcript {
        session: session.to_owned(),
    };
    Some(Correction {
        id: Correction::make_id(&rel, &before, &after, &source),
        path: rel,
        lang: lang::name(language),
        before,
        after,
        source,
        agent_time,
        time,
        note: Some(note.chars().take(500).collect()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_rewrite_after_human_message() {
        let root = Path::new("/repo");
        let lines = [
            r#"{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/src/a.ts","old_string":"x","new_string":"const r = await fetch(url);\nreturn r;"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
            r#"{"type":"user","message":{"content":"no, use apiClient instead of fetch"}}"#,
            r#"{"type":"assistant","timestamp":"2026-01-01T00:01:00Z","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/repo/src/a.ts","old_string":"const r = await fetch(url);","new_string":"const r = await apiClient.get(url);"}}]}}"#,
        ];
        let events: Vec<Event> = lines
            .iter()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .flat_map(|v| events(&v))
            .collect();
        let found = corrections(&events, root, "s1");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "src/a.ts");
        assert_eq!(
            found[0].note.as_deref(),
            Some("no, use apiClient instead of fetch")
        );
        assert_eq!(
            found[0].after.changed_text(),
            "const r = await apiClient.get(url);"
        );
    }

    #[test]
    fn no_human_message_no_correction() {
        let events = vec![
            Event::Edit {
                path: "/repo/a.ts".into(),
                old: String::new(),
                new: "let a = 1;".into(),
                time: 0,
            },
            Event::Edit {
                path: "/repo/a.ts".into(),
                old: "let a = 1;".into(),
                new: "const a = 1;".into(),
                time: 1,
            },
        ];
        assert!(corrections(&events, Path::new("/repo"), "s").is_empty());
    }

    #[test]
    fn encodes_project_dir() {
        assert_eq!(encode(Path::new("/Users/me/my.repo")), "-Users-me-my-repo");
    }
}
