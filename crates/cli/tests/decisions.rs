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
    let pending =
        std::fs::read_to_string(dir.join(format!("decisions/pending/{pid}.yml"))).unwrap();
    assert!(
        pending.contains("rejected:\n- Keep the cache on SQLite."),
        "{pending}"
    );
    // Changing only the scope keeps the wording, so the decision doesn't reject itself.
    let out = ok(
        dir,
        &[
            "supersede",
            "D-0001",
            "--decision",
            "Billing code calls the payments client, never raw fetch.",
            "--reason",
            "billing moved",
            "--scope",
            "src/payments/**",
        ],
    );
    let scope_only = out.split_whitespace().nth(1).unwrap().to_owned();
    let pending =
        std::fs::read_to_string(dir.join(format!("decisions/pending/{scope_only}.yml"))).unwrap();
    assert!(!pending.contains("rejected:"), "{pending}");
    ok(dir, &["reject", &scope_only, "--reason", "test only"]);
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

    // Set up and committed, no repo hooks: the plugin answers.
    ok(dir, &["init", "--no-claude", "--no-git-hook"]);
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "ingrain"]);
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

/// An agent setting ingrain up accepts and commits on its own, with no one to approve (`-p`,
/// headless). Once that commit lands, the ledger is protected again.
#[test]
fn first_setup_lets_the_agent_accept_and_commit() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    let out = ok(
        dir,
        &[
            "add",
            "--propose",
            "No axios in src.",
            "--forbid-import",
            "axios",
        ],
    );
    let pid = out
        .split(|c: char| c.is_whitespace() || c == '`' || c == ':')
        .find(|w| w.starts_with("P-"))
        .unwrap_or_else(|| panic!("no proposal id in {out}"))
        .to_owned();
    let bash = |id: &str, command: &str| {
        json!({"session_id": "s", "cwd": dir, "tool_name": "Bash", "tool_use_id": id,
            "tool_input": {"command": command}})
    };

    // Nothing reviewed is committed yet: accept, editing, and committing go through.
    let accept = bash("a1", &format!("ingrain decisions accept {pid}"));
    assert_eq!(decision_of(hook(dir, "pre-tool-use", accept.clone())), "");
    ok(dir, &["accept", &pid]);
    assert!(hook(dir, "post-tool-use", accept).is_none());
    let edit = json!({"session_id": "s", "cwd": dir, "tool_name": "Edit",
        "tool_input": {"file_path": dir.join("decisions/D-0001.yml"),
            "old_string": "No axios", "new_string": "No axios"}});
    assert_eq!(decision_of(hook(dir, "pre-tool-use", edit.clone())), "");
    // Skipping the pre-commit check is still asked.
    let skip = bash("a2", "git commit -m x --no-verify");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", skip)), "ask");
    let commit = bash(
        "a3",
        "git add decisions .corrections .claude/settings.json && git commit -qm 'set up ingrain'",
    );
    assert_eq!(decision_of(hook(dir, "pre-tool-use", commit.clone())), "");
    git(
        dir,
        &["add", "decisions", ".corrections", ".claude/settings.json"],
    );
    git(dir, &["commit", "-qm", "set up ingrain"]);
    assert!(hook(dir, "post-tool-use", commit).is_none());

    // Committed: protected again.
    assert_eq!(decision_of(hook(dir, "pre-tool-use", edit)), "deny");
    let accept = bash("b1", "ingrain decisions accept P-1");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", accept)), "ask");
    // An orphan branch has no reviewed files in HEAD but doesn't reopen setup.
    git(dir, &["checkout", "-q", "--orphan", "fresh"]);
    let accept = bash("b2", "ingrain decisions accept P-1");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", accept)), "ask");
    git(dir, &["checkout", "-q", "main"]);

    // A person-approved accept, then a plain commit: the commit only records the approved file.
    let out = ok(
        dir,
        &[
            "add",
            "--propose",
            "No lodash in src.",
            "--forbid-import",
            "lodash",
        ],
    );
    let pid = out
        .split(|c: char| c.is_whitespace() || c == '`' || c == ':')
        .find(|w| w.starts_with("P-"))
        .unwrap()
        .to_owned();
    let accept = bash("c1", &format!("ingrain decisions accept {pid}"));
    assert_eq!(
        decision_of(hook(dir, "pre-tool-use", accept.clone())),
        "ask"
    );
    ok(dir, &["accept", &pid]);
    assert!(hook(dir, "post-tool-use", accept).is_none());
    let commit = bash("c2", "git commit -qm 'record decision'");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", commit.clone())), "");
    git(dir, &["commit", "-qm", "record decision"]);
    assert!(hook(dir, "post-tool-use", commit).is_none());
}

