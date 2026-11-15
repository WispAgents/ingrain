//! Thin wrapper over the `git` CLI.

use crate::diff::{self, FileDiff};
use crate::model::LineRange;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A git repository addressed by its working directory.
#[derive(Debug, Clone)]
pub struct Git {
    root: PathBuf,
    git_dir: PathBuf,
}

/// What added lines are compared against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffBase {
    /// Working tree (including untracked files) against `HEAD`.
    Head,
    /// The index against `HEAD`.
    Staged,
    /// A revision range such as `origin/main...HEAD`.
    Range(String),
}

/// Where the checked version of a file lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    WorkingTree,
    Index,
    Revision(String),
}

impl DiffBase {
    /// The revision holding the "before" side, for comparing manifests and loading the ledger
    /// a change is judged against.
    pub fn before_rev(&self, git: &Git) -> Option<String> {
        match self {
            DiffBase::Head | DiffBase::Staged => git.has_head().then(|| "HEAD".to_owned()),
            DiffBase::Range(range) => {
                if let Some((a, b)) = range.split_once("...") {
                    let b = if b.is_empty() { "HEAD" } else { b };
                    git.try_run(&["merge-base", "--end-of-options", a, b])
                        .map(|s| s.trim().to_owned())
                } else {
                    Some(
                        range
                            .split_once("..")
                            .map_or(range.as_str(), |(a, _)| a)
                            .to_owned(),
                    )
                }
            }
        }
    }

    /// The side of the diff whose line numbers `added_lines` reports.
    pub fn content(&self) -> Content {
        match self {
            DiffBase::Head => Content::WorkingTree,
            DiffBase::Staged => Content::Index,
            DiffBase::Range(range) => {
                let right = range
                    .split_once("...")
                    .or_else(|| range.split_once(".."))
                    .map(|(_, r)| r);
                match right {
                    Some("") => Content::Revision("HEAD".into()),
                    Some(r) => Content::Revision(r.into()),
                    // `git diff <rev>` compares a revision with the working tree.
                    None => Content::WorkingTree,
                }
            }
        }
    }
}

