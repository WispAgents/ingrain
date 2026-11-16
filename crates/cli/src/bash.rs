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
        if let Some(why) = skips_hooks(seg) {
            return Some(why);
        }
    }
    // A command substitution inside quotes isn't split out; don't trust it near reviewed paths.
    let lower = command.to_lowercase();
    let substitution = (command.contains("$(") || command.contains('`'))
        && (lower.contains("decisions") || lower.contains(".corrections"));
    let mut in_protected_dir = false;
    let mut mentions = false;
    let mut all_read = true;
    for seg in &segments {
        // A segment can be only a redirect (`> decisions/D-0001.yml`).
        let program = seg.words.first().map_or("", |w| base_name(w));
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
        all_read &= !writes_protected && (program.is_empty() || reads_only(program, &seg.words));
    }
    (mentions && (!all_read || substitution)).then(|| {
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

/// `git commit` without its pre-commit hook: `--no-verify` (or a prefix git accepts, like
/// `--no-ver`), `-n` in a flag cluster, or hooks pointed elsewhere (`core.hooksPath` via `-c`,
/// `--config-env` or `GIT_CONFIG_*`).
fn skips_hooks(seg: &Segment) -> Option<String> {
    let words = &seg.words;
    if !words.first().is_some_and(|w| base_name(w) == "git") {
        return None;
    }
    let config_hooks = words
        .iter()
        .chain(&seg.env)
        .any(|w| w.to_lowercase().contains("hookspath"))
        || seg.env.iter().any(|w| w.starts_with("GIT_CONFIG_"))
        || words.iter().any(|w| w.starts_with("--config-env"));
    let (sub, rest) = git_subcommand(words);
    let values = commit_values(rest);
    let no_verify = sub == Some("commit")
        && rest.iter().enumerate().any(|(i, w)| {
            if values.contains(&i) {
                return false;
            }
            if let Some(long) = w.strip_prefix("--") {
                return long.len() >= 4 && "no-verify".starts_with(long);
            }
            w.strip_prefix('-')
                .is_some_and(|flags| short_flags(flags).contains('n'))
        });
    (no_verify || config_hooks).then(|| {
        "This skips ingrain's pre-commit check of decisions and rules (`--no-verify`, `-n` or \
         `core.hooksPath`)."
            .to_owned()
    })
}

/// `git commit` short options that take a value; after one, the rest of the cluster (or the
/// next word) is that value.
const COMMIT_VALUE_FLAGS: &[char] = &['m', 'F', 'c', 'C', 't', 'u', 'S'];

/// The flags of a short-option cluster up to the first value-taking one, inclusive.
fn short_flags(cluster: &str) -> &str {
    match cluster.find(COMMIT_VALUE_FLAGS) {
        Some(i) => &cluster[..=i],
        None => cluster,
    }
}

/// Indexes (into `rest`, the words after `commit`) of option values: messages, files, keys.
fn commit_values(rest: &[String]) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, w) in rest.iter().enumerate() {
        if out.contains(&i) {
            continue;
        }
        if let Some(long) = w.strip_prefix("--") {
            if matches!(
                long,
                "message"
                    | "file"
                    | "template"
                    | "reuse-message"
                    | "reedit-message"
                    | "author"
                    | "date"
                    | "fixup"
                    | "squash"
                    | "trailer"
            ) {
                out.push(i + 1);
            }
            continue;
        }
        if let Some(flags) = w.strip_prefix('-') {
            let head = short_flags(flags);
            // `-am` + next word; `-m"msg"` carries its value.
            if head.ends_with(COMMIT_VALUE_FLAGS) && head.len() == flags.len() {
                out.push(i + 1);
            }
        }
    }
    out
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

/// Indexes of `git commit` option values (messages), whose text isn't a path.
fn commit_message_values(words: &[String]) -> Vec<usize> {
    if words.is_empty() || base_name(&words[0]) != "git" {
        return Vec::new();
    }
    let (sub, rest) = git_subcommand(words);
    if sub != Some("commit") {
        return Vec::new();
    }
    let offset = words.len() - rest.len();
    let mut out: Vec<usize> = commit_values(rest)
        .into_iter()
        .map(|i| i + offset)
        .collect();
    // Attached values: `--message=...`, `-m"..."`.
    for (i, w) in rest.iter().enumerate() {
        let attached = w.starts_with("--message=")
            || w.strip_prefix('-')
                .is_some_and(|f| !f.starts_with('-') && short_flags(f).len() < f.len());
        if attached {
            out.push(i + offset);
        }
    }
    out
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
    /// Leading `VAR=value` assignments and wrappers, removed from `words`.
    env: Vec<String>,
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
    let mut heredoc: Option<String> = None;
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
            '<' => {
                finish_word(&mut segments, &mut word, &mut has_word, &mut redirect_next);
                if chars.peek() == Some(&'<') {
                    chars.next();
                    if chars.peek() == Some(&'<') {
                        chars.next(); // here-string: the word that follows is data
                        continue;
                    }
                    if chars.peek() == Some(&'-') {
                        chars.next();
                    }
                    while chars
                        .peek()
                        .is_some_and(|c| c.is_whitespace() && *c != '\n')
                    {
                        chars.next();
                    }
                    let mut delim = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_whitespace() || matches!(c, ';' | '&' | '|' | '>' | '<' | '(' | ')')
                        {
                            break;
                        }
                        chars.next();
                        if !matches!(c, '\'' | '"' | '\\') {
                            delim.push(c);
                        }
                    }
                    if !delim.is_empty() {
                        heredoc = Some(delim);
                    }
                }
            }
            ';' | '&' | '|' | '\n' | '(' | ')' | '`' => {
                finish_word(&mut segments, &mut word, &mut has_word, &mut redirect_next);
                redirect_next = false;
                // A heredoc body is data, not commands: skip to its closing line.
                if c == '\n'
                    && let Some(delim) = heredoc.take()
                {
                    let mut line = String::new();
                    for h in chars.by_ref() {
                        if h == '\n' {
                            if line.trim() == delim {
                                break;
                            }
                            line.clear();
                        } else {
                            line.push(h);
                        }
                    }
                }
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
        seg.env = seg.words.drain(..lead).collect();
    }
    segments.retain(|s| !s.words.is_empty() || !s.redirects.is_empty() || !s.env.is_empty());
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
            // The old path of a rename is gone from the tree.
            if let Some(old) = entries.next()
                && status.contains('R')
            {
                files.insert(old.to_owned(), "deleted".to_owned());
            }
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

/// Files that changed between two snapshots: new or different stamps, deleted ones, and ones
/// that dropped out of `git status` (stashed, reset, checked out, committed).
pub fn changed(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<String> {
    let mut out: Vec<String> = after
        .iter()
        .filter(|(path, stamp)| before.get(*path) != Some(stamp))
        .map(|(path, _)| path.clone())
        .collect();
    out.extend(before.keys().filter(|p| !after.contains_key(*p)).cloned());
    out.sort();
    out
}

/// The commit `HEAD` points at.
pub fn head(git: &Git) -> Option<String> {
    git.try_run(&["rev-parse", "--verify", "-q", "HEAD"])
        .map(|s| s.trim().to_owned())
}

/// Files commits between `from` and `to` touched, when a command moved `HEAD` (commit,
/// revert, cherry-pick, reset, pull…).
pub fn moved(git: &Git, from: &str, to: &str) -> Vec<String> {
    git.try_run(&["diff", "--name-only", "-z", "--no-renames", from, to, "--"])
        .map(|out| {
            out.split('\0')
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
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
            // Second review.
            "git commit --no-veri -m x",
            "X=/dev/null git --config-env=core.hooksPath=X commit -m x",
            "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/dev/null git commit -m x",
            "> decisions/D-0001.yml",
            "true; > decisions/D-0001.yml",
            "echo \"$(rm decisions/D-0001.yml)\"",
            "cat <<'EOF' > decisions/D-0001.yml\nstatus: superseded\nEOF",
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
            // Second review.
            "git commit -am 'explain decisions/ layout'",
            "git commit -qm 'see decisions/'",
            "git commit --message='decisions/ docs'",
            "git commit -m\"fix decisions/ docs\"",
            "git commit -uno -m x",
            "cat > /tmp/notes <<'EOF'\nsee decisions/D-0001.yml and rm it\nEOF",
            "cat decisions/D-0001.yml 2>/dev/null",
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
        // Files that dropped out of `git status` (stashed, reset) changed too.
        let cleaned: BTreeMap<String, String> = [("a".into(), "1:1".into())].into();
        assert_eq!(changed(&before, &cleaned), vec!["b"]);
    }
}
