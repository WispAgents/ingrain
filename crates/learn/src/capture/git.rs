//! Mine corrections from git history: human commits that rewrite lines an agent commit added.

use super::snippet::{self, CONTEXT};
use crate::agents::{Author, Classifier};
use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ingrain_core::diff::{self, FileDiff, Hunk};
use ingrain_core::{AgentHunk, CorrectedLines, Correction, Git, LineRange, Source};
use ingrain_engine::lang;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;

/// git's well-known empty tree, used as the parent of root commits.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Debug, Clone)]
pub struct GitMineOptions {
    /// A human change counts as a correction only this long after the agent commit.
    pub window_days: u32,
    /// Human commits touching more files than this are refactors, not corrections.
    pub max_files: usize,
    /// Human commits changing more lines than this are skipped too.
    pub max_changed_lines: usize,
    /// Largest hunk (old side) considered a correction.
    pub max_hunk_lines: u32,
    /// Agent hunks are split into chunks of at most this many lines.
    pub agent_chunk_lines: u32,
    /// Agent commits adding more lines than this are skipped (vendored code, generated files).
    pub max_agent_commit_lines: usize,
    /// Revisions to walk, e.g. `["HEAD"]` or `["--all"]`.
    pub revs: Vec<String>,
    /// `git log --since` value.
    pub since: Option<String>,
    /// Only the newest N commits.
    pub max_commits: Option<usize>,
    pub ignore: Vec<String>,
    pub extra_markers: Vec<String>,
}

