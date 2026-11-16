//! The whole loop on a scripted repository: init, add, hooks, check, extract (with a fake
//! `claude` on PATH), review, supersede, stale.

use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bin_dir(tmp: &Path) -> PathBuf {
    let dir = tmp.join(".bin");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn path_env(tmp: &Path) -> std::ffi::OsString {
    let own = Path::new(env!("CARGO_BIN_EXE_ingrain")).parent().unwrap();
    let rest = std::env::var_os("PATH").unwrap_or_default();
    let paths = [bin_dir(tmp), own.to_path_buf()]
        .into_iter()
        .chain(std::env::split_paths(&rest));
    std::env::join_paths(paths).unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("PATH", path_env(dir))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Ledger commands live under `ingrain decisions`; `init`, `check` and `hook` are top level.
fn prefix<'a>(args: &[&'a str]) -> Vec<&'a str> {
    match args.first() {
        Some(&("init" | "check" | "hook")) => args.to_vec(),
        _ => std::iter::once("decisions")
            .chain(args.iter().copied())
            .collect(),
    }
}

fn run(dir: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ingrain"))
        .args(prefix(args))
        .current_dir(dir)
        .env("HOME", dir.join(".home"))
        .env("PATH", path_env(dir))
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

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = run(dir, args, "");
    assert!(
        out.status.success(),
        "decisions {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn hook(dir: &Path, event: &str, payload: Value) -> Option<Value> {
    let out = run(dir, &["hook", event], &payload.to_string());
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    (!text.trim().is_empty()).then(|| serde_json::from_str(&text).expect("hook prints JSON"))
}

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "dev@example.com"]);
    git(dir, &["config", "user.name", "Dev"]);
    std::fs::create_dir_all(dir.join("src/billing")).unwrap();
    std::fs::write(
        dir.join("src/billing/charge.ts"),
        "export const charge = () => 1;\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/ui.ts"), "export const ui = 1;\n").unwrap();
    std::fs::write(dir.join("package.json"), "{\n  \"dependencies\": {}\n}\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "init"]);
    tmp
}

