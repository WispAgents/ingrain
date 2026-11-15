use crate::cmd::Outcome;
use crate::repo::Repo;
use anyhow::{Context, Result, bail};
use ingrain_core::RuleFile;
use ingrain_engine::CompiledRule;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::ExitCode;

/// An entry in `.corrections/rejected.yml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rejected {
    pub id: String,
    pub reason: String,
    /// The rule body, so a near-identical proposal can be recognized later.
    pub rule: String,
}

pub fn rejected(repo: &Repo) -> Result<Vec<Rejected>> {
    let path = repo.layout.rejected_file();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)?;
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_yaml::from_str(&text).with_context(|| format!("invalid {}", path.display()))
}

pub fn accept(dir: &Path, ids: &[String]) -> Outcome {
    let repo = Repo::open(dir)?;
    for id in ids {
        let path = accept_one(&repo, id)?;
        println!("accepted {id} → {} (mode: shadow)", path.display());
    }
    println!("commit .corrections/rules/ to share the rules with your team and CI");
    Ok(ExitCode::SUCCESS)
}

fn accept_one(repo: &Repo, id: &str) -> Result<std::path::PathBuf> {
    let mut rule = repo.find_pending(id)?;
    CompiledRule::compile(&rule.to_yaml())
        .with_context(|| format!("{id} doesn't compile; edit it first"))?;
    let pending_path = rule.path.clone();
    let dest = repo.layout.rule_path(&repo.layout.rules_dir(), id)?;
    if dest.exists() {
        bail!("an accepted rule `{id}` already exists");
    }
    rule.save(&dest)?;
    std::fs::remove_file(&pending_path)?;
    let rel = repo.git.relative(&dest);
    repo.git.run(&["add", "--", &rel])?;
    Ok(dest)
}

pub fn reject(dir: &Path, id: &str, reason: &str) -> Outcome {
    let repo = Repo::open(dir)?;
    reject_one(&repo, id, reason)?;
    println!("rejected {id}; it won't be proposed again");
    Ok(ExitCode::SUCCESS)
}

fn reject_one(repo: &Repo, id: &str, reason: &str) -> Result<()> {
    if reason.trim().is_empty() {
        bail!("a reason is required");
    }
    let rule = repo.find_pending(id)?;
    let mut all = rejected(repo)?;
    all.push(Rejected {
        id: id.to_owned(),
        reason: reason.trim().to_owned(),
        rule: rule.to_yaml(),
    });
    std::fs::write(repo.layout.rejected_file(), serde_yaml::to_string(&all)?)?;
    std::fs::remove_file(&rule.path)?;
    Ok(())
}

pub fn interactive(dir: &Path) -> Outcome {
    let repo = Repo::open(dir)?;
    let pending = repo.pending()?;
    if pending.is_empty() {
        println!("nothing to review; run `ingrain rules propose` first");
        return Ok(ExitCode::SUCCESS);
    }
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    for (i, rule) in pending.iter().enumerate() {
        println!("\n[{}/{}] {}", i + 1, pending.len(), describe(rule));
        // The id can change when the rule is edited; the file follows it.
        let mut id = rule.id.clone();
        loop {
            print!("[a]ccept  [e]dit  [r]eject  [s]kip  [q]uit > ");
            std::io::stdout().flush()?;
            let Some(answer) = read_line(&mut input)? else {
                return Ok(ExitCode::SUCCESS);
            };
            match answer.as_str() {
                "a" => match accept_one(&repo, &id) {
                    Ok(_) => {
                        println!("accepted (shadow mode)");
                        break;
                    }
                    Err(e) => println!("{e:#}"),
                },
                "e" => match edit_pending(&repo, &id) {
                    Ok(new_id) => {
                        if new_id != id {
                            println!("renamed to {new_id}");
                        }
                        println!("saved; the rule compiles");
                        id = new_id;
                    }
                    Err(e) => println!("{e:#}"),
                },
                "r" => {
                    print!("reason > ");
                    std::io::stdout().flush()?;
                    let reason = read_line(&mut input)?.unwrap_or_default();
                    match reject_one(&repo, &id, &reason) {
                        Ok(()) => {
                            println!("rejected");
                            break;
                        }
                        Err(e) => println!("{e:#}"),
                    }
                }
                "s" => break,
                "q" => return Ok(ExitCode::SUCCESS),
                _ => {}
            }
        }
    }
    println!("\ncommit .corrections/ to share accepted rules");
    Ok(ExitCode::SUCCESS)
}

/// Open the pending rule in an editor, then check it compiles. If the edit changed
/// the `id:`, move the file so its name keeps matching. Returns the (possibly new) id.
fn edit_pending(repo: &Repo, id: &str) -> Result<String> {
    let path = repo.layout.rule_path(&repo.layout.pending_dir(), id)?;
    edit(&path)?;
    let mut rule = RuleFile::load(&path)?;
    CompiledRule::compile(&rule.to_yaml()).context("the edited rule doesn't compile")?;
    if rule.id != id {
        let dest = repo
            .layout
            .rule_path(&repo.layout.pending_dir(), &rule.id)?;
        if dest.exists() {
            bail!("a pending rule `{}` already exists", rule.id);
        }
        rule.save(&dest)?;
        std::fs::remove_file(&path)?;
    }
    Ok(rule.id)
}

fn read_line(input: &mut impl BufRead) -> Result<Option<String>> {
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_owned()))
}

fn edit(path: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| if cfg!(windows) { "notepad" } else { "vi" }.into());
    let status = ingrain_core::process::command(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("starting {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }
    Ok(())
}

/// The review card for one rule.
pub fn describe(rule: &RuleFile) -> String {
    let m = &rule.meta;
    let mut out = format!(
        "{}  ({} · {} · mode {})\n  {}\n",
        rule.id, rule.language, m.origin, m.mode, rule.message
    );
    if let Some(b) = &m.backtest {
        out.push_str(&format!(
            "  backtest: catches {}/{} corrections · flags {}/{} fixed versions · {} false alarms in {} agent hunks · {} hits in current code\n",
            b.before_hits, b.befores, b.after_hits, b.afters, b.kept_hits, b.kept, b.head_hits
        ));
    }
    out.push_str(&format!("  evidence ({} fixes):\n", m.support));
    for e in &m.evidence {
        out.push_str(&format!("    · {e}\n"));
    }
    if let Some(ex) = &m.example {
        out.push_str("  example:\n");
        ex.before
            .lines()
            .for_each(|l| out.push_str(&format!("    - {l}\n")));
        ex.after
            .lines()
            .for_each(|l| out.push_str(&format!("    + {l}\n")));
    }
    out.push_str("  rule:\n");
    for l in rule_body(rule).lines() {
        out.push_str(&format!("    {l}\n"));
    }
    out
}

fn rule_body(rule: &RuleFile) -> String {
    let yaml = rule.to_yaml();
    let doc: serde_yaml::Mapping = serde_yaml::from_str(&yaml).unwrap_or_default();
    doc.get("rule")
        .map(|r| serde_yaml::to_string(r).unwrap_or_default())
        .unwrap_or_default()
}
