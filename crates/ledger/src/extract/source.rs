//! Texts decisions are extracted from: commit messages, GitHub PRs, reviews, issues and
//! comments, and what humans typed in Claude Code sessions.

use anyhow::{Context, Result, bail};
use ingrain_core::Git;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub id: String,
    /// commit | pr | review | issue | issue_comment | session
    pub kind: String,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
    pub text: String,
    /// Link or reference stored as the decision's source.
    pub url: String,
    /// Files the text is about (a commit's or review comment's paths), to help scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}

/// Bots, dependency updates and very short texts state no decisions.
pub fn worth_reading(t: &Text) -> bool {
    let author = t.author.to_lowercase();
    let bot = author.ends_with("[bot]")
        || ["dependabot", "renovate", "github-actions"]
            .iter()
            .any(|b| author.contains(b));
    !bot && t.text.trim().chars().count() >= 40 || (t.kind == "doc" && !t.text.trim().is_empty())
}

/// Commit messages after `since` (exclusive), newest first, at most `max`.
pub fn commits(git: &Git, since: i64, max: usize) -> Result<Vec<Text>> {
    let mut args = vec![
        "log".to_owned(),
        "--no-merges".into(),
        "--name-only".into(),
        format!("--max-count={max}"),
        "--format=%x1e%H%x1f%at%x1f%an%x1f%B%x1f".into(),
    ];
    if since > 0 {
        args.push(format!("--since=@{}", since + 1));
    }
    args.push("HEAD".into());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let Some(out) = git.try_run(&refs) else {
        return Ok(Vec::new()); // no commits yet
    };
    let remote = github_name(git.root()).ok();
    Ok(out
        .split('\x1e')
        .filter_map(|rec| {
            let mut f = rec.split('\x1f');
            let sha = f.next()?.trim().to_owned();
            let time = f.next()?.parse().ok()?;
            let author = f.next()?.to_owned();
            let body = f.next()?.trim().to_owned();
            let paths = f
                .next()
                .unwrap_or("")
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect();
            let url = match &remote {
                Some(r) => format!("https://github.com/{r}/commit/{sha}"),
                None => format!("commit {sha}"),
            };
            (!sha.is_empty()).then(|| Text {
                id: format!("c{}", &sha[..12.min(sha.len())]),
                kind: "commit".into(),
                author,
                time,
                text: body,
                url,
                paths,
            })
        })
        .collect())
}

/// Sections of docs that state the project's rules (AGENTS.md, CONTRIBUTING.md, an ADR folder),
/// split at headings and blank-line paragraphs into pieces of at most ~2,500 characters.
pub fn docs(git: &Git, paths: &[String]) -> Result<Vec<Text>> {
    let mut out = Vec::new();
    for path in paths {
        let rel = git.relative(Path::new(path));
        let text = std::fs::read_to_string(git.root().join(&rel))
            .with_context(|| format!("reading {rel}"))?;
        // Last commit touching the file, so the record's date means something.
        let time = git
            .try_run(&["log", "-1", "--format=%ct", "--", &rel])
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or_else(crate::record::now);
        let mut piece = String::new();
        let mut n = 0;
        let mut flush = |piece: &mut String, out: &mut Vec<Text>| {
            if !piece.trim().is_empty() {
                n += 1;
                out.push(Text {
                    id: format!("doc:{rel}#{n}"),
                    kind: "doc".into(),
                    author: String::new(),
                    time,
                    text: piece.trim().to_owned(),
                    url: rel.clone(),
                    paths: Vec::new(),
                });
            }
            piece.clear();
        };
        for para in text.split("\n\n") {
            let heading = para.trim_start().starts_with('#');
            if heading || piece.len() + para.len() > 2500 {
                flush(&mut piece, &mut out);
            }
            piece.push_str(para);
            piece.push_str("\n\n");
        }
        flush(&mut piece, &mut out);
    }
    Ok(out)
}

/// `owner/name` of the repository's GitHub remote, via `gh`.
pub fn github_name(root: &Path) -> Result<String> {
    let out = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ])
        .current_dir(root)
        .output()
        .context("failed to run `gh`")?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if !out.status.success() || name.is_empty() {
        bail!("couldn't detect the GitHub repo; pass --github owner/name");
    }
    Ok(name)
}

