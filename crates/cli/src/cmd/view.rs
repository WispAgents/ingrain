//! `ingrain view`: open the desktop viewer on a repository.
//!
//! The viewer is its own program, `ingrain-viewer` (the `viewer/` workspace), so this binary
//! stays free of GUI dependencies. It is looked up in `$INGRAIN_VIEWER`, next to this
//! executable, then on PATH, and started detached.

use super::Outcome;
use anyhow::{Context, bail};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

#[derive(clap::Args)]
pub struct Args {
    /// A folder inside the repository to open (default: the current directory, or `--repo`).
    path: Option<PathBuf>,
    /// Open on this item, e.g. `D-0004` or a rule id.
    #[arg(long, value_name = "ID")]
    select: Option<String>,
}

const NAME: &str = "ingrain-viewer";

pub fn viewer() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("INGRAIN_VIEWER") {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    let file = format!("{NAME}{}", std::env::consts::EXE_SUFFIX);
    if let Some(sibling) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(&file)))
        .filter(|p| p.is_file())
    {
        return Some(sibling);
    }
    ingrain_core::process::find(NAME)
}

pub fn run(dir: &Path, global: bool, args: Args) -> Outcome {
    let Some(bin) = viewer() else {
        bail!(
            "the viewer isn't installed. Build it with Rust 1.99 or newer:\n  \
             cargo install --locked --git https://github.com/WispAgents/ingrain ingrain-viewer"
        );
    };
    let mut cmd = std::process::Command::new(&bin);
    // Rune reads its own flags first; the app's come after `--`.
    cmd.arg("--");
    let target = args.path.as_deref().unwrap_or(dir);
    let target = std::fs::canonicalize(target)
        .with_context(|| format!("no such folder: {}", target.display()))?;
    cmd.arg("--repo").arg(strip_verbatim(target));
    if global {
        // Inside a repository: only the personal rules, still changeable through the CLI.
        // Outside one: the global store, read-only.
        cmd.arg("--personal");
    }
    if let Some(id) = &args.select {
        cmd.arg(format!("--select={id}"));
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()
        .with_context(|| format!("starting {}", bin.display()))?;
    println!("opened the viewer");
    Ok(ExitCode::SUCCESS)
}

/// `\\?\C:\x` → `C:\x` and `\\?\UNC\srv\x` → `\\srv\x`: Windows' canonical paths are
/// verbatim, which git rejects.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let Some(s) = p.to_str() else { return p };
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_paths_lose_their_prefix() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\x")),
            PathBuf::from(r"C:\x")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\UNC\srv\share")),
            PathBuf::from(r"\\srv\share")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from("/tmp/x")),
            PathBuf::from("/tmp/x")
        );
    }
}