/// Ways back into first setup once a ledger was committed somewhere (from review).
#[test]
fn first_setup_does_not_reopen() {
    let tmp = repo();
    let dir = tmp.path();
    let before = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_owned();
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &["add", "No axios in src.", "--forbid-import", "axios"],
    );
    // Set up on a branch that isn't merged yet: main doesn't have the ledger.
    git(dir, &["checkout", "-q", "-b", "ingrain-setup"]);
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "set up ingrain"]);
    let accept = |cwd: &Path| {
        json!({"session_id": "r", "cwd": cwd, "tool_name": "Bash", "tool_use_id": "x",
            "tool_input": {"command": "ingrain decisions accept P-1"}})
    };
    git(dir, &["checkout", "-q", "main"]);
    assert_eq!(
        decision_of(hook(dir, "pre-tool-use", accept(dir))),
        "ask",
        "other branch"
    );

    // HEAD moved back to before ingrain, in a fresh clone of the refs (no marker yet).
    let clone = tempfile::tempdir().unwrap();
    git(clone.path(), &["clone", "-q", dir.to_str().unwrap(), "c"]);
    let c = &clone.path().join("c");
    git(c, &["reset", "-q", "--hard", &before]);
    assert_eq!(
        decision_of(hook(c, "pre-tool-use", accept(c))),
        "ask",
        "reset"
    );

    // A worktree on an older commit shares the refs.
    git(dir, &["worktree", "add", "-q", ".wt", &before]);
    let wt = dir.join(".wt");
    assert_eq!(
        decision_of(hook(&wt, "pre-tool-use", accept(&wt))),
        "ask",
        "worktree"
    );

    // A repository nested in one with a ledger.
    let nested = dir.join("scratch");
    std::fs::create_dir_all(&nested).unwrap();
    git(&nested, &["init", "-q"]);
    assert_eq!(
        decision_of(hook(&nested, "pre-tool-use", accept(&nested))),
        "ask",
        "nested"
    );
}

/// An edit hidden from the size and mtime stamp is still seen; a deletion after an earlier
/// approved one isn't excused by it.
#[test]
fn hidden_edits_of_reviewed_files_are_caught() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &["add", "No axios in src.", "--forbid-import", "axios"],
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "set up ingrain"]);
    let bash = |id: &str, command: &str| {
        json!({"session_id": "h", "cwd": dir, "tool_name": "Bash", "tool_use_id": id,
            "tool_input": {"command": command}})
    };
    let out = ok(
        dir,
        &[
            "add",
            "--propose",
            "No lodash in src.",
            "--forbid-import",
            "lodash",
        ],
    );
    let pid = out
        .split(|c: char| c.is_whitespace() || c == '`' || c == ':')
        .find(|w| w.starts_with("P-"))
        .unwrap()
        .to_owned();
    let accept = bash("h1", &format!("ingrain decisions accept {pid}"));
    assert_eq!(
        decision_of(hook(dir, "pre-tool-use", accept.clone())),
        "ask"
    );
    ok(dir, &["accept", &pid]);
    assert!(hook(dir, "post-tool-use", accept).is_none());

    // Same size, same mtime, different content.
    let path = dir.join("decisions/D-0002.yml");
    let text = std::fs::read_to_string(&path).unwrap();
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let sneaky = bash("h2", "f=decisions/D-0002.yml; sed -i s/lodash/LODASH/ $f");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", sneaky.clone())), "");
    std::fs::write(&path, text.replace("lodash", "LODASH")).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let after = hook(dir, "post-tool-use", sneaky).expect("blocked");
    assert_eq!(after["decision"], "block");
    std::fs::write(&path, &text).unwrap();
    git(dir, &["commit", "-qm", "record decision"]);

    // An approved `git rm`, the file restored, then a hidden `rm`: reported.
    let rm = bash("h3", "git rm -q decisions/D-0002.yml");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", rm.clone())), "ask");
    git(dir, &["rm", "-q", "decisions/D-0002.yml"]);
    assert!(hook(dir, "post-tool-use", rm).is_none());
    git(
        dir,
        &["checkout", "-q", "HEAD", "--", "decisions/D-0002.yml"],
    );
    let hidden = bash("h4", "f=decisio; rm ${f}ns/D-0002.yml");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", hidden.clone())), "");
    std::fs::remove_file(&path).unwrap();
    let after = hook(dir, "post-tool-use", hidden).expect("reported");
    assert_eq!(after["decision"], "block");
}