impl Git {
    /// Open the repository containing `dir`.
    pub fn open(dir: &Path) -> Result<Self> {
        // One spawn for both paths: hooks run on every agent edit.
        let out = run_in(dir, &["rev-parse", "--show-toplevel", "--absolute-git-dir"])
            .with_context(|| format!("{} is not inside a git repository", dir.display()))?;
        let mut lines = out.lines().map(|l| PathBuf::from(normalize_path(l.trim())));
        let (Some(root), Some(git_dir)) = (lines.next(), lines.next()) else {
            bail!("unexpected `git rev-parse` output in {}", dir.display());
        };
        Ok(Self { root, git_dir })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Run git and return stdout as UTF-8 (lossy).
    pub fn run(&self, args: &[&str]) -> Result<String> {
        run_in(&self.root, args)
    }

    /// Like `run`, but a non-zero exit returns `None` instead of an error.
    pub fn try_run(&self, args: &[&str]) -> Option<String> {
        self.run(args).ok()
    }

    /// The `.git` directory (resolves worktrees).
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    pub fn has_head(&self) -> bool {
        self.try_run(&["rev-parse", "--verify", "-q", "HEAD"])
            .is_some()
    }

    /// File content at a revision, or `None` if the path doesn't exist there.
    pub fn show(&self, rev: &str, path: &str) -> Option<String> {
        self.try_run(&["show", "--end-of-options", &format!("{rev}:{path}")])
    }

    /// File content in the index (staged version).
    pub fn show_index(&self, path: &str) -> Option<String> {
        self.try_run(&["show", &format!(":{path}")])
    }

    /// Diff for one file or the whole tree, parsed.
    pub fn diff(&self, base: &DiffBase, path: Option<&str>) -> Result<Vec<FileDiff>> {
        let revs: Vec<&str> = match base {
            DiffBase::Head => vec!["HEAD"],
            DiffBase::Staged => vec!["--cached"],
            DiffBase::Range(range) => vec!["--end-of-options", range.as_str()],
        };
        match self.diff_revs_ignoring_whitespace(&revs, path) {
            // No commits yet: compare the index against the empty tree instead.
            Err(_) if *base == DiffBase::Head && !self.has_head() => {
                self.diff_revs_ignoring_whitespace(&["--cached"], path)
            }
            other => other,
        }
    }

    /// `git diff -U0` between revisions, with output that doesn't depend on the
    /// user's config: fixed `a/` `b/` prefixes, unquoted paths, renames detected
    /// (so a moved file doesn't count as fully added), no external diff tools.
    /// Whitespace changes count, so mining sees a fix that wraps and re-indents code.
    pub fn diff_revs(&self, revs: &[&str], path: Option<&str>) -> Result<Vec<FileDiff>> {
        self.diff_with(revs, path, false)
    }

    /// Like `diff_revs`, ignoring whitespace: re-indented existing code isn't "added".
    /// What checks use.
    fn diff_revs_ignoring_whitespace(
        &self,
        revs: &[&str],
        path: Option<&str>,
    ) -> Result<Vec<FileDiff>> {
        self.diff_with(revs, path, true)
    }

    fn diff_with(
        &self,
        revs: &[&str],
        path: Option<&str>,
        ignore_whitespace: bool,
    ) -> Result<Vec<FileDiff>> {
        let mut args = vec![
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "-U0",
            "-M",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ];
        if ignore_whitespace {
            args.push("-w");
        }
        args.extend_from_slice(revs);
        args.push("--");
        if let Some(p) = path {
            args.push(p);
        }
        Ok(diff::parse(&self.run(&args)?))
    }

    /// Untracked files that are not ignored (paths relative to the root).
    pub fn untracked(&self) -> Result<Vec<String>> {
        Ok(self
            .run(&["ls-files", "--others", "--exclude-standard"])?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    /// Files with added lines relative to `base`, with those line ranges.
    /// For `DiffBase::Head`, untracked files count as fully added.
    pub fn added_lines(&self, base: &DiffBase) -> Result<Vec<(String, Vec<LineRange>)>> {
        let mut out: Vec<(String, Vec<LineRange>)> = self
            .diff(base, None)?
            .into_iter()
            .filter_map(|f| {
                let path = f.new_path?;
                let ranges: Vec<LineRange> = f
                    .hunks
                    .iter()
                    .map(|h| h.new_range())
                    .filter(|r| !r.is_empty())
                    .collect();
                (!ranges.is_empty()).then_some((path, ranges))
            })
            .collect();
        if *base == DiffBase::Head {
            for path in self.untracked()? {
                out.push((path, vec![LineRange::new(1, u32::MAX)]));
            }
        }
        Ok(out)
    }

    /// Added line ranges for a single file against `base`.
    pub fn added_lines_for(&self, base: &DiffBase, path: &str) -> Result<Vec<LineRange>> {
        let ranges: Vec<LineRange> = self
            .diff(base, Some(path))?
            .into_iter()
            .flat_map(|f| f.hunks)
            .map(|h| h.new_range())
            .filter(|r| !r.is_empty())
            .collect();
        // An empty diff is either an unchanged file or a new, untracked one.
        if ranges.is_empty()
            && *base == DiffBase::Head
            && self
                .try_run(&["ls-files", "--error-unmatch", "--", path])
                .is_none()
        {
            return Ok(vec![LineRange::new(1, u32::MAX)]);
        }
        Ok(ranges)
    }

    /// Path relative to the repository root, with `/` separators, or `None` if it's outside
    /// the repository. Works for files that don't exist yet, and through symlinked paths
    /// (macOS `/tmp` → `/private/tmp`): the nearest existing ancestor is canonicalized.
    pub fn relative_in_repo(&self, path: &Path) -> Option<String> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        let canon = |p: &Path| {
            std::fs::canonicalize(p)
                .ok()
                .map(|c| PathBuf::from(normalize_path(&c.to_string_lossy())))
        };
        let root = canon(&self.root).unwrap_or_else(|| self.root.clone());
        // Walk up to an existing ancestor, canonicalize it, then re-append the rest.
        let mut existing = abs.as_path();
        let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
        let base = loop {
            if let Some(c) = canon(existing) {
                break c;
            }
            rest.push(existing.file_name()?);
            existing = existing.parent()?;
        };
        let full = rest.iter().rev().fold(base, |p, part| p.join(part));
        let rel = full.strip_prefix(&root).ok()?;
        let rel = rel.to_string_lossy().replace('\\', "/");
        (!rel.is_empty() && !rel.starts_with("../")).then_some(rel)
    }

    /// Like `relative_in_repo`, for command-line arguments: paths outside stay as given.
    pub fn relative(&self, path: &Path) -> String {
        self.relative_in_repo(path)
            .unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"))
    }
}

/// Strip the Windows verbatim prefix (`\\?\`) that git rejects.
pub fn normalize_path(path: &str) -> String {
    path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
}

fn run_in(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .context("failed to run git; is it installed?")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        run_in(dir, args).unwrap();
    }

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "T"]);
        dir
    }

    #[test]
    fn added_lines_against_head_and_untracked() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.ts"), "one\ntwo\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "init"]);
        std::fs::write(dir.path().join("a.ts"), "one\nNEW\ntwo\n").unwrap();
        std::fs::write(dir.path().join("b.ts"), "x\n").unwrap();

        let repo = Git::open(dir.path()).unwrap();
        let added = repo.added_lines_for(&DiffBase::Head, "a.ts").unwrap();
        assert_eq!(added, vec![LineRange::new(2, 2)]);
        let untracked = repo.added_lines_for(&DiffBase::Head, "b.ts").unwrap();
        assert_eq!(untracked, vec![LineRange::new(1, u32::MAX)]);
        let all = repo.added_lines(&DiffBase::Head).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn diff_ignores_user_prefix_config_and_quotes_paths() {
        let dir = init_repo();
        git(dir.path(), &["config", "diff.mnemonicPrefix", "true"]);
        std::fs::write(dir.path().join("é.ts"), "a\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "init"]);
        std::fs::write(dir.path().join("é.ts"), "a\nb\n").unwrap();
        let repo = Git::open(dir.path()).unwrap();
        let added = repo.added_lines(&DiffBase::Head).unwrap();
        assert_eq!(
            added,
            vec![("é.ts".to_string(), vec![LineRange::new(2, 2)])]
        );
    }

    #[test]
    fn renamed_file_is_not_fully_added() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.ts"), "one\ntwo\nthree\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "init"]);
        git(dir.path(), &["mv", "a.ts", "b.ts"]);
        let repo = Git::open(dir.path()).unwrap();
        assert!(repo.added_lines(&DiffBase::Staged).unwrap().is_empty());
    }

    #[test]
    fn checks_ignore_reindent_but_mining_sees_it() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.py"), "x = 1\ncall()\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "init"]);
        std::fs::write(
            dir.path().join("a.py"),
            "x = 1\ntry:\n    call()\nexcept E:\n    pass\n",
        )
        .unwrap();
        git(dir.path(), &["commit", "-qam", "wrap"]);
        let repo = Git::open(dir.path()).unwrap();
        // Mining: one hunk that replaces `call()`, so the correction is found.
        let mined = repo.diff_revs(&["HEAD~1", "HEAD"], None).unwrap();
        assert!(
            mined[0]
                .hunks
                .iter()
                .any(|h| h.removed.iter().any(|l| l.contains("call()")))
        );
        // Checks: the re-indented `call()` is not an added line.
        let added = repo
            .added_lines(&DiffBase::Range("HEAD~1..HEAD".into()))
            .unwrap();
        let lines: Vec<u32> = added[0].1.iter().flat_map(|r| r.start..=r.end).collect();
        assert!(!lines.contains(&3), "{lines:?}");
    }

    #[test]
    fn range_content_side() {
        assert_eq!(
            DiffBase::Range("main...feat".into()).content(),
            Content::Revision("feat".into())
        );
        assert_eq!(
            DiffBase::Range("main..".into()).content(),
            Content::Revision("HEAD".into())
        );
        assert_eq!(
            DiffBase::Range("main".into()).content(),
            Content::WorkingTree
        );
        assert_eq!(DiffBase::Staged.content(), Content::Index);
    }

    #[test]
    fn relative_paths_for_new_files_and_outside_paths() {
        let dir = init_repo();
        let repo = Git::open(dir.path()).unwrap();
        let new = dir.path().join("src/new/dir/file.ts");
        assert_eq!(
            repo.relative_in_repo(&new).as_deref(),
            Some("src/new/dir/file.ts")
        );
        assert_eq!(
            repo.relative_in_repo(Path::new("/definitely/elsewhere/x.md")),
            None
        );
        assert_eq!(
            repo.relative_in_repo(dir.path()),
            None,
            "the root itself is not a file"
        );
    }

    #[test]
    fn strips_verbatim_prefix() {
        assert_eq!(normalize_path(r"\\?\C:\repo"), r"C:\repo");
        assert_eq!(normalize_path("/tmp/x"), "/tmp/x");
    }
}
