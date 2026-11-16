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

/// Programs that only read. A command that mentions a reviewed path is let through only when
/// every program in it is one of these (without their writing options) and nothing is
/// redirected into a reviewed path.
const READ_ONLY: &[&str] = &[
    "cat", "less", "more", "head", "tail", "grep", "egrep", "rg", "ag", "ls", "wc", "file", "stat",
    "diff", "echo", "printf", "bat", "jq", "cut", "true", "test", "[", "cd", "pushd", "popd",
    "pwd", "awk", "find", "sed",
];

/// Options that make a "read-only" program write files.
fn writes_with(program: &str, words: &[String]) -> bool {
    let short_has = |w: &str, c: char| w.starts_with('-') && !w.starts_with("--") && w.contains(c);
    words.iter().skip(1).any(|w| match program {
        "sed" => short_has(w, 'i') || w.starts_with("--in-place"),
        "awk" => w == "inplace" || w.contains("inplace"),
        "find" => {
            w.starts_with("-exec")
                || w.starts_with("-ok")
                || w == "-delete"
                || w.starts_with("-fprint")
                || w == "-fls"
        }
        _ => false,
    })
}

/// `git` subcommands that only read.
const GIT_READ_ONLY: &[&str] = &[
    "diff",
    "log",
    "show",
    "status",
    "blame",
    "grep",
    "ls-files",
    "ls-tree",
    "cat-file",
    "add",
    "rev-parse",
];

/// Why the person should approve `command` before it runs, if they should.
pub fn needs_approval(command: &str) -> Option<String> {
    let segments = lex(command);
    for seg in &segments {
        let words: Vec<&str> = seg.words.iter().map(String::as_str).collect();
        if let Some(why) = ledger_command(&words) {
            return Some(why);
        }
        if let Some(why) = skips_hooks(&words) {
            return Some(why);
        }
    }
    let mut in_protected_dir = false;
    let mut mentions = false;
    let mut all_read = true;
    for seg in &segments {
        let Some(program) = seg.words.first() else {
            continue;
        };
        let program = base_name(program);
        let skip = commit_message_values(&seg.words);
        let named = seg
            .words
            .iter()
            .enumerate()
            .filter(|(i, _)| !skip.contains(i))
            .any(|(_, w)| is_protected(w));
        if matches!(program, "cd" | "pushd") && named {
            in_protected_dir = true;
        }
        let writes_protected = seg
            .redirects
            .iter()
            .any(|t| is_protected(t) || (in_protected_dir && !t.starts_with('/')));
        mentions |= named || writes_protected || in_protected_dir;
        all_read &= !writes_protected && reads_only(program, &seg.words);
    }
    (mentions && !all_read).then(|| {
        "This command may change accepted decisions, learned rules or ingrain's config, which \
         people review. Agents propose changes with `ingrain decisions add --propose` or \
         `ingrain decisions supersede`."
            .to_owned()
    })
}

/// A word that names `decisions/` or `.corrections/` outside their proposal folders.
fn is_protected(word: &str) -> bool {
    let w = word.to_lowercase().replace('\\', "/");
    if w.contains("decisions/") || w.contains(".corrections") {
        // Only `pending/` proposals are free, and only without `..` tricks.
        let free = !w.contains("..")
            && (w.contains("decisions/pending") || w.contains(".corrections/pending"))
            && !w.contains(' ');
        return !free;
    }
    // `rm -rf decisions`, `cd decisions`.
    w.trim_end_matches('/').rsplit('/').next() == Some("decisions")
}

fn reads_only(program: &str, words: &[String]) -> bool {
    if program == "git" {
        let (sub, rest) = git_subcommand(words);
        return sub.is_some_and(|s| GIT_READ_ONLY.contains(&s))
            && !rest.iter().any(|w| w.starts_with("--output"));
    }
    if program == "ingrain" {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        return ledger_command(&words).is_none();
    }
    READ_ONLY.contains(&program) && !writes_with(program, words)
}