#[test]
fn records_inject_block_and_supersede() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init"]);
    assert!(dir.join(".claude/settings.json").exists());

    let out = ok(
        dir,
        &[
            "add",
            "Billing code calls the payments client, never raw fetch.",
            "--reason",
            "the client adds idempotency keys",
            "--scope",
            "src/billing/**",
            "--forbid-pattern",
            "TypeScript:fetch($$$A)",
            "--forbid-import",
            "axios",
        ],
    );
    assert!(out.contains("added D-0001"), "{out}");
    ok(
        dir,
        &[
            "add",
            "Keep the cache on SQLite.",
            "--forbid-dep",
            "redis",
            "--category",
            "architecture",
        ],
    );
    git(dir, &["commit", "-qm", "decisions"]);

    // Session start shows repo-wide decisions.
    let start = hook(
        dir,
        "session-start",
        json!({"session_id": "s1", "cwd": dir}),
    )
    .unwrap();
    let text = start["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(text.contains("D-0002: Keep the cache on SQLite."), "{text}");

    // An edit that breaks D-0001 is refused before it happens.
    let file = dir.join("src/billing/charge.ts");
    let bad = json!({"session_id": "s1", "cwd": dir, "tool_name": "Edit",
        "tool_input": {"file_path": file, "old_string": "=> 1", "new_string": "=> fetch('/pay')"}});
    let deny = hook(dir, "pre-tool-use", bad).unwrap();
    assert_eq!(deny["hookSpecificOutput"]["permissionDecision"], "deny");
    let reason = deny["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap();
    assert!(
        reason.contains("D-0001") && reason.contains("decisions supersede"),
        "{reason}"
    );

    // A clean edit gets the scoped decision as context, once per session.
    let good = json!({"session_id": "s1", "cwd": dir, "tool_name": "Edit",
        "tool_input": {"file_path": file, "old_string": "=> 1", "new_string": "=> payments.charge()"}});
    let ctx = hook(dir, "pre-tool-use", good.clone()).unwrap();
    assert!(
        ctx["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("D-0001")
    );
    assert!(
        hook(dir, "pre-tool-use", good).is_none(),
        "already shown this session"
    );

    // Unrelated file: no billing decision.
    let ui = json!({"session_id": "s1", "cwd": dir, "tool_name": "Write", "tool_input": {"file_path": dir.join("src/ui.ts"), "content": "export const ui = 2;\n"}});
    assert!(hook(dir, "pre-tool-use", ui).is_none());

    // A dependency the ledger forbids, in a manifest.
    let dep = json!({"session_id": "s1", "cwd": dir, "tool_name": "Edit",
        "tool_input": {"file_path": dir.join("package.json"), "old_string": "{}", "new_string": "{ \"redis\": \"^4\" }"}});
    assert_eq!(
        hook(dir, "pre-tool-use", dep).unwrap()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );

    // If the bad code lands anyway (written outside the hook), Stop sends the turn back,
    // but only for files this session edited (recorded after a successful edit).
    hook(
        dir,
        "post-tool-use",
        json!({"session_id": "s1", "cwd": dir, "tool_name": "Edit", "tool_input": {"file_path": file}}),
    );
    std::fs::write(&file, "export const charge = () => fetch('/pay');\n").unwrap();
    let stop = hook(dir, "stop", json!({"session_id": "s1", "cwd": dir})).unwrap();
    assert_eq!(stop["decision"], "block");
    assert!(hook(dir, "stop", json!({"session_id": "other", "cwd": dir})).is_none());

    // CLI check, staged check, GitHub annotations, override.
    assert_eq!(run(dir, &["check"], "").status.code(), Some(1));
    git(dir, &["add", "src/billing/charge.ts"]);
    assert_eq!(run(dir, &["check", "--staged"], "").status.code(), Some(1));
    let gh = String::from_utf8_lossy(&run(dir, &["check", "--format", "github"], "").stdout)
        .into_owned();
    assert!(
        gh.starts_with("::error file=src/billing/charge.ts,line=1,title=decision D-0001::"),
        "{gh}"
    );
    std::fs::write(&file, "// decisions-ignore D-0001: webhook replay needs raw fetch\nexport const charge = () => fetch('/pay');\n").unwrap();
    assert!(ok(dir, &["check"]).contains("[overridden]"));

    // `for` lists what applies.
    assert!(ok(dir, &["for", "src/billing/charge.ts"]).contains("D-0001"));

    // Supersede: proposal first; the old decision stays live until accepted.
    let out = ok(
        dir,
        &[
            "supersede",
            "D-0002",
            "--decision",
            "Use Redis for the cache.",
            "--reason",
            "multi-node deploys",
        ],
    );
    let pid = out.split_whitespace().nth(1).unwrap().to_owned();
    assert!(ok(dir, &["list"]).contains("D-0002"));
    ok(dir, &["accept", &pid]);
    let list = ok(dir, &["list", "--all"]);
    assert!(list.contains("D-0003: Use Redis for the cache."), "{list}");
    assert!(
        list.contains("D-0002: Keep the cache on SQLite. [superseded by D-0003]"),
        "{list}"
    );
    assert!(!ok(dir, &["list"]).contains("D-0002"));
    assert!(ok(dir, &["stale"]).contains("no stale decisions"));
}

#[test]
fn extract_proposes_and_review_remembers_rejections() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-claude", "--no-git-hook"]);
    std::fs::write(dir.join("src/ui.ts"), "export const ui = 2;\n").unwrap();
    git(
        dir,
        &[
            "commit",
            "-qam",
            "ui: render with the design-system Button\n\nWe decided all buttons come from @acme/ds; hand-rolled <button> elements drift from the theme.",
        ],
    );

    // A fake `claude` that answers every extraction call the same way: a shell script, or a
    // .cmd file on Windows (which also exercises finding `claude.cmd` on PATH).
    let reply_file = bin_dir(dir).join("reply.json");
    // The reply names the newest commit, the text extraction sends.
    let answer_for_head = || {
        let head = Command::new("git")
            .args(["log", "-1", "--format=%H"])
            .current_dir(dir)
            .output()
            .unwrap();
        let text_id = format!("c{}", &String::from_utf8_lossy(&head.stdout)[..12]);
        let reply = r#"{"results":[{"id":"ANY","records":[{"decision":"Render buttons with the @acme/ds Button, not raw <button>.","reason":"raw buttons drift from the theme","scope":["src/**"],"category":"convention"}]}]}"#;
        std::fs::write(&reply_file, reply.replace("ANY", &text_id)).unwrap();
    };
    answer_for_head();
    if cfg!(windows) {
        std::fs::write(
            bin_dir(dir).join("claude.cmd"),
            format!(
                "@echo off\r\nmore >nul\r\ntype \"{}\"\r\n",
                reply_file.display()
            ),
        )
        .unwrap();
    } else {
        let fake = bin_dir(dir).join("claude");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\ncat >/dev/null\ncat '{}'\n",
                reply_file.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    let out = ok(
        dir,
        &[
            "extract",
            "--llm",
            "claude",
            "--no-github",
            "--no-sessions",
            "--since",
            "2000-01-01",
        ],
    );
    assert!(out.contains("1 proposals"), "{out}");
    let pending: Vec<_> = std::fs::read_dir(dir.join("decisions/pending"))
        .unwrap()
        .collect();
    assert_eq!(pending.len(), 1);
    let id = pending[0]
        .as_ref()
        .unwrap()
        .path()
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let extract = |dir: &Path| {
        ok(
            dir,
            &[
                "extract",
                "--llm",
                "claude",
                "--no-github",
                "--no-sessions",
                "--since",
                "2000-01-01",
            ],
        )
    };

    // Texts already read aren't read again.
    let again = extract(dir);
    assert!(again.contains("0 new texts worth reading"), "{again}");

    // Reject needs a reason, and the decision isn't proposed again.
    assert!(
        !run(dir, &["reject", &id, "--reason", " "], "")
            .status
            .success()
    );
    ok(
        dir,
        &[
            "reject",
            &id,
            "--reason",
            "we're dropping the design system",
        ],
    );
    std::fs::write(dir.join("src/ui.ts"), "export const ui = 3;\n").unwrap();
    git(
        dir,
        &[
            "commit",
            "-qam",
            "ui: more buttons from the design system\n\nAgain: all buttons come from @acme/ds, never hand-rolled button elements.",
        ],
    );
    answer_for_head();
    let out = extract(dir);
    assert!(
        out.contains("0 proposals") && out.contains("1 duplicates"),
        "{out}"
    );
}

#[test]
fn hooks_fail_open_outside_a_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let out = run(
        tmp.path(),
        &["hook", "pre-tool-use"],
        "{\"tool_input\":{\"file_path\":\"/x.ts\"}}",
    );
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn review_regressions_symlinks_outside_edits_and_ledger_tampering() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &[
            "add",
            "No axios in src.",
            "--scope",
            "src/**",
            "--forbid-import",
            "axios",
        ],
    );
    git(dir, &["commit", "-qm", "decisions"]);

    // The temp dir is behind a symlink on macOS (/var → /private/var); a new file in a new
    // directory must still be checked.
    let new_file = dir.join("src/newdir/c.ts");
    let write = json!({"session_id": "s", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": new_file, "content": "import axios from 'axios';\n"}});
    assert_eq!(
        hook(dir, "pre-tool-use", write).unwrap()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );

    // An edit outside the repo doesn't switch off the Stop hook.
    let outside = json!({"session_id": "s2", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": "/tmp/somewhere-else/plan.md", "content": "notes"}});
    assert!(hook(dir, "pre-tool-use", outside.clone()).is_none());
    assert!(hook(dir, "post-tool-use", outside).is_none());
    let inside = json!({"session_id": "s2", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": dir.join("src/d.ts"), "content": "export const d = 1;\n"}});
    hook(dir, "pre-tool-use", inside.clone());
    hook(dir, "post-tool-use", inside);
    std::fs::write(dir.join("src/d.ts"), "import axios from 'axios';\n").unwrap();
    assert_eq!(
        hook(dir, "stop", json!({"session_id": "s2", "cwd": dir})).unwrap()["decision"],
        "block"
    );

    // Agents can't edit accepted records.
    let tamper = json!({"session_id": "s", "cwd": dir, "tool_name": "Edit",
        "tool_input": {"file_path": dir.join("decisions/D-0001.yml"), "old_string": "status: accepted", "new_string": "status: superseded"}});
    assert_eq!(
        hook(dir, "pre-tool-use", tamper).unwrap()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );

    // CI: a branch that retires the decision and breaks it in the same diff still fails,
    // because it's judged by the base revision's ledger.
    std::fs::remove_file(dir.join("src/d.ts")).unwrap();
    git(dir, &["checkout", "-q", "-b", "sneaky"]);
    let record = std::fs::read_to_string(dir.join("decisions/D-0001.yml"))
        .unwrap()
        .replace("status: accepted", "status: superseded");
    std::fs::write(dir.join("decisions/D-0001.yml"), record).unwrap();
    std::fs::write(dir.join("src/e.ts"), "import axios from 'axios';\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "retire and break"]);
    let out = run(dir, &["check", "--diff", "main...HEAD"], "");
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .contains("note: this change edits decisions/D-0001.yml")
    );
}

#[test]
fn merge_review_regressions() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1\"\n",
    )
    .unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "manifest"]);
    ok(
        dir,
        &["add", "Keep the cache on SQLite.", "--forbid-dep", "redis"],
    );
    git(dir, &["commit", "-qm", "decisions"]);

    // Pre-commit checks manifests ast-grep can't parse.
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1\"\nredis = \"0.25\"\n",
    )
    .unwrap();
    git(dir, &["add", "Cargo.toml"]);
    let out = run(dir, &["check", "--staged"], "");
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Cargo.toml"));
    git(dir, &["checkout", "-q", "HEAD", "--", "Cargo.toml"]);

    // A denied edit doesn't make the file "agent-edited": the person's own later change
    // to it doesn't send the agent's turn back.
    let denied = json!({"session_id": "s", "cwd": dir, "tool_name": "Edit",
        "tool_input": {"file_path": dir.join("Cargo.toml"), "old_string": "serde = \"1\"", "new_string": "serde = \"1\"\nredis = \"0.25\""}});
    assert_eq!(
        hook(dir, "pre-tool-use", denied).unwrap()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1\"\nredis = \"0.25\"\n",
    )
    .unwrap();
    assert!(hook(dir, "stop", json!({"session_id": "s", "cwd": dir})).is_none());

    // v0.1 settings still work (with a deprecation note) until `ingrain init` is re-run.
    let out = run(
        dir,
        &["check", "--hook", "stop"],
        &json!({"session_id": "t", "cwd": dir}).to_string(),
    );
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("deprecated"));
}