/// First setup covers only the repository being set up, and a ledger committed outside agent
/// sessions counts even after its branch is deleted.
#[test]
fn first_setup_stays_in_its_repo() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &["add", "No axios in src.", "--forbid-import", "axios"],
    );
    git(dir, &["checkout", "-q", "-b", "setup"]);
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "set up ingrain"]);
    git(dir, &["checkout", "-q", "main"]);
    git(dir, &["branch", "-qD", "setup"]);
    let bash = |cwd: &Path, id: &str, command: &str| {
        json!({"session_id": "o", "cwd": cwd, "tool_name": "Bash", "tool_use_id": id,
            "tool_input": {"command": command}})
    };
    let accept = bash(dir, "o1", "ingrain decisions accept P-1");
    assert_eq!(
        decision_of(hook(dir, "pre-tool-use", accept)),
        "ask",
        "deleted branch"
    );

    // A fresh repository next to one with a ledger: its own setup is open, the other isn't.
    let other = tempfile::tempdir().unwrap();
    let fresh = other.path().join("fresh");
    std::fs::create_dir_all(&fresh).unwrap();
    git(&fresh, &["init", "-q"]);
    let target = dir.join("decisions/D-0001.yml");
    for command in [
        format!("sed -i s/axios/x/ {}", target.display()),
        format!("ingrain --repo {} decisions accept P-1", dir.display()),
        "ingrain decisions accept P-1 --repo ../x".to_owned(),
    ] {
        let payload = bash(&fresh, "o2", &command);
        assert_eq!(
            decision_of(hook(&fresh, "pre-tool-use", payload)),
            "ask",
            "{command}"
        );
    }
    let own = bash(&fresh, "o3", "ingrain decisions accept P-1");
    assert_eq!(decision_of(hook(&fresh, "pre-tool-use", own)), "");

    // Setup files written with the Write tool, then committed by an alias: not reported.
    ok(&fresh, &["init", "--no-git-hook"]);
    let write = json!({"session_id": "o", "cwd": fresh, "tool_name": "Write",
        "tool_input": {"file_path": fresh.join("decisions/D-0001.yml"), "content": "id: D-0001\n"}});
    assert_eq!(decision_of(hook(&fresh, "pre-tool-use", write)), "");
    std::fs::write(fresh.join("decisions/D-0001.yml"), "id: D-0001\n").unwrap();
    git(&fresh, &["config", "alias.ci", "commit"]);
    git(&fresh, &["config", "user.email", "dev@example.com"]);
    git(&fresh, &["config", "user.name", "Dev"]);
    let ci = bash(&fresh, "o4", "git add -A && git ci -qm setup");
    assert_eq!(decision_of(hook(&fresh, "pre-tool-use", ci.clone())), "");
    git(&fresh, &["add", "-A"]);
    git(&fresh, &["ci", "-qm", "setup"]);
    assert!(hook(&fresh, "post-tool-use", ci).is_none());
}

/// A commit must hold the reviewed file as it was on disk: content staged around it (`git
/// apply --cached`) is reported. And first setup doesn't reach a nested repo's ledger.
#[test]
fn staged_tampering_and_nested_ledgers() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &["add", "No axios in src.", "--forbid-import", "axios"],
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "set up ingrain"]);
    let bash = |cwd: &Path, id: &str, command: &str| {
        json!({"session_id": "n", "cwd": cwd, "tool_name": "Bash", "tool_use_id": id,
            "tool_input": {"command": command}})
    };
    let record = std::fs::read_to_string(dir.join("decisions/D-0001.yml")).unwrap();
    let staged = record.replace("axios", "axios-allowed");
    let blob = {
        let mut child = Command::new("git")
            .args(["hash-object", "-w", "--stdin"])
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(staged.as_bytes())
            .unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned()
    };
    // The person's own edit, still uncommitted: on disk as committed, but listed by git status.
    std::fs::write(dir.join("decisions/D-0001.yml"), format!("{record}\n")).unwrap();
    let sneaky = bash(dir, "n1", "sh /tmp/stage-it.sh && git commit -qm wip");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", sneaky.clone())), "");
    git(
        dir,
        &[
            "update-index",
            "--cacheinfo",
            &format!("100644,{blob},decisions/D-0001.yml"),
        ],
    );
    git(dir, &["commit", "-qm", "wip", "--no-verify"]);
    let after = hook(dir, "post-tool-use", sneaky).expect("reported");
    assert_eq!(after["decision"], "block");
    git(dir, &["reset", "-q", "--hard", "HEAD~1"]);

    // A fresh repo with a nested clone that has a ledger.
    let outer = tempfile::tempdir().unwrap();
    let o = outer.path().join("o");
    std::fs::create_dir_all(o.join("vendor")).unwrap();
    git(&o, &["init", "-q"]);
    git(
        &o.join("vendor"),
        &["clone", "-q", dir.to_str().unwrap(), "lib"],
    );
    for command in [
        "cd vendor/lib && ingrain decisions accept P-1",
        "ingrain --repo vendor/lib decisions accept P-1",
        "sed -i s/axios/x/ vendor/lib/decisions/D-0001.yml",
    ] {
        let payload = bash(&o, "n2", command);
        assert_eq!(
            decision_of(hook(&o, "pre-tool-use", payload)),
            "ask",
            "{command}"
        );
    }
}

