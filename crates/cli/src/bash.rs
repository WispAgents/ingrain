//! Shell commands an agent runs through Claude Code's Bash tool. Edit/Write hooks never see
//! them, so `sed -i decisions/D-0001.yml` or `ingrain rules mode x shadow` would get past
//! every check. Before a command runs, ingrain asks the person when it could change a reviewed
//! file or the ledger; after it runs, the files it changed are checked like edited files.

use ingrain_core::Git;
use std::collections::BTreeMap;
use std::path::Path;

/// Files people review; agents propose instead of editing them.
pub fn is_reviewed_file(rel: &str) -> bool {
    let yaml =
        |rest: &str| !rest.contains('/') && (rest.ends_with(".yml") || rest.ends_with(".yaml"));
    rel.strip_prefix("decisions/").is_some_and(yaml)
        || rel.strip_prefix(".corrections/rules/").is_some_and(yaml)
        || rel == ".corrections/config.yml"
        || rel == ".corrections/rejected.yml"
}

/// Path fragments that point at reviewed files from a shell command.
const PROTECTED: &[&str] = &[
    "decisions/",
    "decisions\\",
    ".corrections/rules",
    ".corrections\\rules",
    ".corrections/config",
    ".corrections\\config",
    ".corrections/rejected",
    ".corrections\\rejected",
];

/// Programs that only read. A command that mentions a reviewed path is let through only when
/// every program in it is one of these and nothing is redirected into a file.
const READ_ONLY: &[&str] = &[
    "cat", "less", "more", "head", "tail", "grep", "rg", "ag", "ls", "tree", "wc", "file", "stat",
    "diff", "echo", "printf", "fd", "bat", "jq", "yq", "sort", "uniq", "cut", "true", "test", "[",
    "cd", "pwd",
];

/// `git` subcommands that only read.
const GIT_READ_ONLY: &[&str] = &[
    "diff", "log", "show", "status", "blame", "grep", "ls-files", "ls-tree", "cat-file", "add",
];

/// Why the person should approve `command` before it runs, if they should.
pub fn needs_approval(command: &str) -> Option<String> {
    // Fd duplications (`2>&1`) aren't command separators.
    let normalized = command.replace(">&", "> ");
    let segments = segments(&normalized);
    for words in &segments {
        if let Some(why) = ledger_command(words) {
            return Some(why);
        }
        if is_git(words, "commit") && words.contains(&"--no-verify") {
            return Some(
                "`git commit --no-verify` skips ingrain's pre-commit check of decisions and rules."
                    .into(),
            );
        }
    }
    let mentions = segments
        .iter()
        .flatten()
        .any(|w| PROTECTED.iter().any(|p| w.contains(p)) && !w.contains("pending"));
    if mentions && !only_reads(command, &segments) {
        return Some(
            "This command may change accepted decisions, learned rules or ingrain's config, \
             which people review. Agents propose changes with `ingrain decisions add --propose` \
             or `ingrain decisions supersede`."
                .into(),
        );
    }
    None
}

/// `ingrain` subcommands that accept, reject or retune what people review.
fn ledger_command(words: &[&str]) -> Option<String> {
    let at = words.iter().position(|w| is_program(w, "ingrain"))?;
    let rest = &words[at + 1..];
    let (group, sub) = (rest.first().copied()?, rest.get(1).copied().unwrap_or(""));
    let mutating = match group {
        "decisions" => match sub {
            "accept" | "reject" | "review" => true,
            "add" => !rest.contains(&"--propose"),
            _ => false,
        },
        "rules" => match sub {
            "accept" | "reject" | "review" | "mode" => true,
            "promote" => rest.contains(&"--apply"),
            _ => false,
        },
        _ => false,
    };
    mutating.then(|| {
        format!(
            "`ingrain {group} {sub}` changes what people reviewed; a person runs it, or approves \
             it here."
        )
    })
}

fn only_reads(command: &str, segments: &[Vec<&str>]) -> bool {
    // Any redirection into a file (`>`, `>>`, `2>`), but not `2>&1` or `>/dev/null`.
    let writes_file = command.match_indices('>').any(|(i, _)| {
        let rest = command[i + 1..].trim_start_matches('>').trim_start();
        !(rest.starts_with('&') || rest.starts_with("/dev/null"))
    });
    !writes_file
        && segments.iter().all(|words| {
            let Some(program) = words.first() else {
                return true;
            };
            if is_program(program, "git") {
                let sub = words.iter().skip(1).find(|w| !w.starts_with('-'));
                return sub.is_some_and(|s| GIT_READ_ONLY.contains(s));
            }
            if is_program(program, "ingrain") {
                return ledger_command(words).is_none();
            }
            if is_program(program, "find") {
                return !words
                    .iter()
                    .any(|w| w.starts_with("-exec") || *w == "-delete" || w.starts_with("-ok"));
            }
            if is_program(program, "sed") {
                return !words
                    .iter()
                    .any(|w| w.starts_with("-i") || *w == "--in-place");
            }
            READ_ONLY.iter().any(|p| is_program(program, p))
        })
}

