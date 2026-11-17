//! The whole loop on a scripted repository: agent commits, human fix-ups,
//! mine → propose → accept → check (CLI, hooks, pre-commit).

use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const AGENT: &str = "Co-Authored-By: Claude <noreply@anthropic.com>";

/// PATH with the built binary first, so installed hooks find `ingrain`.
fn path_with_bin() -> std::ffi::OsString {
    let bin = Path::new(env!("CARGO_BIN_EXE_ingrain")).parent().unwrap();
    let rest = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(&rest)))
        .unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("PATH", path_with_bin())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn commit(dir: &Path, path: &str, content: &str, message: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, content).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", message]);
}

fn ingrain(dir: &Path, args: &[&str]) -> Output {
    ingrain_stdin(dir, args, "")
}

fn ingrain_stdin(dir: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ingrain"))
        .args(args)
        .current_dir(dir)
        .env("HOME", dir.join(".home")) // no real transcripts
        // The global store comes from HOME; a developer's own must not leak in.
        .env_remove("INGRAIN_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
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

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// A repo where an agent wrote `fetch(...)` four times and humans replaced it each time.
fn scripted_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "dev@example.com"]);
    git(dir, &["config", "user.name", "Dev"]);
    commit(
        dir,
        "src/lib/api.ts",
        "export const apiClient = { get: (u: string) => u };\n",
        "init",
    );
    for i in 0..4 {
        let path = format!("src/feature{i}.ts");
        let agent = format!(
            "import {{ apiClient }} from './lib/api';\n\nexport async function load{i}(id: string) {{\n  const res = await fetch(`/api/items/${{id}}`);\n  return res;\n}}\n\nexport const kept{i} = (x: number) => x * {i};\n"
        );
        commit(dir, &path, &agent, &format!("feat: load {i}\n\n{AGENT}"));
        let fixed = agent.replace(
            "await fetch(`/api/items/${id}`)",
            "await apiClient.get(`/items/${id}`)",
        );
        commit(dir, &path, &fixed, &format!("use apiClient in load{i}"));
    }
    tmp
}

