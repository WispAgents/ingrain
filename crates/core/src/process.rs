//! Starting external programs (`git`, `gh`, `claude`, an editor) the same way on every OS.
//!
//! On Windows, `Command::new("claude")` only finds `claude.exe`. npm installs `claude.cmd`, so
//! programs are looked up on PATH with each `PATHEXT` extension and started by full path.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

/// A command for `program`, found on PATH (with Windows extensions). Falls back to the bare name
/// so the spawn error names the program.
pub fn command(program: &str) -> Command {
    Command::new(find(program).map_or_else(|| program.into(), OsString::from))
}

/// The full path `program` resolves to on PATH, if any.
pub fn find(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    let exts = extensions();
    std::env::split_paths(&paths).find_map(|dir| {
        exts.iter()
            .map(|ext| dir.join(format!("{program}{ext}")))
            .find(|p| p.is_file())
    })
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
    }
}