fn gh(path: &str) -> Result<Vec<Value>> {
    let out = Command::new("gh")
        .args(["api", path])
        .output()
        .context("failed to run `gh`; install it and run `gh auth login`")?;
    if !out.status.success() {
        bail!(
            "gh api {path}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

fn time_of(v: &Value, key: &str) -> i64 {
    v[key].as_str().and_then(parse_time).unwrap_or(0)
}

/// PRs, review comments, issues and issue comments created after `since`, newest first,
/// up to `max_pages` pages of 100 per stream. The flag is true when a stream hit the page
/// limit before reaching `since`.
pub fn github(repo: &str, since: i64, max_pages: u32) -> Result<(Vec<Text>, bool)> {
    let mut out = Vec::new();
    let mut truncated = false;
    let streams = [
        (
            "pr",
            format!("repos/{repo}/pulls?state=all&sort=created&direction=desc"),
        ),
        (
            "review",
            format!("repos/{repo}/pulls/comments?sort=created&direction=desc"),
        ),
        (
            "issue",
            format!("repos/{repo}/issues?state=all&sort=created&direction=desc"),
        ),
        (
            "issue_comment",
            format!("repos/{repo}/issues/comments?sort=created&direction=desc"),
        ),
    ];
    for (kind, base) in streams {
        'pages: for page in 1..=max_pages {
            let batch = gh(&format!("{base}&per_page=100&page={page}"))?;
            let full = batch.len() == 100;
            for v in batch {
                let time = time_of(&v, "created_at");
                if time <= since {
                    break 'pages;
                }
                if kind == "issue" && v.get("pull_request").is_some() {
                    continue;
                }
                // Only people with a role in the repo state its decisions; anyone can open an issue.
                let role = v["author_association"].as_str().unwrap_or("");
                if !matches!(role, "OWNER" | "MEMBER" | "COLLABORATOR") {
                    continue;
                }
                let text = match kind {
                    "pr" | "issue" => format!(
                        "{}\n\n{}",
                        v["title"].as_str().unwrap_or(""),
                        v["body"].as_str().unwrap_or("")
                    ),
                    _ => v["body"].as_str().unwrap_or("").to_owned(),
                };
                let id = match kind {
                    "pr" => format!("pr{}", v["number"]),
                    "issue" => format!("is{}", v["number"]),
                    "review" => format!("rc{}", v["id"]),
                    _ => format!("ic{}", v["id"]),
                };
                out.push(Text {
                    id,
                    kind: kind.into(),
                    author: v["user"]["login"].as_str().unwrap_or("").to_owned(),
                    time,
                    text: text.trim().to_owned(),
                    url: v["html_url"].as_str().unwrap_or("").to_owned(),
                    paths: v["path"]
                        .as_str()
                        .map(|p| vec![p.to_owned()])
                        .unwrap_or_default(),
                });
            }
            if !full {
                break;
            }
            if page == max_pages {
                truncated = true;
            }
        }
    }
    Ok((out, truncated))
}

/// What the human typed in Claude Code sessions for this repository, after `since`.
pub fn sessions(projects_dir: &Path, repo_root: &Path, since: i64) -> Vec<Text> {
    let mut out = Vec::new();
    for file in session_files(projects_dir, repo_root) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let session = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        for (n, line) in text.lines().enumerate() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(said) = human_text(&v) else { continue };
            let time = time_of(&v, "timestamp");
            if time <= since {
                continue;
            }
            out.push(Text {
                id: format!("s{}-{n}", session.chars().take(8).collect::<String>()),
                kind: "session".into(),
                author: "you".into(),
                time,
                text: said,
                url: format!("session {session}"),
                paths: Vec::new(),
            });
        }
    }
    out
}

/// Session files: Claude Code names project dirs after the path, non-alphanumerics as `-`.
fn session_files(projects_dir: &Path, repo_root: &Path) -> Vec<PathBuf> {
    let key: String = repo_root
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
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

fn human_text(v: &Value) -> Option<String> {
    let flagged = |k: &str| v[k].as_bool().unwrap_or(false);
    if v["type"] != "user"
        || flagged("isSidechain")
        || flagged("isMeta")
        || flagged("isCompactSummary")
    {
        return None;
    }
    let text = match &v["message"]["content"] {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            if blocks.iter().any(|b| b["type"] == "tool_result") {
                return None;
            }
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => return None,
    };
    let t = text.trim();
    (!t.is_empty() && !t.starts_with('<') && !t.starts_with("Caveat:")).then(|| t.to_owned())
}

/// `2026-03-04T05:06:07Z` (or with offset/fraction) to unix seconds (UTC).
pub fn parse_time(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let clock = &time[..time.find(['Z', '+', '-']).unwrap_or(time.len())];
    let mut t = clock
        .split(':')
        .map(|p| p.split('.').next().and_then(|x| x.parse::<i64>().ok()));
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next().flatten().unwrap_or(0));
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hh * 3600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("1970-01-02T00:00:00Z"), Some(86_400));
        assert_eq!(parse_time("2026-03-04T05:06:07.123Z"), Some(1_772_600_767));
    }

    #[test]
    fn session_human_text_only() {
        let user: Value = serde_json::from_str(
            r#"{"type":"user","message":{"content":"no, we stay on SQLite; don't add Redis"}}"#,
        )
        .unwrap();
        let tool: Value = serde_json::from_str(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
        )
        .unwrap();
        let side: Value = serde_json::from_str(
            r#"{"type":"user","isSidechain":true,"message":{"content":"subagent prompt"}}"#,
        )
        .unwrap();
        assert!(human_text(&user).is_some());
        assert!(human_text(&tool).is_none());
        assert!(human_text(&side).is_none());
    }

    #[test]
    fn docs_split_at_headings() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            "# Rules\n\nUse apiClient.\n\n## Tests\n\nNo network in unit tests.\n",
        )
        .unwrap();
        let git = Git::open(dir.path()).unwrap();
        let texts = docs(&git, &["AGENTS.md".into()]).unwrap();
        assert_eq!(texts.len(), 2);
        assert_eq!(texts[0].id, "doc:AGENTS.md#1");
        assert!(texts[1].text.contains("No network"));
        assert!(
            texts.iter().all(worth_reading),
            "short doc sections still count"
        );
    }

    #[test]
    fn filters_bots_and_short_texts() {
        let t = |author: &str, text: &str| Text {
            id: "x".into(),
            kind: "pr".into(),
            author: author.into(),
            time: 0,
            text: text.into(),
            url: String::new(),
            paths: vec![],
        };
        assert!(!worth_reading(&t("dependabot[bot]", &"x".repeat(100))));
        assert!(!worth_reading(&t("ana", "lgtm")));
        assert!(worth_reading(&t(
            "ana",
            "We decided to keep SQLite for the cache; Redis is out of scope."
        )));
    }
}