/// The plugin's hooks run in every repo: they answer only where ingrain is set up and the repo's
/// own settings don't already run it (current or v0.1 hook commands).
#[test]
fn plugin_hooks_answer_only_in_set_up_repos_without_their_own() {
    let tmp = repo();
    let dir = tmp.path();
    let edit = |path: &str| {
        json!({ "session_id": "p", "cwd": dir, "tool_name": "Write",
                "tool_input": { "file_path": dir.join(path), "content": "x: 1\n" } })
    };
    let plugin = |event: &str, payload: Value| {
        let out = run(dir, &["hook", event, "--from-plugin"], &payload.to_string());
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        (!text.trim().is_empty()).then(|| serde_json::from_str::<Value>(&text).unwrap())
    };

    // A repo that never ran `ingrain init`, with its own unrelated decisions/ folder.
    std::fs::create_dir_all(dir.join("decisions")).unwrap();
    assert_eq!(plugin("pre-tool-use", edit("decisions/0001-db.yml")), None);

    // Set up, no repo hooks: the plugin answers.
    ok(dir, &["init", "--no-claude", "--no-git-hook"]);
    let denied = plugin("pre-tool-use", edit("decisions/D-0001.yml")).expect("plugin answers");
    assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny");

    // The repo's own hooks run ingrain, current or legacy: the plugin steps aside.
    std::fs::create_dir_all(dir.join(".claude")).unwrap();
    for command in ["ingrain hook pre-tool-use", "ingrain check --hook stop"] {
        let settings = json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": command }] }] } });
        std::fs::write(dir.join(".claude/settings.json"), settings.to_string()).unwrap();
        assert_eq!(
            plugin("pre-tool-use", edit("decisions/D-0001.yml")),
            None,
            "{command}"
        );
    }
    // A permission rule naming ingrain is not a hook.
    let settings = json!({ "permissions": { "allow": ["Bash(ingrain hook stop)"] } });
    std::fs::write(dir.join(".claude/settings.json"), settings.to_string()).unwrap();
    assert!(plugin("pre-tool-use", edit("decisions/D-0001.yml")).is_some());
}

