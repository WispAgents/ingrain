//! Mine ```suggestion blocks from pull-request review comments via `gh api`.
//!
//! A suggestion is an exact before/after pair: the commented lines and the
//! reviewer's replacement. Only comments on lines an agent commit wrote count.

use super::snippet;
use crate::agents::{Author, Classifier};
use anyhow::{Context, Result, bail};
use ingrain_core::{Correction, Git, LineRange, Snippet, Source};
use ingrain_engine::lang;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewComment {
    pub body: String,
    pub diff_hunk: String,
    pub path: String,
    #[serde(default)]
    pub original_line: Option<u32>,
    #[serde(default)]
    pub original_start_line: Option<u32>,
    pub original_commit_id: String,
    pub html_url: String,
    pub created_at: String,
    pub user: Option<User>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub login: String,
}

/// Fetch review comments for `owner/name`, newest first, up to `max_pages` pages of 100.
pub fn fetch(repo: &str, max_pages: u32) -> Result<Vec<ReviewComment>> {
    let mut all = Vec::new();
    for page in 1..=max_pages {
        let path = format!(
            "repos/{repo}/pulls/comments?sort=created&direction=desc&per_page=100&page={page}"
        );
        let output = ingrain_core::process::command("gh")
            .args(["api", &path])
            .output()
            .context("failed to run `gh`; install the GitHub CLI and run `gh auth login`")?;
        if !output.status.success() {
            bail!(
                "gh api {path} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let batch: Vec<ReviewComment> =
            serde_json::from_slice(&output.stdout).context("unexpected response from GitHub")?;
        let done = batch.len() < 100;
        all.extend(batch);
        if done {
            break;
        }
    }
    Ok(all)
}

/// Turn suggestion comments on agent-written lines into corrections.
/// `git` is used to check who wrote the commented commit; comments on commits
/// that aren't available locally are skipped.
pub fn corrections(
    git: &Git,
    comments: &[ReviewComment],
    classifier: &Classifier,
) -> Vec<Correction> {
    comments
        .iter()
        .filter_map(|c| {
            let reviewer_is_human = c
                .user
                .as_ref()
                .is_some_and(|u| classifier.classify("", &u.login, "") == Author::Human);
            if !reviewer_is_human {
                return None;
            }
            let (start, end) = commented_lines(c)?;
            let (agent_time, author) = line_author(git, c, start, end, classifier)?;
            if author != Author::Agent {
                return None;
            }
            let time = parse_time(&c.created_at)?;
            from_comment(c, agent_time, time)
        })
        .collect()
}

/// The commented line range, if well-formed.
fn commented_lines(c: &ReviewComment) -> Option<(u32, u32)> {
    let end = c.original_line?;
    let start = c.original_start_line.unwrap_or(end);
    (start >= 1 && start <= end).then_some((start, end))
}

/// Who wrote the commented lines: blame them at the commit the comment was made on.
/// (That commit is the PR head at the time, which may be someone else's commit.)
fn line_author(
    git: &Git,
    c: &ReviewComment,
    start: u32,
    end: u32,
    classifier: &Classifier,
) -> Option<(i64, Author)> {
    let range = format!("{start},{end}");
    let out = git.try_run(&[
        "blame",
        "--porcelain",
        "-L",
        &range,
        &c.original_commit_id,
        "--",
        &c.path,
    ])?;
    let origins = super::git::parse_blame(&out);
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for o in origins.values() {
        *counts.entry(o.sha.as_str()).or_default() += 1;
    }
    let (sha, _) = counts.into_iter().max_by_key(|(sha, n)| (*n, *sha))?;
    commit_author(git, sha, classifier)
}

fn commit_author(git: &Git, sha: &str, classifier: &Classifier) -> Option<(i64, Author)> {
    let out = git.try_run(&["log", "-1", "--format=%at%x1f%an%x1f%ae%x1f%B", sha])?;
    let mut f = out.splitn(4, '\x1f');
    let time = f.next()?.trim().parse().ok()?;
    let name = f.next()?;
    let email = f.next()?;
    let body = f.next().unwrap_or("");
    Some((time, classifier.classify(body, name, email)))
}

/// Build a correction from one comment, if it carries a suggestion.
pub fn from_comment(c: &ReviewComment, agent_time: i64, time: i64) -> Option<Correction> {
    let suggestion = suggestion(&c.body)?;
    let language = lang::detect(&c.path).filter(|l| lang::is_code(*l))?;
    let (start, end) = commented_lines(c)?;
    let commented = (end - start + 1) as usize;

    // New-side lines of the diff hunk; the comment covers its last `commented` lines.
    let new_side: Vec<&str> = c
        .diff_hunk
        .lines()
        .skip(1)
        .filter(|l| !l.starts_with('-') && !l.starts_with('\\'))
        .map(|l| l.get(1..).unwrap_or(""))
        .collect();
    if new_side.len() < commented {
        return None;
    }
    let context = &new_side[..new_side.len() - commented];
    let before_lines = &new_side[new_side.len() - commented..];

    let before = with_context(context, before_lines, start);
    let after_lines: Vec<&str> = suggestion.lines().collect();
    let after = with_context(context, &after_lines, start);
    let source = Source::ReviewSuggestion {
        url: c.html_url.clone(),
    };
    let note = c
        .body
        .split("```")
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    Some(Correction {
        id: Correction::make_id(&c.path, &before, &after, &source),
        path: c.path.clone(),
        lang: lang::name(language),
        before,
        after,
        source,
        agent_time,
        time,
        note: note.map(str::to_owned),
    })
}

fn with_context(context: &[&str], changed: &[&str], first_changed_line: u32) -> Snippet {
    let mut text: Vec<&str> = context.to_vec();
    text.extend_from_slice(changed);
    let start = context.len() as u32 + 1;
    let range = if changed.is_empty() {
        LineRange::empty()
    } else {
        LineRange::new(start, start + changed.len() as u32 - 1)
    };
    let mut s = snippet::whole(&text.join("\n"));
    s.changed = range;
    s.first_line = first_changed_line
        .saturating_sub(context.len() as u32)
        .max(1);
    s
}

/// The body of the first ```suggestion block.
fn suggestion(body: &str) -> Option<String> {
    let start = body.find("```suggestion")?;
    let rest = &body[start + "```suggestion".len()..];
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;
    let end = rest.find("```")?;
    Some(
        rest[..end]
            .trim_end_matches(['\n', '\r'])
            .replace("\r\n", "\n"),
    )
}

/// Parse `2026-03-04T05:06:07Z` to unix seconds (UTC).
pub fn parse_time(s: &str) -> Option<i64> {
    let (date, time) = s.trim_end_matches('Z').split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time
        .split(':')
        .map(|p| p.split('.').next().and_then(|x| x.parse::<i64>().ok()));
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next().flatten().unwrap_or(0));
    Some(days_from_civil(y, m, day) * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(body: &str) -> ReviewComment {
        ReviewComment {
            body: body.into(),
            diff_hunk:
                "@@ -1,3 +1,4 @@\n import x\n+const a = 1;\n-old\n+const r = await fetch(url);"
                    .into(),
            path: "src/a.ts".into(),
            original_line: Some(3),
            original_start_line: None,
            original_commit_id: "c".into(),
            html_url: "https://github.com/o/r/pull/1#discussion_r1".into(),
            created_at: "2026-03-04T05:06:07Z".into(),
            user: Some(User {
                login: "ana".into(),
            }),
        }
    }

    #[test]
    fn builds_correction_from_suggestion() {
        let c = comment("Use the client\n```suggestion\nconst r = await apiClient.get(url);\n```");
        let corr = from_comment(&c, 0, 1).unwrap();
        assert_eq!(corr.before.changed_text(), "const r = await fetch(url);");
        assert_eq!(
            corr.after.changed_text(),
            "const r = await apiClient.get(url);"
        );
        assert_eq!(corr.note.as_deref(), Some("Use the client"));
        assert!(corr.before.text.starts_with("import x"));
    }

    #[test]
    fn ignores_plain_comments() {
        assert!(from_comment(&comment("nit: rename"), 0, 1).is_none());
    }

    #[test]
    fn parses_timestamps() {
        assert_eq!(parse_time("1970-01-02T00:00:00Z"), Some(86_400));
        assert_eq!(parse_time("2026-03-04T05:06:07Z"), Some(1_772_600_767));
    }
}
