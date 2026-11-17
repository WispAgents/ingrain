//! `ingrain viewer`: open the desktop viewer on a repository.
//!
//! By default it starts itself again in the background (`viewer --wait`) and returns, so the
//! terminal is free, like `code .`. `--wait` runs the window in this process until it closes.

use super::Outcome;
use anyhow::Context;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

#[derive(clap::Args)]
pub struct Args {
    /// A folder inside the repository to open (default: the current directory, or `--repo`).
    path: Option<PathBuf>,
    /// Open on this item, e.g. `D-0004` or a rule id.
    #[arg(long, value_name = "ID")]
    select: Option<String>,
    /// Keep the terminal until the window closes.
    #[arg(long)]
    wait: bool,
    /// Start on "Applies to" with this path.
    #[arg(long, hide = true, value_name = "PATH")]
    applies: Option<String>,
    /// Rune's own flags for headless runs (`-- --snap out.png --scene timeline`); implies `--wait`.
    #[arg(last = true, hide = true)]
    rune: Vec<String>,
}

/// Which personal store `--local` / `--global` picked, if any: the viewer then shows only it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Only {
    Local,
    Global,
}

impl Only {
    fn flag(self) -> &'static str {
        match self {
            Only::Local => "--local",
            Only::Global => "--global",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Only::Local => "local",
            Only::Global => "global",
        }
    }
}

pub fn run(dir: &Path, only: Option<Only>, args: Args) -> Outcome {
    let target = args.path.as_deref().unwrap_or(dir);
    let target = std::fs::canonicalize(target)
        .with_context(|| format!("no such folder: {}", target.display()))?;
    let target = strip_verbatim(target);
    if !cfg!(feature = "viewer") {
        anyhow::bail!(
            "this build of ingrain has no viewer (Linux release builds leave it out so ingrain runs \
             on any machine). On a Linux desktop, build it with the viewer: install \
             libxkbcommon-dev and libwayland-dev, then\n  \
             cargo install --locked --git https://github.com/WispAgents/ingrain ingrain"
        );
    }
    if args.wait || !args.rune.is_empty() {
        #[cfg(feature = "viewer")]
        crate::viewer::run(app_args(&target, only, &args), args.rune)?;
        return Ok(ExitCode::SUCCESS);
    }
    let exe = std::env::current_exe().context("finding the ingrain executable")?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(child_args(&target, only, &args))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut cmd);
    cmd.spawn().context("starting the viewer")?;
    println!("opened the viewer");
    Ok(ExitCode::SUCCESS)
}

/// The window must outlive this command and the terminal it came from: its own process group
/// (a runner that kills the group when a command ends leaves it alone), and on Windows no console.
fn detach(cmd: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
}

/// What the window opens, in the viewer's own argument form.
#[cfg_attr(not(feature = "viewer"), allow(dead_code))]
fn app_args(target: &Path, only: Option<Only>, args: &Args) -> Vec<String> {
    let mut out = vec!["--repo".to_owned(), target.to_string_lossy().into_owned()];
    if let Some(o) = only {
        // Only that store. Outside a repository the global store opens read-only.
        out.push(format!("--store={}", o.name()));
    }
    if let Some(id) = &args.select {
        out.push(format!("--select={id}"));
    }
    if let Some(p) = &args.applies {
        out.push(format!("--applies={p}"));
    }
    out
}

/// The command line of the background copy: the same request, with `--wait`.
fn child_args(target: &Path, only: Option<Only>, args: &Args) -> Vec<OsString> {
    let mut out: Vec<OsString> = Vec::new();
    if let Some(o) = only {
        out.push(o.flag().into());
    }
    out.extend([
        "viewer".into(),
        target.as_os_str().to_owned(),
        "--wait".into(),
    ]);
    if let Some(id) = &args.select {
        out.extend(["--select".into(), id.into()]);
    }
    if let Some(p) = &args.applies {
        out.extend(["--applies".into(), p.into()]);
    }
    out
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

    fn args(select: Option<&str>) -> Args {
        Args {
            path: None,
            select: select.map(str::to_owned),
            wait: false,
            applies: None,
            rune: vec![],
        }
    }

    #[test]
    fn the_background_copy_asks_for_the_same_thing_and_waits() {
        let t = Path::new("/r");
        assert_eq!(
            child_args(t, None, &args(Some("D-0004"))),
            ["viewer", "/r", "--wait", "--select", "D-0004"].map(OsString::from)
        );
        let mut a = args(None);
        a.applies = Some("src/a.rs".into());
        assert_eq!(
            child_args(t, Some(Only::Local), &a),
            ["--local", "viewer", "/r", "--wait", "--applies", "src/a.rs"].map(OsString::from)
        );
        assert_eq!(
            app_args(t, Some(Only::Global), &args(Some("G-0001"))),
            ["--repo", "/r", "--store=global", "--select=G-0001"]
        );
    }

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