fn decision_of(answer: Option<Value>) -> String {
    answer
        .and_then(|a| {
            a["hookSpecificOutput"]["permissionDecision"]
                .as_str()
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

#[test]
fn enforcement_gaps_shell_config_overrides_and_ci() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &[
            "add",
            "No axios in src.",
            "--scope",
            "src/**",
            "--forbid-import",
            "axios",
        ],
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "decisions"]);

    // Shell commands that would change reviewed files need the person's approval.
    let bash = |id: &str, command: &str| {
        json!({"session_id": "b", "cwd": dir, "tool_name": "Bash", "tool_use_id": id,
            "tool_input": {"command": command}})
    };
    let sed = bash("t1", "sed -i 's/accepted/superseded/' decisions/D-0001.yml");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", sed)), "ask");
    let mode = bash("t2", "ingrain rules mode some-rule shadow");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", mode)), "ask");
    let cat = bash("t3", "cat decisions/D-0001.yml");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", cat.clone())), "");
    assert!(hook(dir, "post-tool-use", cat).is_none());

    // A file a shell command writes is checked after it runs, and recorded for Stop.
    std::fs::write(dir.join("src/notes.ts"), "// the person's own work\n").unwrap();
    let write = bash(
        "t4",
        "printf \"import axios from 'axios';\\n\" > src/new.ts",
    );
    assert!(hook(dir, "pre-tool-use", write.clone()).is_none());
    std::fs::write(dir.join("src/new.ts"), "import axios from 'axios';\n").unwrap();
    let after = hook(dir, "post-tool-use", write).expect("post hook answers");
    assert_eq!(after["decision"], "block");
    let reason = after["reason"].as_str().unwrap();
    assert!(reason.contains("src/new.ts"), "{reason}");
    assert!(!reason.contains("notes.ts"), "{reason}");
    let session: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(".git/ingrain/sessions/b.json")).unwrap(),
    )
    .unwrap();
    let edited: Vec<&str> = session["edited"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(edited, vec!["src/new.ts"], "only what the command changed");
    // Only the two asked commands, which never ran here, still have snapshots.
    let pending: Vec<&String> = session["bash"].as_object().unwrap().keys().collect();
    assert_eq!(pending, ["t1", "t2"]);
    std::fs::remove_file(dir.join("src/new.ts")).unwrap();
    std::fs::remove_file(dir.join("src/notes.ts")).unwrap();

    // What the pre-check can't read (a path in a variable) is caught after the command runs.
    let sneaky = bash(
        "t5",
        "D=decisions; sed -i s/accepted/superseded/ $D/D-0001.yml",
    );
    assert_eq!(decision_of(hook(dir, "pre-tool-use", sneaky.clone())), "");
    let record = std::fs::read_to_string(dir.join("decisions/D-0001.yml")).unwrap();
    std::fs::write(
        dir.join("decisions/D-0001.yml"),
        record.replace("status: accepted", "status: superseded"),
    )
    .unwrap();
    let after = hook(dir, "post-tool-use", sneaky).expect("post hook answers");
    assert_eq!(after["decision"], "block");
    assert!(
        after["reason"]
            .as_str()
            .unwrap()
            .contains("decisions/D-0001.yml")
    );
    git(dir, &["checkout", "-q", "--", "decisions/D-0001.yml"]);

    // A command that moves HEAD (revert, cherry-pick, reset) leaves no uncommitted trace.
    let revert = bash("t6", "git revert --no-edit HEAD");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", revert.clone())), "");
    git(dir, &["rm", "-q", "decisions/D-0001.yml"]);
    git(dir, &["commit", "-qm", "drop D-0001", "--no-verify"]);
    let after = hook(dir, "post-tool-use", revert).expect("post hook answers");
    assert_eq!(after["decision"], "block");
    assert!(
        after["systemMessage"]
            .as_str()
            .unwrap()
            .contains("decisions/D-0001.yml")
    );
    git(dir, &["reset", "-q", "--hard", "HEAD~1"]);

    // The config is reviewed too, and a broken one doesn't switch the hooks off.
    let config = json!({"session_id": "c", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": dir.join(".corrections/config.yml"), "content": "ignore: ['[']\n"}});
    assert_eq!(decision_of(hook(dir, "pre-tool-use", config)), "deny");
    std::fs::write(dir.join(".corrections/config.yml"), "ignore: ['[']\n").unwrap();
    let tamper = json!({"session_id": "c", "cwd": dir, "tool_name": "Edit",
        "tool_input": {"file_path": dir.join("decisions/D-0001.yml"),
            "old_string": "status: accepted", "new_string": "status: superseded"}});
    assert_eq!(decision_of(hook(dir, "pre-tool-use", tamper)), "deny");
    let via_dotdot = json!({"session_id": "c", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": dir.join("decisions/nope/../D-0001.yml"), "content": "x"}});
    assert_eq!(decision_of(hook(dir, "pre-tool-use", via_dotdot)), "deny");
    let axios = json!({"session_id": "c", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": dir.join("src/x.ts"), "content": "import axios from 'axios';\n"}});
    assert_eq!(decision_of(hook(dir, "pre-tool-use", axios)), "deny");
    git(dir, &["checkout", "-q", "--", ".corrections/config.yml"]);

    // Adding an ignore comment is an exception a person approves.
    let ignore = json!({"session_id": "c", "cwd": dir, "tool_name": "Write",
        "tool_input": {"file_path": dir.join("src/y.ts"),
            "content": "// decisions-ignore D-0001: upload needs it\nimport axios from 'axios';\n"}});
    assert_eq!(decision_of(hook(dir, "pre-tool-use", ignore)), "ask");

    // CI shows overrides on added lines, so reviewers see the exception.
    git(dir, &["checkout", "-q", "-b", "override"]);
    std::fs::write(
        dir.join("src/y.ts"),
        "// decisions-ignore D-0001: upload needs it\nimport axios from 'axios';\n",
    )
    .unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "override"]);
    let out = run(
        dir,
        &["check", "--diff", "main...HEAD", "--format", "github"],
        "",
    );
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("::warning file=src/y.ts,line=2,title=decision D-0001 overridden::"),
        "{text}"
    );
}
