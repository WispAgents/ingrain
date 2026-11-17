//! A record's history from git: the commits that touched its file, and each commit's patch.

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub hash: String,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
    pub subject: String,
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = ingrain_core::process::command("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Newest first. Empty when the file isn't tracked (a proposal, a personal store).
pub fn log(root: &Path, file: &Path) -> Vec<Commit> {
    let rel = file.strip_prefix(root).unwrap_or(file);
    let Some(text) = git(
        root,
        &[
            "log",
            "--follow",
            "--no-color",
            "--format=%H%x1f%an%x1f%at%x1f%s",
            "--",
            &rel.to_string_lossy(),
        ],
    ) else {
        return Vec::new();
    };
    parse_log(&text)
}

pub fn parse_log(text: &str) -> Vec<Commit> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\x1f');
            Some(Commit {
                hash: f.next()?.to_owned(),
                author: f.next()?.to_owned(),
                time: f.next()?.parse().ok()?,
                subject: f.next().unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

/// The patch a commit made to `file` (or to everything when `file` is `None`).
pub fn patch(root: &Path, hash: &str, file: Option<&Path>) -> String {
    let mut args = vec!["show", "--no-color", "--format=", "--find-renames", hash];
    let rel;
    if let Some(f) = file {
        rel = f
            .strip_prefix(root)
            .unwrap_or(f)
            .to_string_lossy()
            .into_owned();
        args.extend(["--", &rel]);
    }
    git(root, &args).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_lines() {
        let c = parse_log("abc\x1fAda\x1f1700000000\x1fdocs: add D-1\nbad line\n");
        assert_eq!(
            c,
            vec![Commit {
                hash: "abc".into(),
                author: "Ada".into(),
                time: 1_700_000_000,
                subject: "docs: add D-1".into()
            }]
        );
    }
}