/// `git commit` without its pre-commit hook: `--no-verify`, `-n`, or hooks pointed elsewhere.
fn skips_hooks(words: &[&str]) -> Option<String> {
    if !words.first().is_some_and(|w| base_name(w) == "git") {
        return None;
    }
    let owned: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
    let config_hooks = words
        .iter()
        .any(|w| w.to_lowercase().starts_with("core.hookspath"));
    let (sub, rest) = git_subcommand(&owned);
    // `-m <msg>` / `-am <msg>` / `-F <file>`: the next word is a value, not an option.
    let takes_value = |w: &str| {
        matches!(w, "--message" | "--file")
            || (w.starts_with('-')
                && !w.starts_with("--")
                && (w.ends_with('m') || w.ends_with('F')))
    };
    let no_verify = sub == Some("commit")
        && rest.iter().enumerate().any(|(i, w)| {
            let is_value = i > 0 && takes_value(&rest[i - 1]);
            !is_value
                && (w == "--no-verify"
                    || (w.starts_with('-') && !w.starts_with("--") && w[1..].contains('n')))
        });
    (no_verify || config_hooks).then(|| {
        "This skips ingrain's pre-commit check of decisions and rules (`--no-verify`, `-n` or \
         `core.hooksPath`)."
            .to_owned()
    })
}

/// The git subcommand and the words after it, skipping global options (`-C dir`, `-c k=v`).
fn git_subcommand(words: &[String]) -> (Option<&str>, &[String]) {
    let mut i = 1;
    while i < words.len() {
        let w = words[i].as_str();
        if matches!(w, "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace") {
            i += 2;
        } else if w.starts_with('-') {
            i += 1;
        } else {
            return (Some(w), &words[i + 1..]);
        }
    }
    (None, &[])
}

/// Indexes of `git commit -m <msg>` values, whose text isn't a path.
fn commit_message_values(words: &[String]) -> Vec<usize> {
    if base_name(&words[0]) != "git" || git_subcommand(words).0 != Some("commit") {
        return Vec::new();
    }
    words
        .iter()
        .enumerate()
        .filter(|(_, w)| matches!(w.as_str(), "-m" | "--message"))
        .map(|(i, _)| i + 1)
        .collect()
}

/// `ingrain` subcommands that accept, reject or retune what people review.
fn ledger_command(words: &[&str]) -> Option<String> {
    let at = words.iter().position(|w| base_name(w) == "ingrain")?;
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

/// One simple command: its words (quotes removed) and the targets it redirects output into.
#[derive(Debug, Default)]
struct Segment {
    words: Vec<String>,
    redirects: Vec<String>,
}

/// Split a shell line into simple commands on `;`, `&`, `|`, newlines, `(`, `)` and backticks,
/// outside quotes. Quotes and backslashes are honoured; variables and globs are not expanded
/// (the PostToolUse check catches what this can't see). Leading `VAR=x`, `sudo`, `env`,
/// `command`, `exec`, `xargs`, `time`, `nohup` are dropped so the program comes first.
fn lex(command: &str) -> Vec<Segment> {
    let mut segments = vec![Segment::default()];
    let mut word = String::new();
    let mut has_word = false;
    let mut redirect_next = false;
    let mut chars = command.chars().peekable();
    let finish_word = |segments: &mut Vec<Segment>,
                       word: &mut String,
                       has_word: &mut bool,
                       redirect: &mut bool| {
        if *has_word {
            let seg = segments.last_mut().expect("one segment");
            if *redirect {
                seg.redirects.push(std::mem::take(word));
                *redirect = false;
            } else {
                seg.words.push(std::mem::take(word));
            }
        }
        word.clear();
        *has_word = false;
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                has_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                has_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                word.push(n);
                            }
                        }
                        _ => word.push(q),
                    }
                }
            }
            '\\' => {
                has_word = true;
                if let Some(n) = chars.next() {
                    word.push(n);
                }
            }
            '>' => {
                // `2>` / `&>`: the fd number isn't a word.
                if word.chars().all(|c| c.is_ascii_digit()) {
                    word.clear();
                    has_word = false;
                }
                finish_word(&mut segments, &mut word, &mut has_word, &mut redirect_next);
                while matches!(chars.peek(), Some('>' | '|')) {
                    chars.next();
                }
                if chars.peek() == Some(&'&') {
                    chars.next();
                    // `>&2` duplicates a descriptor; `>&file` writes a file.
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                    if chars
                        .peek()
                        .is_some_and(|c| c.is_ascii_digit() || *c == '-')
                    {
                        while chars
                            .peek()
                            .is_some_and(|c| c.is_ascii_digit() || *c == '-')
                        {
                            chars.next();
                        }
                        continue;
                    }
                }
                redirect_next = true;
            }
            '&' if chars.peek() == Some(&'>') => {
                finish_word(&mut segments, &mut word, &mut has_word, &mut redirect_next);
            }
            '<' => finish_word(&mut segments, &mut word, &mut has_word, &mut redirect_next),
            ';' | '&' | '|' | '\n' | '(' | ')' | '`' => {
                finish_word(&mut segments, &mut word, &mut has_word, &mut redirect_next);
                redirect_next = false;
                segments.push(Segment::default());
            }
            c if c.is_whitespace() => {
                finish_word(&mut segments, &mut word, &mut has_word, &mut redirect_next)
            }
            c => {
                has_word = true;
                word.push(c);
            }
        }
    }
    finish_word(&mut segments, &mut word, &mut has_word, &mut redirect_next);
    for seg in &mut segments {
        let lead = seg
            .words
            .iter()
            .take_while(|w| {
                (w.contains('=') && !w.starts_with('-'))
                    || matches!(
                        w.as_str(),
                        "sudo" | "env" | "command" | "exec" | "xargs" | "time" | "nohup" | "$"
                    )
            })
            .count();
        seg.words.drain(..lead);
    }
    segments.retain(|s| !s.words.is_empty() || !s.redirects.is_empty());
    segments
}

