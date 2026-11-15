//! Starting external programs (`git`, `gh`, `claude`, an editor) the same way on every OS.
//!
//! On Windows, `Command::new("claude")` only finds `claude.exe`. npm installs `claude.cmd`, so
//! programs are looked up on PATH with each `PATHEXT` extension and started by full path.

use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A command for `program`, found on PATH (with Windows extensions). Falls back to the bare name
/// so the spawn error names the program.
pub fn command(program: &str) -> Command {
    Command::new(find(program).map_or_else(|| program.into(), OsString::from))
}

/// The full path `program` resolves to on PATH, if any. A name with a path separator is a
/// path, not looked up.
pub fn find(program: &str) -> Option<PathBuf> {
    if program.contains('/') || program.contains('\\') {
        return None;
    }
    let paths = std::env::var_os("PATH")?;
    let exts = extensions();
    std::env::split_paths(&paths).find_map(|dir| {
        exts.iter()
            .map(|ext| dir.join(format!("{program}{ext}")))
            .find(|p| is_executable(p))
    })
}

/// Open `path` in `$VISUAL` / `$EDITOR` (notepad on Windows, vi elsewhere) and wait for it.
/// The setting may carry arguments, like `code --wait`.
pub fn edit(path: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .ok()
        .filter(|e| !e.trim().is_empty())
        .unwrap_or_else(|| if cfg!(windows) { "notepad" } else { "vi" }.into());
    let mut words = editor.split_whitespace();
    let program = words.next().unwrap_or_default();
    let status = command(program)
        .args(words)
        .arg(path)
        .status()
        .with_context(|| format!("starting {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn extensions() -> Vec<String> {
    if cfg!(windows) {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        pathext
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_lowercase)
            .collect()
    } else {
        vec![String::new()]
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_programs_on_path() {
        let git = super::find("git").expect("git is on PATH");
        assert!(git.is_file());
        assert!(super::find("surely-not-a-real-program-xyz").is_none());
        assert!(super::find("bin/git").is_none(), "paths aren't looked up");
    }
}