/// A file the person partly staged is committed as staged, not reported. A change made only
/// in the index is reported when it happens.
#[test]
fn partly_staged_files_and_index_only_changes() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &["add", "No axios in src.", "--forbid-import", "axios"],
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "set up ingrain"]);
    let bash = |id: &str, command: &str| {
        json!({"session_id": "p", "cwd": dir, "tool_name": "Bash", "tool_use_id": id,
            "tool_input": {"command": command}})
    };
    let path = dir.join("decisions/D-0001.yml");
    let record = std::fs::read_to_string(&path).unwrap();
    // The person stages one edit and keeps another unstaged.
    std::fs::write(&path, format!("{record}# a\n")).unwrap();
    git(dir, &["add", "decisions/D-0001.yml"]);
    std::fs::write(&path, format!("{record}# a\n# b\n")).unwrap();
    let commit = bash("p1", "git commit -qm 'person staged this'");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", commit.clone())), "");
    git(dir, &["commit", "-qm", "person staged this", "--no-verify"]);
    assert!(
        hook(dir, "post-tool-use", commit).is_none(),
        "partly staged"
    );
    git(dir, &["checkout", "-q", "--", "decisions/D-0001.yml"]);

    // Only the index changes, of a file the person is editing: reported right away.
    std::fs::write(&path, format!("{record}# the person's work\n")).unwrap();
    let staged = format!("{record}# sneaky\n");
    let blob = {
        let mut child = Command::new("git")
            .args(["hash-object", "-w", "--stdin"])
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(staged.as_bytes())
            .unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned()
    };
    let sneaky = bash("p2", "sh /tmp/stage.sh");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", sneaky.clone())), "");
    git(
        dir,
        &[
            "update-index",
            "--cacheinfo",
            &format!("100644,{blob},decisions/D-0001.yml"),
        ],
    );
    let after = hook(dir, "post-tool-use", sneaky).expect("reported");
    assert_eq!(after["decision"], "block");
}

/// From the last review: unstaging the person's staged decision edit isn't tampering, and a
/// snapshot written by an older ingrain (upgraded mid-command) doesn't block.
#[test]
fn unstaging_and_upgrades_dont_block() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &["add", "No axios in src.", "--forbid-import", "axios"],
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "set up ingrain"]);
    let bash = |id: &str, command: &str| {
        json!({"session_id": "u", "cwd": dir, "tool_name": "Bash", "tool_use_id": id,
            "tool_input": {"command": command}})
    };
    let path = dir.join("decisions/D-0001.yml");
    let record = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{record}# the person's edit\n")).unwrap();
    git(dir, &["add", "decisions/D-0001.yml"]);
    let reset = bash("u1", "git reset -q");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", reset.clone())), "");
    git(dir, &["reset", "-q"]);
    assert!(hook(dir, "post-tool-use", reset).is_none(), "unstaged");

    // An older ingrain's snapshot: no `staged`, stamps without hashes.
    let ls = bash("u2", "ls");
    assert_eq!(decision_of(hook(dir, "pre-tool-use", ls.clone())), "");
    let file = dir.join(".git/ingrain/sessions/u.json");
    let mut session: Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let snap = &mut session["bash"]["u2"];
    snap.as_object_mut().unwrap().remove("staged");
    for (_, stamp) in snap["files"].as_object_mut().unwrap() {
        let old = stamp
            .as_str()
            .unwrap()
            .rsplitn(2, ':')
            .last()
            .unwrap()
            .to_owned();
        *stamp = Value::String(old);
    }
    std::fs::write(&file, session.to_string()).unwrap();
    assert!(hook(dir, "post-tool-use", ls).is_none(), "legacy snapshot");
}