impl Default for GitMineOptions {
    fn default() -> Self {
        Self {
            window_days: 14,
            max_files: 20,
            max_changed_lines: 400,
            max_hunk_lines: 30,
            agent_chunk_lines: 40,
            max_agent_commit_lines: 3000,
            revs: vec!["HEAD".into()],
            since: None,
            max_commits: None,
            ignore: Vec::new(),
            extra_markers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct MineStats {
    pub commits: usize,
    pub agent_commits: usize,
    pub human_commits: usize,
    pub bot_commits: usize,
    /// Human commits skipped as refactors or formatter runs.
    pub human_commits_skipped_large: usize,
    pub hunks_blamed: usize,
    pub corrections: usize,
    pub agent_hunks: usize,
    pub agent_commits_skipped_large: usize,
}

#[derive(Debug, Default)]
pub struct GitMine {
    pub corrections: Vec<Correction>,
    pub agent_hunks: Vec<AgentHunk>,
    pub stats: MineStats,
}

#[derive(Debug, Clone)]
struct Commit {
    sha: String,
    parent: Option<String>,
    time: i64,
    subject: String,
    author: Author,
}

/// A corrected range in an agent commit's version of a file.
#[derive(Debug, Clone)]
struct Link {
    agent_commit: String,
    path: String,
    lines: LineRange,
    time: i64,
    correction: String,
}

pub fn mine(git: &Git, opts: &GitMineOptions) -> Result<GitMine> {
    let classifier = Classifier::new(&opts.extra_markers);
    let ignore = build_globs(&opts.ignore)?;
    let commits = log(git, opts, &classifier)?;
    let by_sha: HashMap<&str, &Commit> = commits.iter().map(|c| (c.sha.as_str(), c)).collect();

    let mut stats = MineStats {
        commits: commits.len(),
        ..Default::default()
    };
    for c in &commits {
        match c.author {
            Author::Agent => stats.agent_commits += 1,
            Author::Human => stats.human_commits += 1,
            Author::Bot => stats.bot_commits += 1,
        }
    }

    let window = i64::from(opts.window_days) * 86_400;
    let ctx = Ctx {
        git,
        opts,
        ignore: &ignore,
        by_sha: &by_sha,
        window,
    };

    let human: Vec<&Commit> = commits
        .iter()
        .filter(|c| c.author == Author::Human && c.parent.is_some())
        .collect();
    let results: Vec<HumanResult> = human.par_iter().map(|c| ctx.mine_human(c)).collect();

    let mut corrections = Vec::new();
    let mut links = Vec::new();
    for r in results {
        stats.human_commits_skipped_large += usize::from(r.skipped_large);
        stats.hunks_blamed += r.hunks_blamed;
        corrections.extend(r.corrections);
        links.extend(r.links);
    }

    let mut links_by_commit: HashMap<(&str, &str), Vec<&Link>> = HashMap::new();
    for link in &links {
        links_by_commit
            .entry((link.agent_commit.as_str(), link.path.as_str()))
            .or_default()
            .push(link);
    }

    let agent: Vec<&Commit> = commits
        .iter()
        .filter(|c| c.author == Author::Agent)
        .collect();
    let agent_results: Vec<(Vec<AgentHunk>, bool)> = agent
        .par_iter()
        .map(|c| ctx.mine_agent(c, &links_by_commit))
        .collect();
    let mut agent_hunks = Vec::new();
    for (hunks, skipped) in agent_results {
        stats.agent_commits_skipped_large += usize::from(skipped);
        agent_hunks.extend(hunks);
    }

    corrections.sort_by(|a, b| a.time.cmp(&b.time).then(a.id.cmp(&b.id)));
    corrections.dedup_by(|a, b| a.id == b.id);
    agent_hunks.sort_by(|a, b| a.time.cmp(&b.time).then(a.path.cmp(&b.path)));
    stats.corrections = corrections.len();
    stats.agent_hunks = agent_hunks.len();
    Ok(GitMine {
        corrections,
        agent_hunks,
        stats,
    })
}

struct Ctx<'a> {
    git: &'a Git,
    opts: &'a GitMineOptions,
    ignore: &'a GlobSet,
    by_sha: &'a HashMap<&'a str, &'a Commit>,
    window: i64,
}

#[derive(Default)]
struct HumanResult {
    corrections: Vec<Correction>,
    links: Vec<Link>,
    skipped_large: bool,
    hunks_blamed: usize,
}

impl Ctx<'_> {
    /// Code files in a diff that are worth looking at.
    fn wanted(&self, path: &str) -> bool {
        !self.ignore.is_match(path) && lang::detect(path).is_some_and(lang::is_code)
    }

    fn diff(&self, parent: &str, commit: &str) -> Vec<FileDiff> {
        self.git
            .try_run(&[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "-U0",
                "-M",
                parent,
                commit,
            ])
            .map(|t| diff::parse(&t))
            .unwrap_or_default()
    }

    fn mine_human(&self, commit: &Commit) -> HumanResult {
        let mut out = HumanResult::default();
        if is_formatting(&commit.subject) {
            out.skipped_large = true;
            return out;
        }
        let parent = commit
            .parent
            .as_deref()
            .expect("filtered to commits with a parent");
        let files = self.diff(parent, &commit.sha);
        let changed: usize = files
            .iter()
            .flat_map(|f| &f.hunks)
            .map(|h| (h.old_len + h.new_len) as usize)
            .sum();
        if files.len() > self.opts.max_files || changed > self.opts.max_changed_lines {
            out.skipped_large = true;
            return out;
        }

        for file in &files {
            let (Some(old_path), Some(new_path)) = (&file.old_path, &file.new_path) else {
                continue;
            };
            if !self.wanted(new_path) {
                continue;
            }
            let hunks: Vec<&Hunk> = file
                .hunks
                .iter()
                .filter(|h| {
                    h.old_len >= 1
                        && h.old_len <= self.opts.max_hunk_lines
                        && h.new_len <= self.opts.max_hunk_lines * 2
                        && !cosmetic_only(h)
                })
                .collect();
            if hunks.is_empty() {
                continue;
            }
            out.hunks_blamed += hunks.len();
            let Some(origins) = self.blame(parent, old_path, &hunks) else {
                continue;
            };
            let (Some(before_src), Some(after_src)) = (
                self.git.show(parent, old_path),
                self.git.show(&commit.sha, new_path),
            ) else {
                continue;
            };
            for hunk in hunks {
                if let Some((correction, link)) =
                    self.correction(commit, hunk, &origins, new_path, &before_src, &after_src)
                {
                    out.links.push(link);
                    out.corrections.push(correction);
                }
            }
        }
        out
    }

    /// Blame all hunks of one file in a single call.
    fn blame(&self, rev: &str, path: &str, hunks: &[&Hunk]) -> Option<HashMap<u32, Origin>> {
        let mut args: Vec<String> = vec!["blame".into(), "--porcelain".into(), "-w".into()];
        for h in hunks {
            let r = h.old_range();
            args.push("-L".into());
            args.push(format!("{},{}", r.start, r.end));
        }
        args.push(rev.into());
        args.push("--".into());
        args.push(path.into());
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.git.try_run(&refs).map(|t| parse_blame(&t))
    }

    fn correction(
        &self,
        commit: &Commit,
        hunk: &Hunk,
        origins: &HashMap<u32, Origin>,
        path: &str,
        before_src: &str,
        after_src: &str,
    ) -> Option<(Correction, Link)> {
        let old = hunk.old_range();
        // The agent commit that wrote most of the replaced lines.
        let mut per_commit: HashMap<&str, Vec<&Origin>> = HashMap::new();
        for line in old.start..=old.end {
            if let Some(o) = origins.get(&line) {
                per_commit.entry(o.sha.as_str()).or_default().push(o);
            }
        }
        let (sha, lines) = per_commit.into_iter().max_by_key(|(_, v)| v.len())?;
        if (lines.len() as u32) * 2 < old.len() {
            return None;
        }
        let agent = self.by_sha.get(sha)?;
        let age = commit.time - agent.time;
        if agent.author != Author::Agent || age < 0 || age > self.window {
            return None;
        }

        let before = snippet::cut(before_src, old, old.start, CONTEXT);
        let after = snippet::cut(after_src, hunk.new_range(), hunk.new_start, CONTEXT);
        let source = Source::GitFixup {
            agent_commit: agent.sha.clone(),
            fix_commit: commit.sha.clone(),
        };
        let id = Correction::make_id(path, &before, &after, &source);
        let orig_lines: Vec<u32> = lines.iter().map(|o| o.orig_line).collect();
        let link = Link {
            agent_commit: agent.sha.clone(),
            path: lines[0].filename.clone().unwrap_or_else(|| path.to_owned()),
            lines: LineRange::new(*orig_lines.iter().min()?, *orig_lines.iter().max()?),
            time: commit.time,
            correction: id.clone(),
        };
        let correction = Correction {
            id,
            path: path.to_owned(),
            lang: lang::name(lang::detect(path)?),
            before,
            after,
            source,
            agent_time: agent.time,
            time: commit.time,
            note: Some(commit.subject.clone()),
        };
        Some((correction, link))
    }

    /// Added code of one agent commit, split into chunks, with corrected lines attached.
    fn mine_agent(
        &self,
        commit: &Commit,
        links: &HashMap<(&str, &str), Vec<&Link>>,
    ) -> (Vec<AgentHunk>, bool) {
        let parent = commit.parent.as_deref().unwrap_or(EMPTY_TREE);
        let files = self.diff(parent, &commit.sha);
        let added: usize = files
            .iter()
            .flat_map(|f| &f.hunks)
            .map(|h| h.new_len as usize)
            .sum();
        if added > self.opts.max_agent_commit_lines {
            return (Vec::new(), true);
        }
        let mut out = Vec::new();
        for file in files {
            let Some(path) = file.new_path else { continue };
            if !self.wanted(&path) {
                continue;
            }
            let Some(content) = self.git.show(&commit.sha, &path) else {
                continue;
            };
            let file_links = links.get(&(commit.sha.as_str(), path.as_str()));
            let lang_name = lang::name(lang::detect(&path).expect("wanted() checked the language"));
            for hunk in &file.hunks {
                for chunk in chunks(hunk.new_range(), self.opts.agent_chunk_lines) {
                    let snippet = snippet::cut(&content, chunk, chunk.start, CONTEXT);
                    let corrected = file_links
                        .into_iter()
                        .flatten()
                        .filter_map(|link| {
                            let start = link.lines.start.max(chunk.start);
                            let end = link.lines.end.min(chunk.end);
                            (start <= end).then(|| CorrectedLines {
                                lines: LineRange::new(
                                    start - snippet.first_line + 1,
                                    end - snippet.first_line + 1,
                                ),
                                time: link.time,
                                correction: link.correction.clone(),
                            })
                        })
                        .collect();
                    out.push(AgentHunk {
                        commit: commit.sha.clone(),
                        path: path.clone(),
                        lang: lang_name.clone(),
                        time: commit.time,
                        snippet,
                        corrected,
                    });
                }
            }
        }
        (out, false)
    }
}

fn chunks(range: LineRange, size: u32) -> Vec<LineRange> {
    if range.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = range.start;
    while start <= range.end {
        let end = (start + size - 1).min(range.end);
        out.push(LineRange::new(start, end));
        start = end + 1;
    }
    out
}

/// Whitespace, trailing commas and quote style only: what formatters change.
fn cosmetic_only(h: &Hunk) -> bool {
    let squash = |lines: &[String]| -> String {
        lines
            .iter()
            .flat_map(|l| l.chars())
            .filter(|c| !c.is_whitespace() && *c != ',')
            .map(|c| if c == '\'' || c == '`' { '"' } else { c })
            .collect()
    };
    squash(&h.removed) == squash(&h.added)
}

/// Commits that only run a formatter. Their hunks are not corrections.
fn is_formatting(subject: &str) -> bool {
    let s = subject.to_lowercase();
    [
        "black",
        "prettier",
        "isort",
        "ruff format",
        "rustfmt",
        "gofmt",
        "biome format",
        "reformat",
        "formatting",
        "run format",
        "cargo fmt",
        "go fmt",
    ]
    .iter()
    .any(|k| s.contains(k))
}

fn build_globs(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(Glob::new(p)?);
    }
    Ok(b.build()?)
}

