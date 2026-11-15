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

impl Git {
    /// Open the repository containing `dir`.
    pub fn open(dir: &Path) -> Result<Self> {
        let out = run_in(dir, &["rev-parse", "--show-toplevel"])
            .with_context(|| format!("{} is not inside a git repository", dir.display()))?;
        Ok(Self {
            root: PathBuf::from(normalize_path(out.trim())),
        })
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
    pub fn git_dir(&self) -> Result<PathBuf> {
        let out = self.run(&["rev-parse", "--absolute-git-dir"])?;
        Ok(PathBuf::from(normalize_path(out.trim())))
    }

    pub fn has_head(&self) -> bool {
        self.try_run(&["rev-parse", "--verify", "-q", "HEAD"])
            .is_some()
    }

    /// File content at a revision, or `None` if the path doesn't exist there.
    pub fn show(&self, rev: &str, path: &str) -> Option<String> {
        self.try_run(&["show", &format!("{rev}:{path}")])
    }

    /// Diff for one file or the whole tree, parsed.
    pub fn diff(&self, base: &DiffBase, path: Option<&str>) -> Result<Vec<FileDiff>> {
        let mut args: Vec<String> = vec![
            "diff".into(),
            "--no-color".into(),
            "--no-ext-diff".into(),
            "-U0".into(),
            "--no-renames".into(),
        ];
        match base {
            DiffBase::Head => {
                if self.has_head() {
                    args.push("HEAD".into());
                } else {
                    // No commits yet: compare the index against the empty tree.
                    args.push("--cached".into());
                }
            }
            DiffBase::Staged => args.push("--cached".into()),
            DiffBase::Range(range) => args.push(range.clone()),
        }
        args.push("--".into());
        if let Some(p) = path {
            args.push(p.into());
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        Ok(diff::parse(&self.run(&refs)?))
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
        if *base == DiffBase::Head {
            let tracked = self
                .try_run(&["ls-files", "--error-unmatch", "--", path])
                .is_some();
            if !tracked {
                return Ok(vec![LineRange::new(1, u32::MAX)]);
            }
        }
        Ok(self
            .diff(base, Some(path))?
            .into_iter()
            .flat_map(|f| f.hunks)
            .map(|h| h.new_range())
            .filter(|r| !r.is_empty())
            .collect())
    }

    /// Path relative to the repository root, with `/` separators.
    pub fn relative(&self, path: &Path) -> String {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|d| d.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        };
        let abs = PathBuf::from(normalize_path(&abs.to_string_lossy()));
        let root = std::fs::canonicalize(&self.root)
            .map(|p| PathBuf::from(normalize_path(&p.to_string_lossy())))
            .unwrap_or_else(|_| self.root.clone());
        let abs = std::fs::canonicalize(&abs)
            .map(|p| PathBuf::from(normalize_path(&p.to_string_lossy())))
            .unwrap_or(abs);
        abs.strip_prefix(&root)
            .unwrap_or(&abs)
            .to_string_lossy()
            .replace('\\', "/")
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
    fn strips_verbatim_prefix() {
        assert_eq!(normalize_path(r"\\?\C:\repo"), r"C:\repo");
        assert_eq!(normalize_path("/tmp/x"), "/tmp/x");
    }
}