/// Moving to history that already exists (checkout, merge, rebase, pull, stash) brings in
/// decisions people already reviewed: not reported. Rewinding the branch, a conflict the
/// agent resolved, and a new commit dropping a decision still are.
#[test]
fn moving_through_existing_history_is_not_tampering() {
    let tmp = repo();
    let dir = tmp.path();
    // A branch from before ingrain, merged long ago: it has no decisions.
    git(dir, &["branch", "old"]);
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &["add", "No axios in src.", "--forbid-import", "axios"],
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "set up ingrain"]);
    // Elsewhere, a person changed the decision and added one.
    git(dir, &["checkout", "-qb", "team"]);
    let path = dir.join("decisions/D-0001.yml");
    let record = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        record.replace("No axios in src.", "No axios anywhere."),
    )
    .unwrap();
    ok(
        dir,
        &["add", "No lodash in src.", "--forbid-import", "lodash"],
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "team decisions"]);
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("src/ui.ts"), "export const ui = 2;\n").unwrap();
    git(dir, &["commit", "-qam", "main work"]);

    let n = std::cell::Cell::new(0);
    let run_hooked = |cwd: &Path, command: &str, args: &[&[&str]]| -> Option<Value> {
        n.set(n.get() + 1);
        let payload = json!({"session_id": "m", "cwd": cwd, "tool_name": "Bash",
            "tool_use_id": format!("m{}", n.get()), "tool_input": {"command": command}});
        assert_eq!(
            decision_of(hook(cwd, "pre-tool-use", payload.clone())),
            "",
            "{command}"
        );
        for a in args {
            git(cwd, a);
        }
        hook(cwd, "post-tool-use", payload)
    };
    let fine = |cwd: &Path, command: &str, args: &[&[&str]]| {
        assert!(run_hooked(cwd, command, args).is_none(), "{command}");
    };
    let blocked = |cwd: &Path, command: &str, args: &[&[&str]]| {
        let after = run_hooked(cwd, command, args).unwrap_or_else(|| panic!("{command}"));
        assert_eq!(after["decision"], "block", "{command}");
    };

    fine(dir, "git checkout team", &[&["checkout", "-q", "team"]]);
    fine(dir, "git checkout main", &[&["checkout", "-q", "main"]]);
    fine(dir, "git checkout old", &[&["checkout", "-q", "old"]]);
    fine(dir, "git checkout main", &[&["checkout", "-q", "main"]]);
    // A merge commit that brings in the team's decisions.
    fine(
        dir,
        "git merge --no-edit team",
        &[&["merge", "-q", "--no-edit", "team"]],
    );
    git(dir, &["reset", "-q", "--hard", "HEAD~1"]);
    // A rebase onto the team's branch.
    git(dir, &["checkout", "-qb", "feature", "main"]);
    fine(dir, "git rebase team", &[&["rebase", "-q", "team"]]);
    git(dir, &["checkout", "-q", "main"]);
    // A fast-forward.
    git(dir, &["checkout", "-qb", "ff", "main"]);
    git(dir, &["reset", "-q", "--hard", "main~1"]);
    fine(
        dir,
        "git merge --ff-only main",
        &[&["merge", "-q", "--ff-only", "main"]],
    );
    git(dir, &["checkout", "-q", "main"]);

    // A pull from a clone where the team pushed.
    let other = tempfile::tempdir().unwrap();
    git(other.path(), &["clone", "-q", dir.to_str().unwrap(), "c"]);
    let c = other.path().join("c");
    git(&c, &["config", "user.email", "dev@example.com"]);
    git(&c, &["config", "user.name", "Dev"]);
    git(&c, &["reset", "-q", "--hard", "HEAD~1"]);
    fine(&c, "git pull --ff-only", &[&["pull", "-q", "--ff-only"]]);

    // The person's dirty edit, stashed and popped.
    std::fs::write(&path, format!("{record}# the person's edit\n")).unwrap();
    fine(dir, "git stash", &[&["stash", "-q"]]);
    fine(dir, "git stash pop", &[&["stash", "pop", "-q"]]);
    git(dir, &["checkout", "-q", "--", "decisions/D-0001.yml"]);

    // Still reported: rewinding the branch past a decision.
    git(dir, &["merge", "-q", "--no-edit", "team"]);
    blocked(
        dir,
        "git reset --hard HEAD~1",
        &[&["reset", "-q", "--hard", "HEAD~1"]],
    );
    git(dir, &["reset", "-q", "--hard", "ORIG_HEAD"]);
    // A conflict the agent resolved by hand.
    git(dir, &["checkout", "-qb", "mine", "main~1"]);
    std::fs::write(&path, record.replace("No axios in src.", "Axios is fine.")).unwrap();
    git(dir, &["commit", "-qam", "mine", "--no-verify"]);
    let resolve = || {
        let out = Command::new("git")
            .args(["merge", "-q", "--no-edit", "team"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(!out.status.success(), "conflicts");
        std::fs::write(
            &path,
            record.replace("No axios in src.", "Axios when needed."),
        )
        .unwrap();
    };
    {
        n.set(n.get() + 1);
        let payload = json!({"session_id": "m", "cwd": dir, "tool_name": "Bash",
            "tool_use_id": format!("m{}", n.get()),
            "tool_input": {"command": "git merge team; git add -A; git commit --no-edit"}});
        hook(dir, "pre-tool-use", payload.clone());
        resolve();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "--no-edit", "--no-verify"]);
        let after = hook(dir, "post-tool-use", payload).expect("resolved conflict reported");
        assert_eq!(after["decision"], "block");
    }
    git(dir, &["checkout", "-q", "main"]);
    // A new commit that drops a decision, though an old branch never had it.
    blocked(
        dir,
        "f=decisions/D-0001.yml; git rm -q $f; git commit -qm x",
        &[
            &["rm", "-q", "decisions/D-0001.yml"],
            &["commit", "-qm", "x", "--no-verify"],
        ],
    );
}

