//! Personal stores: `--local` (this repository, inside `.git/`) and `--global` (every
//! repository). Nothing they write lands in the working tree, and hooks and checks read them
//! alongside the project's decisions.

use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn path_env() -> std::ffi::OsString {
    let own = Path::new(env!("CARGO_BIN_EXE_ingrain")).parent().unwrap();
    let rest = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(std::iter::once(own.to_path_buf()).chain(std::env::split_paths(&rest)))
        .unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("PATH", path_env())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `home` holds the global store and the Claude Code settings.
fn run(dir: &Path, home: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ingrain"))
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("INGRAIN_HOME", home.join("ingrain"))
        .env("CLAUDE_CONFIG_DIR", home.join("claude"))
        .env("PATH", path_env())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("CLAUDE_PROJECT_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn ok(dir: &Path, home: &Path, args: &[&str]) -> String {
    let out = run(dir, home, args, "");
    assert!(
        out.status.success(),
        "ingrain {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn hook(dir: &Path, home: &Path, args: &[&str], payload: Value) -> Option<Value> {
    let out = run(dir, home, args, &payload.to_string());
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    (!text.trim().is_empty()).then(|| serde_json::from_str(&text).expect("hook prints JSON"))
}

fn repo(parent: &Path, name: &str) -> std::path::PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.email", "dev@example.com"]);
    git(&dir, &["config", "user.name", "Dev"]);
    std::fs::write(dir.join("src/app.ts"), "export const app = 1;\n").unwrap();
    std::fs::write(dir.join("package.json"), "{\n  \"dependencies\": {}\n}\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-qm", "init"]);
    dir
}

fn decision(answer: &Option<Value>) -> String {
    answer
        .as_ref()
        .and_then(|a| a["hookSpecificOutput"]["permissionDecision"].as_str())
        .unwrap_or("")
        .to_owned()
}

#[test]
fn local_store_stays_out_of_the_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let dir = repo(tmp.path(), "app");

    let out = ok(&dir, &home, &["init", "--local"]);
    assert!(out.contains("local store"), "{out}");
    assert!(!dir.join("decisions").exists() && !dir.join(".corrections").exists());
    assert!(dir.join(".claude/settings.local.json").exists());
    assert_eq!(
        git(&dir, &["status", "--porcelain"]),
        "",
        "nothing to commit"
    );

    // Without a flag, a repository set up with `--local` writes to the local store.
    let out = ok(
        &dir,
        &home,
        &[
            "decisions",
            "add",
            "Use the fetch wrapper, not axios.",
            "--forbid-dep",
            "axios",
        ],
    );
    assert!(
        out.contains("added L-0001") && !out.contains("staged"),
        "{out}"
    );
    assert!(
        dir.join(".git/ingrain/local/decisions/L-0001.yml")
            .is_file()
    );
    assert_eq!(git(&dir, &["status", "--porcelain"]), "");
    let listed = ok(&dir, &home, &["decisions", "for", "package.json"]);
    assert!(listed.contains("L-0001"), "{listed}");

    // Hooks enforce it.
    let pkg = dir.join("package.json");
    let bad = json!({"session_id": "s", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": pkg, "content": "{\n  \"dependencies\": {\"axios\": \"1\"}\n}\n"}});
    assert_eq!(
        decision(&hook(&dir, &home, &["hook", "pre-tool-use"], bad)),
        "deny"
    );

    // The pre-commit check too.
    std::fs::write(&pkg, "{\n  \"dependencies\": {\"axios\": \"1\"}\n}\n").unwrap();
    git(&dir, &["add", "package.json"]);
    let check = run(&dir, &home, &["check", "--staged"], "");
    assert!(
        !check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("L-0001"));
    git(&dir, &["reset", "-q", "--hard"]);

    // Agents can't edit the local store's accepted files, with the Edit tool or the shell;
    // proposals stay free.
    let record = dir.join(".git/ingrain/local/decisions/L-0001.yml");
    let tamper = json!({"session_id": "s", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": record, "content": "x"}});
    assert_eq!(
        decision(&hook(&dir, &home, &["hook", "pre-tool-use"], tamper)),
        "deny"
    );
    let sed = json!({"session_id": "s", "cwd": dir, "tool_name": "Bash", "tool_use_id": "t1",
        "tool_input": {"command": "sed -i 's/axios/x/' .git/ingrain/local/decisions/L-0001.yml"}});
    assert_eq!(
        decision(&hook(&dir, &home, &["hook", "pre-tool-use"], sed)),
        "ask"
    );
    let accept = json!({"session_id": "s", "cwd": dir, "tool_name": "Bash", "tool_use_id": "t2",
        "tool_input": {"command": "ingrain --local decisions accept P-1"}});
    assert_eq!(
        decision(&hook(&dir, &home, &["hook", "pre-tool-use"], accept)),
        "ask"
    );
    // `..` and globs don't get around it.
    let dotted = dir.join(".git/ingrain/local/decisions/../decisions/L-0001.yml");
    let tamper = json!({"session_id": "s", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": dotted, "content": "x"}});
    assert_eq!(
        decision(&hook(&dir, &home, &["hook", "pre-tool-use"], tamper)),
        "deny"
    );
    for (i, command) in [
        "rm .git/ingr*/local/corrections/rules/x.yml",
        "echo x > $XDG_CONFIG_HOME/ingrain/corrections/config.yml",
        "rm -rf ~/.config/ingrain",
        "ingrain --local init",
    ]
    .iter()
    .enumerate()
    {
        let bash = json!({"session_id": "s", "cwd": dir, "tool_name": "Bash", "tool_use_id": format!("g{i}"),
            "tool_input": {"command": command}});
        assert_eq!(
            decision(&hook(&dir, &home, &["hook", "pre-tool-use"], bash)),
            "ask",
            "{command}"
        );
    }
    // Plain `accept` here goes to the local store, so there is no first-setup exception.
    let accept = json!({"session_id": "s", "cwd": dir, "tool_name": "Bash", "tool_use_id": "t3",
        "tool_input": {"command": "ingrain decisions accept P-1"}});
    assert_eq!(
        decision(&hook(&dir, &home, &["hook", "pre-tool-use"], accept)),
        "ask"
    );

    // A later plain `init` sets up the project store, not the local one.
    ok(&dir, &home, &["init", "--no-claude", "--no-git-hook"]);
    assert!(dir.join("decisions").is_dir() && dir.join(".corrections").is_dir());

    let pending = json!({"session_id": "s", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": dir.join(".git/ingrain/local/decisions/pending/P-1.yml"), "content": "x"}});
    assert_eq!(
        decision(&hook(&dir, &home, &["hook", "pre-tool-use"], pending)),
        ""
    );
}

#[test]
fn global_store_applies_to_every_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let a = repo(tmp.path(), "a");
    let b = repo(tmp.path(), "b");

    // Works outside a git repository, and installs your own Claude Code hooks.
    let out = ok(&home, &home, &["--global", "init"]);
    assert!(out.contains("global store"), "{out}");
    let settings = std::fs::read_to_string(home.join("claude/settings.json")).unwrap();
    assert!(
        settings.contains("ingrain hook session-start --from-plugin"),
        "{settings}"
    );

    let out = ok(
        &a,
        &home,
        &[
            "--global",
            "decisions",
            "add",
            "Prefer early returns over nested ifs.",
        ],
    );
    assert!(out.contains("added G-0001"), "{out}");
    assert!(home.join("ingrain/decisions/G-0001.yml").is_file());
    assert_eq!(git(&a, &["status", "--porcelain"]), "");

    // Repository b never ran `ingrain init`: user-level hooks (`--from-plugin`) still answer,
    // because a global decision counts as set up.
    let start = hook(
        &b,
        &home,
        &["hook", "session-start", "--from-plugin"],
        json!({"session_id": "s", "cwd": b}),
    )
    .expect("answers in every repository");
    let text = start["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("G-0001: Prefer early returns") && text.contains("personal"),
        "{text}"
    );

    // Project decisions and global ones live side by side, with their own ids.
    ok(&b, &home, &["init", "--no-claude", "--no-git-hook"]);
    let out = ok(
        &b,
        &home,
        &["decisions", "add", "Keep the cache on SQLite."],
    );
    assert!(out.contains("added D-0001"), "{out}");
    let list = ok(&b, &home, &["decisions", "list"]);
    assert!(list.contains("D-0001") && list.contains("G-0001"), "{list}");

    // Superseding a global decision proposes into the global store; accepting needs no flag.
    let out = ok(
        &b,
        &home,
        &[
            "decisions",
            "supersede",
            "G-0001",
            "--decision",
            "Prefer early returns.",
            "--reason",
            "shorter",
        ],
    );
    let proposal = out
        .split_whitespace()
        .nth(1)
        .expect("proposed <id>")
        .to_owned();
    assert!(
        home.join(format!("ingrain/decisions/pending/{proposal}.yml"))
            .is_file()
    );
    let out = ok(&b, &home, &["decisions", "accept", &proposal]);
    assert!(
        out.contains("as G-0002") && out.contains("personal"),
        "{out}"
    );
    let shown = ok(&a, &home, &["decisions", "show", "G-0001"]);
    assert!(shown.contains("superseded_by: G-0002"), "{shown}");

    // A repository that never chose ingrain keeps its own `decisions/` folder (ADRs) editable,
    // even though global decisions make the hooks answer there.
    std::fs::create_dir_all(a.join("decisions")).unwrap();
    let adr = json!({"session_id": "s", "cwd": a, "tool_name": "Write",
        "tool_input": {"file_path": a.join("decisions/0001-db.yml"), "content": "title: db\n"}});
    assert_ne!(
        decision(&hook(
            &a,
            &home,
            &["hook", "pre-tool-use", "--from-plugin"],
            adr
        )),
        "deny"
    );

    // Pointing the store elsewhere is never part of a setup command.
    let moved = json!({"session_id": "s", "cwd": a, "tool_name": "Bash", "tool_use_id": "m",
        "tool_input": {"command": "INGRAIN_HOME=/tmp/x ingrain --global decisions accept P-1"}});
    assert_eq!(
        decision(&hook(&a, &home, &["hook", "pre-tool-use"], moved)),
        "ask"
    );

    // A broken file in the global store is skipped, not fatal, in every repository.
    std::fs::write(home.join("ingrain/decisions/G-0009.yml"), "<<<<<<< ours\n").unwrap();
    std::fs::create_dir_all(home.join("ingrain/corrections/rules")).unwrap();
    std::fs::write(home.join("ingrain/corrections/rules/x.yml"), ": : :\n").unwrap();
    let check = run(&a, &home, &["check"], "");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(String::from_utf8_lossy(&check.stderr).contains("skipping"));
    let list = ok(&a, &home, &["decisions", "list"]);
    assert!(list.contains("G-0002"), "{list}");
    std::fs::remove_file(home.join("ingrain/decisions/G-0009.yml")).unwrap();
    std::fs::remove_file(home.join("ingrain/corrections/rules/x.yml")).unwrap();

    // Editing the global store from an agent is refused.
    let tamper = json!({"session_id": "s", "cwd": b, "tool_name": "Write",
        "tool_input": {"file_path": home.join("ingrain/decisions/G-0002.yml"), "content": "x"}});
    assert_eq!(
        decision(&hook(&b, &home, &["hook", "pre-tool-use"], tamper)),
        "deny"
    );
}