/// Words of each simple command in a shell line, split on `;`, `&&`, `||`, `|`, newlines and
/// `$(`/backticks. Quotes are stripped but not interpreted; good enough to spot programs and
/// paths, and errs towards asking.
fn segments(command: &str) -> Vec<Vec<&str>> {
    command
        .split(['\n', ';', '|', '&', '`', '(', ')'])
        .map(|seg| {
            seg.split_whitespace()
                .map(|w| w.trim_matches(['"', '\'']))
                .filter(|w| !w.is_empty())
                // Leading `VAR=value` assignments and wrappers aren't the program.
                .skip_while(|w| {
                    (w.contains('=') && !w.starts_with('-'))
                        || matches!(*w, "sudo" | "env" | "command" | "exec" | "xargs" | "$")
                })
                .collect::<Vec<_>>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn is_program(word: &str, name: &str) -> bool {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    base == name || base.strip_suffix(".exe") == Some(name)
}

fn is_git(words: &[&str], sub: &str) -> bool {
    words.first().is_some_and(|w| is_program(w, "git"))
        && words.iter().skip(1).find(|w| !w.starts_with('-')) == Some(&sub)
}

/// Changed and untracked files, with a cheap stamp (size and modification time) each, so the
/// files a command changed can be told from the person's own uncommitted work.
pub fn snapshot(git: &Git) -> BTreeMap<String, String> {
    let Some(out) = git.try_run(&["status", "--porcelain=v1", "-z", "--untracked-files=all"])
    else {
        return BTreeMap::new();
    };
    let mut files = BTreeMap::new();
    let mut entries = out.split('\0');
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (status, path) = (&entry[..2], &entry[3..]);
        if status.contains('R') || status.contains('C') {
            entries.next(); // the old path
        }
        files.insert(path.to_owned(), stamp(&git.root().join(path)));
    }
    files
}

fn stamp(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(m) => {
            let modified = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            format!("{}:{modified}", m.len())
        }
        Err(_) => "deleted".into(),
    }
}

/// Files that changed between two snapshots and still exist.
pub fn changed(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<String> {
    after
        .iter()
        .filter(|(path, stamp)| *stamp != "deleted" && before.get(*path) != Some(stamp))
        .map(|(path, _)| path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_before_writing_reviewed_files() {
        for cmd in [
            "sed -i 's/x/y/' decisions/D-0001.yml",
            "echo 'status: superseded' >> decisions/D-0001.yml",
            "rm .corrections/rules/avoid-fetch.yml",
            "git mv decisions/D-0001.yml decisions/pending/",
            "python3 -c \"open('decisions/D-0001.yml','w')\"",
            "cp /tmp/x.yml .corrections/config.yml",
            "cat x > .corrections/rejected.yml",
            "ingrain rules mode avoid-fetch shadow",
            "cd sub && ingrain decisions accept P-1",
            "ingrain decisions add \"x\" --reason y",
            "ingrain rules promote --apply",
            "/usr/local/bin/ingrain rules accept x",
            "git commit -m wip --no-verify",
        ] {
            assert!(needs_approval(cmd).is_some(), "{cmd}");
        }
    }

    #[test]
    fn lets_reads_and_proposals_through() {
        for cmd in [
            "cat decisions/D-0001.yml",
            "grep -r fetch decisions/ | head -5",
            "ls .corrections/rules 2>&1",
            "git diff decisions/",
            "git log -p -- .corrections/rules/x.yml",
            "sed -n 1,5p decisions/D-0001.yml",
            "ingrain decisions add --propose \"x\" --reason y",
            "ingrain decisions list",
            "ingrain rules promote",
            "ingrain check --staged",
            "echo hi > decisions/pending/P-1.yml",
            "npm test > out.log",
            "git commit -m x",
            "cargo test 2>&1 | tail",
        ] {
            assert!(needs_approval(cmd).is_none(), "{cmd}");
        }
    }

    #[test]
    fn reviewed_files() {
        assert!(is_reviewed_file("decisions/D-0001.yml"));
        assert!(is_reviewed_file(".corrections/rules/avoid-x.yml"));
        assert!(is_reviewed_file(".corrections/config.yml"));
        assert!(is_reviewed_file(".corrections/rejected.yml"));
        assert!(!is_reviewed_file("decisions/pending/P-1.yml"));
        assert!(!is_reviewed_file(".corrections/pending/x.yml"));
        assert!(!is_reviewed_file("src/decisions/x.yml"));
    }

    #[test]
    fn changed_files_between_snapshots() {
        let before: BTreeMap<String, String> =
            [("a".into(), "1:1".into()), ("b".into(), "1:1".into())].into();
        let after: BTreeMap<String, String> = [
            ("a".into(), "1:1".into()),
            ("b".into(), "2:5".into()),
            ("c".into(), "3:3".into()),
            ("d".into(), "deleted".into()),
        ]
        .into();
        assert_eq!(changed(&before, &after), vec!["b", "c"]);
    }
}