/// From review of the history rule: what it must not excuse, and moves it must.
#[test]
fn history_rule_edges() {
    let tmp = repo();
    let dir = tmp.path();
    git(dir, &["branch", "old"]);
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &["add", "No axios in src.", "--forbid-import", "axios"],
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "set up ingrain"]);
    git(dir, &["tag", "-a", "-m", "v1", "v1"]);
    let path = dir.join("decisions/D-0001.yml");
    let record = std::fs::read_to_string(&path).unwrap();
    // A bare remote the team pushes to.
    let remote = tempfile::tempdir().unwrap();
    let bare = remote.path().join("r.git");
    git(remote.path(), &["init", "-q", "--bare", "r.git"]);
    git(dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(dir, &["push", "-q", "origin", "main"]);

    let n = std::cell::Cell::new(0);
    let run_hooked = |command: &str, steps: &dyn Fn()| -> Option<Value> {
        n.set(n.get() + 1);
        let payload = json!({"session_id": "e", "cwd": dir, "tool_name": "Bash",
            "tool_use_id": format!("e{}", n.get()), "tool_input": {"command": command}});
        hook(dir, "pre-tool-use", payload.clone());
        steps();
        hook(dir, "post-tool-use", payload)
    };
    let blocked = |command: &str, steps: &dyn Fn()| {
        let after = run_hooked(command, steps).unwrap_or_else(|| panic!("{command}"));
        assert_eq!(after["decision"], "block", "{command}");
    };
    let fine = |command: &str, steps: &dyn Fn()| {
        assert!(run_hooked(command, steps).is_none(), "{command}");
    };

    // New content, committed and pushed in one command.
    blocked(
        "sh edit.sh && git commit -qam x && git push -q origin main",
        &|| {
            std::fs::write(&path, record.replace("axios", "AXIOS")).unwrap();
            git(dir, &["commit", "-qam", "x", "--no-verify"]);
            git(dir, &["push", "-q", "origin", "main"]);
        },
    );
    git(dir, &["push", "-qf", "origin", "main~1:main"]);
    git(dir, &["reset", "-q", "--hard", "main~1"]);

    // The person's uncommitted edit, thrown away.
    std::fs::write(&path, format!("{record}# the person's edit\n")).unwrap();
    blocked("git reset --hard", &|| git(dir, &["reset", "-q", "--hard"]));
    std::fs::write(&path, format!("{record}# the person's edit\n")).unwrap();
    blocked("git stash && git stash drop", &|| {
        git(dir, &["stash", "-q"]);
        git(dir, &["stash", "drop", "-q"]);
    });
    // The person's new decision, untracked, cleaned away.
    std::fs::write(dir.join("decisions/D-0009.yml"), "id: D-0009\n").unwrap();
    blocked("git clean -fd", &|| git(dir, &["clean", "-qfd"]));

    // Rewinding main from a detached HEAD.
    git(dir, &["checkout", "-q", "--detach"]);
    blocked("git checkout -B main old", &|| {
        git(dir, &["checkout", "-qB", "main", "old"])
    });
    git(dir, &["reset", "-q", "--hard", "v1"]);

    // Moves that are fine: an annotated tag, a fork-style PR fetch, stash -u and pop.
    git(dir, &["checkout", "-qb", "side", "old"]);
    fine("git rebase v1", &|| git(dir, &["rebase", "-q", "v1"]));
    git(dir, &["checkout", "-q", "main"]);
    let team = tempfile::tempdir().unwrap();
    git(team.path(), &["clone", "-q", bare.to_str().unwrap(), "t"]);
    let t = team.path().join("t");
    git(&t, &["config", "user.email", "dev@example.com"]);
    git(&t, &["config", "user.name", "Dev"]);
    std::fs::write(
        t.join("decisions/D-0001.yml"),
        record.replace("in src", "in the repo"),
    )
    .unwrap();
    git(&t, &["commit", "-qam", "team"]);
    git(&t, &["push", "-q", "origin", "HEAD:refs/heads/pr"]);
    fine("git fetch origin pr:pr && git checkout pr", &|| {
        git(dir, &["fetch", "-q", "origin", "pr:pr"]);
        git(dir, &["checkout", "-q", "pr"]);
    });
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("decisions/D-0009.yml"), "id: D-0009\n").unwrap();
    fine("git stash -u", &|| git(dir, &["stash", "-qu"]));
    fine("git stash pop", &|| git(dir, &["stash", "pop", "-q"]));
}