fn log(git: &Git, opts: &GitMineOptions, classifier: &Classifier) -> Result<Vec<Commit>> {
    let mut args: Vec<String> = vec![
        "log".into(),
        "--no-merges".into(),
        "--format=%H%x1f%P%x1f%at%x1f%an%x1f%ae%x1f%B%x1e".into(),
    ];
    if let Some(since) = &opts.since {
        args.push(format!("--since={since}"));
    }
    if let Some(n) = opts.max_commits {
        args.push(format!("--max-count={n}"));
    }
    args.extend(opts.revs.iter().cloned());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let text = git.run(&refs)?;
    Ok(text
        .split('\x1e')
        .filter_map(|record| {
            let mut f = record.trim_start_matches('\n').splitn(6, '\x1f');
            let sha = f.next()?.trim().to_owned();
            if sha.is_empty() {
                return None;
            }
            let parent = f.next()?.split_whitespace().next().map(str::to_owned);
            let time = f.next()?.parse().ok()?;
            let name = f.next()?;
            let email = f.next()?;
            let body = f.next().unwrap_or("");
            Some(Commit {
                author: classifier.classify(body, name, email),
                subject: body.lines().next().unwrap_or("").to_owned(),
                sha,
                parent,
                time,
            })
        })
        .collect())
}