fn base_name(word: &str) -> &str {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    base.strip_suffix(".exe").unwrap_or(base)
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

/// Files that changed between two snapshots, deleted ones included.
pub fn changed(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<String> {
    after
        .iter()
        .filter(|(path, stamp)| before.get(*path) != Some(stamp))
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
            // From review: paths split across words, redirects, write flags, hook skips.
            "cd decisions && rm D-0001.yml",
            "rm -rf decisions",
            "rm -rf .corrections",
            "rm .corrections/*",
            "rm \"decisions\"/D-0001.yml",
            "rm Decisions/D-0001.yml",
            "echo x >& decisions/D-0001.yml",
            "echo x &> decisions/D-0001.yml",
            "sed -Ei 's/a/b/' decisions/D-0001.yml",
            "sed --in-place=.bak 's/a/b/' decisions/D-0001.yml",
            "yq -i '.status = \"superseded\"' decisions/D-0001.yml",
            "sort -o decisions/D-0001.yml decisions/D-0001.yml",
            "find decisions -fprint decisions/x.yml",
            "git diff --output=decisions/D-0001.yml",
            "rm decisions/pending/../D-0001.yml",
            "git checkout -- decisions/",
            "git restore .corrections/config.yml",
            "tee decisions/D-0001.yml < /tmp/x",
            "perl -pi -e s/a/b/ decisions/D-0001.yml",
            "awk -i inplace '{print}' decisions/D-0001.yml",
            "git commit -n -m x",
            "git commit -anm x",
            "git -c core.hooksPath=/dev/null commit -m x",
            "ls decisions | xargs rm",
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
            // From review: harmless commands that used to ask.
            "git commit -m 'docs: explain decisions/ layout'",
            "grep -rn x decisions/ > /tmp/out.txt",
            "awk '/id:/' decisions/D-0001.yml",
            "git log --format='%an <%ae>' -- decisions/",
            "grep -E 'a->b' decisions/",
            "git -C . log decisions/",
            "cat decisions/D-0001.yml >&2",
            "git commit -am 'fix: x'",
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
        assert_eq!(changed(&before, &after), vec!["b", "c", "d"]);
    }
}