/// From the second review of the history rule.
#[test]
fn history_rule_edges_2() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &["add", "No axios in src.", "--forbid-import", "axios"],
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "set up ingrain"]);
    let path = dir.join("decisions/D-0001.yml");
    let record = std::fs::read_to_string(&path).unwrap();
    let remote = tempfile::tempdir().unwrap();
    let bare = remote.path().join("r.git");
    git(remote.path(), &["init", "-q", "--bare", "r.git"]);
    git(dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(dir, &["push", "-q", "-u", "origin", "main"]);
    // The team moves the remote ahead (no decision change).
    let team = tempfile::tempdir().unwrap();
    git(team.path(), &["clone", "-q", bare.to_str().unwrap(), "t"]);
    let t = team.path().join("t");
    git(&t, &["config", "user.email", "dev@example.com"]);
    git(&t, &["config", "user.name", "Dev"]);
    let team_push = |name: &str| {
        std::fs::write(t.join(name), "x\n").unwrap();
        git(&t, &["add", "."]);
        git(&t, &["commit", "-qm", name]);
        git(&t, &["push", "-q", "origin", "HEAD:main"]);
    };
    team_push("a.txt");

    let n = std::cell::Cell::new(0);
    let run_hooked = |command: &str, steps: &dyn Fn()| -> Option<Value> {
        n.set(n.get() + 1);
        let payload = json!({"session_id": "f", "cwd": dir, "tool_name": "Bash",
            "tool_use_id": format!("f{}", n.get()), "tool_input": {"command": command}});
        hook(dir, "pre-tool-use", payload.clone());
        steps();
        hook(dir, "post-tool-use", payload)
    };
    let blocked = |command: &str, steps: &dyn Fn()| {
        let after = run_hooked(command, steps).unwrap_or_else(|| panic!("{command}"));
        assert_eq!(after["decision"], "block", "{command}");
    };
    let fine = |command: &str, steps: &dyn Fn()| {
        let after = run_hooked(command, steps);
        assert!(after.is_none(), "{command}: {after:?}");
    };
    let edit = || std::fs::write(&path, record.replace("axios", "AXIOS")).unwrap();

    // The agent's own edit, committed, then a pull: still the agent's edit.
    blocked(
        "sh edit.sh; git commit -qam x; git pull --no-rebase --no-edit",
        &|| {
            edit();
            git(dir, &["commit", "-qam", "x", "--no-verify"]);
            git(dir, &["pull", "-q", "--no-rebase", "--no-edit"]);
        },
    );
    git(dir, &["reset", "-q", "--hard", "origin/main"]);
    // Untracked and committed: the decision is gone from HEAD.
    blocked("sh rmc.sh; git commit -qm x", &|| {
        git(dir, &["rm", "-q", "--cached", "decisions/D-0001.yml"]);
        git(dir, &["commit", "-qm", "x", "--no-verify"]);
    });
    git(dir, &["reset", "-q", "--hard", "origin/main"]);
    // An edit laundered through the stash.
    fine("sh edit.sh && git stash", &|| {
        edit();
        git(dir, &["stash", "-q"]);
    });
    blocked("git stash pop", &|| git(dir, &["stash", "pop", "-q"]));
    git(dir, &["checkout", "-q", "--", "decisions/D-0001.yml"]);

    // The person's dirty edit, carried through an autostash pull and a stash around a pull.
    team_push("b.txt");
    std::fs::write(&path, format!("{record}# the person's edit\n")).unwrap();
    fine("git pull --rebase --autostash", &|| {
        git(dir, &["pull", "-q", "--rebase", "--autostash"]);
    });
    team_push("c.txt");
    fine("git stash && git pull && git stash pop", &|| {
        git(dir, &["stash", "-q"]);
        git(dir, &["pull", "-q", "--no-rebase"]);
        git(dir, &["stash", "pop", "-q"]);
    });
    git(dir, &["checkout", "-q", "--", "decisions/D-0001.yml"]);

    // Rewinding in two steps: moving the branch is reported when it drops decision commits.
    git(dir, &["checkout", "-q", "--detach"]);
    blocked("git branch -f main HEAD~4", &|| {
        git(dir, &["branch", "-f", "main", "HEAD~4"]);
    });
}

