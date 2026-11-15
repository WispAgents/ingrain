//! Model backends used to write rules. `ingrain check` never calls these.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

/// Default model for rule writing.
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

pub trait Llm: Send + Sync {
    fn complete(&self, prompt: &str) -> Result<String>;
    fn describe(&self) -> String;
}

/// Claude Code in headless mode (`claude -p`), using the user's existing login.
/// Runs with no tools, in an empty temp directory so project hooks don't fire.
pub struct ClaudeCli {
    pub model: String,
}

impl Llm for ClaudeCli {
    fn complete(&self, prompt: &str) -> Result<String> {
        // A fresh empty directory: no project CLAUDE.md, settings or hooks get loaded.
        let dir = tempfile::tempdir().context("creating a temp dir for claude")?;
        let mut child = ingrain_core::process::command("claude")
            .args([
                "-p",
                "--tools",
                "",
                "--no-session-persistence",
                "--output-format",
                "text",
                "--model",
            ])
            .arg(&self.model)
            .current_dir(dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to start `claude`; is Claude Code installed?")?;
        child
            .stdin
            .take()
            .context("no stdin for claude")?
            .write_all(prompt.as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!(
                "claude -p failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    fn describe(&self) -> String {
        format!("claude CLI ({})", self.model)
    }
}

/// The Anthropic Messages API over HTTPS, keyed by `ANTHROPIC_API_KEY`.
pub struct AnthropicApi {
    key: String,
    pub model: String,
    agent: ureq::Agent,
}

impl AnthropicApi {
    pub fn new(key: String, model: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(600)))
            .http_status_as_error(false)
            .build()
            .into();
        Self { key, model, agent }
    }
}

impl Llm for AnthropicApi {
    fn complete(&self, prompt: &str) -> Result<String> {
        let body = json!({
            "model": self.model,
            "max_tokens": 16000,
            // Route refusals to a fallback model server-side instead of failing the rule.
            "fallbacks": "default",
            "messages": [{"role": "user", "content": prompt}],
        });
        let mut resp = self
            .agent
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2026-10-21")
            .header("anthropic-beta", "server-side-fallback-2026-07-01")
            .header("content-type", "application/json")
            .send_json(&body)
            .context("calling the Anthropic API")?;
        let status = resp.status();
        let value: Value = resp
            .body_mut()
            .read_json()
            .context("reading API response")?;
        if !status.is_success() {
            bail!(
                "Anthropic API returned {status}: {}",
                value["error"]["message"]
            );
        }
        if value["stop_reason"] == "refusal" {
            bail!("the model declined to write this rule");
        }
        Ok(value["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join(""))
    }
    fn describe(&self) -> String {
        format!("Anthropic API ({})", self.model)
    }
}

/// Pick a backend: `claude`, `api`, `none` or `auto` (API if a key is set, else the CLI if installed).
pub fn backend(name: &str, model: Option<&str>) -> Result<Option<Box<dyn Llm>>> {
    let model = model.unwrap_or(DEFAULT_MODEL).to_owned();
    let api = || -> Result<Box<dyn Llm>> {
        let key = std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY is not set")?;
        Ok(Box::new(AnthropicApi::new(key, model.clone())))
    };
    match name {
        "none" => Ok(None),
        "claude" => Ok(Some(Box::new(ClaudeCli { model }))),
        "api" => api().map(Some),
        "auto" => {
            if std::env::var_os("ANTHROPIC_API_KEY").is_some() {
                api().map(Some)
            } else if which("claude") {
                Ok(Some(Box::new(ClaudeCli { model })))
            } else {
                Ok(None)
            }
        }
        other => bail!("unknown LLM backend `{other}` (expected auto, claude, api or none)"),
    }
}

fn which(bin: &str) -> bool {
    ingrain_core::process::find(bin).is_some()
}