#[test]
fn learns_a_rule_and_enforces_it_everywhere() {
    let tmp = scripted_repo();
    let dir = tmp.path();

    let out = ingrain(dir, &["init"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.join(".claude/settings.json").exists());
    assert!(dir.join(".git/hooks/pre-commit").exists());

    let out = ingrain(dir, &["rules", "mine", "--no-transcripts"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout(&out).contains("git: 4 corrections"),
        "{}",
        stdout(&out)
    );

    let out = ingrain(dir, &["rules", "propose", "--llm", "none"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout(&out).contains("avoid-fetch-call"),
        "{}",
        stdout(&out)
    );
    assert!(
        dir.join(".corrections/pending/avoid-fetch-call.yml")
            .exists()
    );

    // Proposals stay out of git; accepted rules get staged.
    let out = ingrain(dir, &["rules", "accept", "avoid-fetch-call"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let staged = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&staged.stdout).contains(".corrections/rules/avoid-fetch-call.yml")
    );
    git(dir, &["commit", "-qm", "add ingrain rule"]);

    // New agent code with the same mistake.
    let new_file = "src/feature9.ts";
    std::fs::write(
        dir.join(new_file),
        "export async function load9(id: string) {\n  return await fetch(`/api/x/${id}`);\n}\n",
    )
    .unwrap();

    // Shadow mode: logged, not shown, exit 0.
    let out = ingrain(dir, &["check"]);
    assert!(out.status.success());
    assert!(
        stdout(&out).contains("1 shadow-mode rule hits logged"),
        "{}",
        stdout(&out)
    );

    // Block mode: CLI fails and names the rule and line.
    assert!(
        ingrain(dir, &["rules", "mode", "avoid-fetch-call", "block"])
            .status
            .success()
    );
    let out = ingrain(dir, &["check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out).contains("src/feature9.ts:2  [block] avoid-fetch-call"),
        "{}",
        stdout(&out)
    );

    // GitHub annotation format.
    let out = ingrain(dir, &["check", "--format", "github"]);
    assert!(
        stdout(&out).starts_with(
            "::error file=src/feature9.ts,line=2,title=ingrain rule avoid-fetch-call::"
        )
    );

    // PostToolUse hook answers with a block decision and the past fix.
    let payload = serde_json::json!({
        "session_id": "s1",
        "cwd": dir,
        "tool_name": "Write",
        "tool_input": { "file_path": dir.join(new_file) },
    })
    .to_string();
    let out = ingrain_stdin(dir, &["hook", "post-tool-use"], &payload);
    assert!(out.status.success());
    let answer: Value = serde_json::from_str(&stdout(&out)).expect("hook prints JSON");
    assert_eq!(answer["decision"], "block");
    let reason = answer["reason"].as_str().unwrap();
    assert!(reason.contains("avoid-fetch-call at src/feature9.ts:2"));
    assert!(reason.contains("+ "), "includes the human fix: {reason}");

    // Stop hook sends the turn back up to stop_max_attempts (3) times, then lets it end.
    let stop = serde_json::json!({ "session_id": "s1", "cwd": dir, "stop_hook_active": false })
        .to_string();
    for _ in 0..3 {
        let out = ingrain_stdin(dir, &["hook", "stop"], &stop);
        let answer: Value = serde_json::from_str(&stdout(&out)).unwrap();
        assert_eq!(answer["decision"], "block");
    }
    // The 4th is let through (no block), and the person is told.
    let out = ingrain_stdin(dir, &["hook", "stop"], &stop);
    let answer: Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert!(answer.get("decision").is_none(), "4th stop is let through");
    assert!(
        answer["systemMessage"]
            .as_str()
            .unwrap()
            .contains("1 broken rules")
    );

    // An override with a reason clears it; without a reason it doesn't.
    std::fs::write(
        dir.join(new_file),
        "export async function load9(id: string) {\n  // ingrain-ignore avoid-fetch-call\n  return await fetch(`/api/x/${id}`);\n}\n",
    )
    .unwrap();
    assert_eq!(ingrain(dir, &["check"]).status.code(), Some(1));
    std::fs::write(
        dir.join(new_file),
        "export async function load9(id: string) {\n  // ingrain-ignore avoid-fetch-call: streams the body\n  return await fetch(`/api/x/${id}`);\n}\n",
    )
    .unwrap();
    let out = ingrain(dir, &["check"]);
    assert!(out.status.success(), "{}", stdout(&out));
    assert!(stdout(&out).contains("[overridden]"));

    // Pre-commit judges by the rules committed at HEAD, so a commit can't relax a rule and
    // break it: the uncommitted switch to block doesn't count yet.
    std::fs::write(dir.join("src/feature10.ts"), "const r = fetch('/a');\n").unwrap();
    git(dir, &["add", "src/feature10.ts", ".corrections/rules"]);
    assert_eq!(ingrain(dir, &["check", "--staged"]).status.code(), Some(0));
    git(dir, &["reset", "-q", "src/feature10.ts"]);
    git(dir, &["commit", "-qm", "block avoid-fetch-call"]);
    git(dir, &["add", "src/feature10.ts"]);
    assert_eq!(ingrain(dir, &["check", "--staged"]).status.code(), Some(1));
    // Staging a change that sets the rule back to shadow doesn't help.
    assert!(
        ingrain(dir, &["rules", "mode", "avoid-fetch-call", "shadow"])
            .status
            .success()
    );
    git(dir, &["add", ".corrections/rules"]);
    assert_eq!(ingrain(dir, &["check", "--staged"]).status.code(), Some(1));

    // Existing code never blocks: feature0..3 were fixed, and api.ts never used fetch.
    let out = ingrain(dir, &["check", "src/lib/api.ts"]);
    assert!(out.status.success());

    // Rules list shows hits and the override.
    let out = ingrain(dir, &["rules", "list"]);
    assert!(
        stdout(&out).contains("avoid-fetch-call"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn reject_needs_a_reason_and_is_remembered() {
    let tmp = scripted_repo();
    let dir = tmp.path();
    assert!(
        ingrain(dir, &["init", "--no-claude", "--no-git-hook"])
            .status
            .success()
    );
    assert!(
        ingrain(dir, &["rules", "mine", "--no-transcripts"])
            .status
            .success()
    );
    assert!(
        ingrain(dir, &["rules", "propose", "--llm", "none"])
            .status
            .success()
    );
    let out = ingrain(
        dir,
        &[
            "rules",
            "list",
            "reject",
            "avoid-fetch-call",
            "--reason",
            " ",
        ],
    );
    assert!(!out.status.success());
    let out = ingrain(
        dir,
        &[
            "rules",
            "reject",
            "avoid-fetch-call",
            "--reason",
            "we stream responses",
        ],
    );
    assert!(out.status.success());
    assert!(
        std::fs::read_to_string(dir.join(".corrections/rejected.yml"))
            .unwrap()
            .contains("we stream responses")
    );
    // Proposing again doesn't bring the rule back, under any id.
    let out = ingrain(dir, &["rules", "propose", "--llm", "none"]);
    let pending: Vec<String> = std::fs::read_dir(dir.join(".corrections/pending"))
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap_or_default())
        .collect();
    assert!(
        pending.iter().all(|r| !r.contains("fetch($$$ARGS)")),
        "{}",
        stdout(&out)
    );
}

#[test]
fn hook_fails_open_outside_a_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let out = ingrain_stdin(
        tmp.path(),
        &["hook", "post-tool-use"],
        "{\"tool_input\":{\"file_path\":\"/x.ts\"}}",
    );
    assert!(out.status.success());
    assert_eq!(stdout(&out), "");
}