/// From the third review of the history rule: ordinary flows that must not block.
#[test]
fn history_rule_edges_3() {
    let tmp = repo();
    let dir = tmp.path();
    ok(dir, &["init", "--no-git-hook"]);
    ok(
        dir,
        &["add", "No axios in src.", "--forbid-import", "axios"],
    );
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "set up ingrain"]);
    let path = dir.join("decisions/D-0001.yml");
    let record = std::fs::read_to_string(&path).unwrap();
    let remote = tempfile::tempdir().unwrap();
    let bare = remote.path().join("r.git");
    git(remote.path(), &["init", "-q", "--bare", "r.git"]);
    git(dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(dir, &["push", "-q", "-u", "origin", "main"]);

    let n = std::cell::Cell::new(0);
    let run_hooked = |command: &str, steps: &dyn Fn()| -> Option<Value> {
        n.set(n.get() + 1);
        let payload = json!({"session_id": "g", "cwd": dir, "tool_name": "Bash",
            "tool_use_id": format!("g{}", n.get()), "tool_input": {"command": command}});
        hook(dir, "pre-tool-use", payload.clone());
        steps();
        hook(dir, "post-tool-use", payload)
    };
    let fine = |command: &str, steps: &dyn Fn()| {
        let after = run_hooked(command, steps);
        assert!(after.is_none(), "{command}: {after:?}");
    };
    let fails = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(!out.status.success(), "{args:?} should stop");
    };

    // The person's PR with a decision change, squash-merged upstream; their branch is rebased.
    git(dir, &["checkout", "-qb", "feature"]);
    std::fs::write(&path, record.replace("in src", "in the app")).unwrap();
    git(dir, &["commit", "-qam", "decision", "--no-verify"]);
    std::fs::write(dir.join("src/ui.ts"), "export const ui = 3;\n").unwrap();
    git(dir, &["commit", "-qam", "code", "--no-verify"]);
    git(dir, &["checkout", "-q", "main"]);
    git(dir, &["merge", "-q", "--squash", "feature"]);
    git(dir, &["commit", "-qm", "squashed PR", "--no-verify"]);
    git(dir, &["push", "-q", "origin", "main"]);
    git(dir, &["checkout", "-q", "feature"]);
    fine("git rebase origin/main", &|| {
        git(dir, &["rebase", "-q", "origin/main"])
    });
    git(dir, &["checkout", "-q", "main"]);

    // The person's stash from before the session survives the agent's own stash and pop.
    std::fs::write(
        &path,
        format!(
            "{}# the person's edit\n",
            std::fs::read_to_string(&path).unwrap()
        ),
    )
    .unwrap();
    git(dir, &["stash", "-q"]);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(dir.join("src/ui.ts"), "export const ui = 4;\n").unwrap();
    fine("git stash", &|| git(dir, &["stash", "-q"]));
    fine("git stash pop", &|| git(dir, &["stash", "pop", "-q"]));
    git(dir, &["checkout", "-q", "--", "src/ui.ts"]);
    fine("git stash pop", &|| git(dir, &["stash", "pop", "-q"]));
    git(dir, &["checkout", "-q", "--", "decisions/D-0001.yml"]);

    // A merge stopped on a code conflict, then aborted; a rebase stopped, then continued.
    git(dir, &["checkout", "-qb", "team", "main~1"]);
    std::fs::write(&path, record.replace("in src", "in team code")).unwrap();
    std::fs::write(dir.join("src/ui.ts"), "export const ui = 5;\n").unwrap();
    git(dir, &["commit", "-qam", "team", "--no-verify"]);
    git(dir, &["checkout", "-q", "main"]);
    let main_record = std::fs::read_to_string(&path).unwrap();
    let main_d = main_record.clone();
    let _ = main_d;
    // Make the decision merge cleanly: team changes only what main didn't.
    git(dir, &["checkout", "-q", "team"]);
    std::fs::write(&path, main_record.clone() + "# team note\n").unwrap();
    git(dir, &["commit", "-qam", "team note", "--no-verify"]);
    git(dir, &["checkout", "-q", "main"]);
    fails(&["merge", "-q", "--no-edit", "team"]);
    fine("git merge --abort", &|| git(dir, &["merge", "--abort"]));
    // A rebase stopped on a code conflict, then continued: it replays the person's decision.
    git(dir, &["checkout", "-qb", "work", "main"]);
    std::fs::write(dir.join("src/ui.ts"), "export const ui = 7;\n").unwrap();
    git(dir, &["commit", "-qam", "code", "--no-verify"]);
    let current = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{current}# the person's decision note\n")).unwrap();
    git(dir, &["commit", "-qam", "decision note", "--no-verify"]);
    git(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("src/ui.ts"), "export const ui = 8;\n").unwrap();
    git(dir, &["commit", "-qam", "main code", "--no-verify"]);
    git(dir, &["checkout", "-q", "work"]);
    fails(&["rebase", "-q", "main"]);
    std::fs::write(dir.join("src/ui.ts"), "export const ui = 9;\n").unwrap();
    git(dir, &["add", "src/ui.ts"]);
    fine("git rebase --continue", &|| {
        let out = Command::new("git")
            .args(["-c", "core.editor=true", "rebase", "--continue"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    });
}