/// Where a blamed line came from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Origin {
    sha: String,
    orig_line: u32,
    filename: Option<String>,
}

/// Parse `git blame --porcelain` into final line → origin.
fn parse_blame(text: &str) -> HashMap<u32, Origin> {
    let mut filenames: HashMap<String, String> = HashMap::new();
    let mut lines: Vec<(u32, String, u32)> = Vec::new();
    let mut current: Option<(String, u32, u32)> = None;
    for line in text.lines() {
        if line.starts_with('\t') {
            if let Some((sha, orig, fin)) = current.take() {
                lines.push((fin, sha, orig));
            }
        } else if let Some(name) = line.strip_prefix("filename ") {
            if let Some((sha, _, _)) = &current {
                filenames.insert(sha.clone(), name.to_owned());
            }
        } else {
            let mut parts = line.split(' ');
            let sha = parts.next().unwrap_or("");
            if sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
                let orig = parts.next().and_then(|p| p.parse().ok());
                let fin = parts.next().and_then(|p| p.parse().ok());
                if let (Some(orig), Some(fin)) = (orig, fin) {
                    current = Some((sha.to_owned(), orig, fin));
                }
            }
        }
    }
    lines
        .into_iter()
        .map(|(fin, sha, orig)| {
            let filename = filenames.get(&sha).cloned();
            (
                fin,
                Origin {
                    sha,
                    orig_line: orig,
                    filename,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let text = format!(
            "{a} 3 10 2\nauthor X\nfilename src/old.ts\n\tline ten\n{a} 4 11\n\tline eleven\n{b} 1 12 1\nauthor Y\nfilename src/new.ts\n\tline twelve\n"
        );
        let map = parse_blame(&text);
        assert_eq!(map[&10].sha, a);
        assert_eq!(map[&11].orig_line, 4);
        assert_eq!(map[&11].filename.as_deref(), Some("src/old.ts"));
        assert_eq!(map[&12].filename.as_deref(), Some("src/new.ts"));
    }

    #[test]
    fn cosmetic_changes_are_not_corrections() {
        let h = |removed: &[&str], added: &[&str]| Hunk {
            old_start: 1,
            old_len: removed.len() as u32,
            new_start: 1,
            new_len: added.len() as u32,
            removed: removed.iter().map(|s| s.to_string()).collect(),
            added: added.iter().map(|s| s.to_string()).collect(),
        };
        assert!(cosmetic_only(&h(
            &["f(a=1, b='x')"],
            &["f(", "    a=1,", "    b=\"x\",", ")"]
        )));
        assert!(!cosmetic_only(&h(&["fetch(u)"], &["api.get(u)"])));
        assert!(is_formatting("Run black formatter"));
        assert!(!is_formatting("Fix permission check"));
    }

    #[test]
    fn chunking() {
        assert_eq!(
            chunks(LineRange::new(1, 5), 2),
            vec![
                LineRange::new(1, 2),
                LineRange::new(3, 4),
                LineRange::new(5, 5)
            ]
        );
        assert!(chunks(LineRange::empty(), 3).is_empty());
    }
}
