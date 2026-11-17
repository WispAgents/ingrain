//! Everything that changes files goes through the `ingrain` CLI (this binary, run again), so the viewer never writes a
//! record or rule itself. Plus opening files, folders and links with the system.

use crate::viewer::bus::{Bus, Msg};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The CLI that makes changes: this very binary (`$INGRAIN_BIN` overrides it, for tests).
pub fn cli() -> Option<PathBuf> {
    std::env::var_os("INGRAIN_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| std::env::current_exe().ok())
}

/// Run `ingrain --repo <root> <args>` on a worker, streaming its output as [`Msg::Line`] when
/// `stream` is set, and send [`Msg::Done`] at the end.
pub fn run(
    bus: &Bus,
    root: Option<&Path>,
    label: impl Into<String>,
    args: Vec<String>,
    stream: bool,
) {
    let label = label.into();
    let root = root.map(Path::to_path_buf);
    bus.spawn(move |bus| {
        let (ok, output) = if stream {
            exec(bus, root.as_deref(), &args, true)
        } else {
            exec_serial(bus, root.as_deref(), &args)
        };
        Some(Msg::Done { label, ok, output })
    });
}

/// Writes run one at a time: two `accept`s at once would both take the next free `D-` id, and
/// two `reject`s would each rewrite `rejected.yml` without the other's entry.
static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// [`exec`], waiting for any other write to finish first.
pub fn exec_serial(bus: &Bus, root: Option<&Path>, args: &[String]) -> (bool, String) {
    let _turn = WRITES.lock().unwrap_or_else(|e| e.into_inner());
    exec(bus, root, args, false)
}

/// Run the CLI and wait: whether it succeeded, and its output (stdout, then stderr).
pub fn exec(bus: &Bus, root: Option<&Path>, args: &[String], stream: bool) -> (bool, String) {
    let Some(bin) = cli() else {
        return (false, "can't find the ingrain executable".into());
    };
    let mut cmd = Command::new(bin);
    if let Some(root) = root {
        cmd.arg("--repo").arg(root).current_dir(root);
    }
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (false, e.to_string()),
    };
    // stderr on its own thread so neither pipe fills and blocks the other.
    let err = child.stderr.take().map(|e| {
        let bus = bus.clone();
        std::thread::spawn(move || {
            let mut all = String::new();
            for line in BufReader::new(e).lines().map_while(Result::ok) {
                if stream {
                    bus.send(Msg::Line(line.clone()));
                }
                all.push_str(&line);
                all.push('\n');
            }
            all
        })
    });
    let mut output = String::new();
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if stream {
                bus.send(Msg::Line(line.clone()));
            }
            output.push_str(&line);
            output.push('\n');
        }
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    if let Some(e) = err.and_then(|h| h.join().ok()) {
        output.push_str(&e);
    }
    (ok, output.trim().to_owned())
}

/// The system's opener, looked up through `ingrain_core::process` (D-0009). On Windows,
/// `explorer <path>` opens a file with its default app without going through `cmd`, whose
/// parsing would run anything after an `&` in the argument.
fn system_open(args: &[&std::ffi::OsStr]) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = ingrain_core::process::command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Open a file with its default app.
pub fn open_file(path: &Path) {
    system_open(&[path.as_os_str()]);
}

/// Show a file in Finder / Explorer / the file manager.
pub fn reveal(path: &Path) {
    if cfg!(target_os = "macos") {
        system_open(&["-R".as_ref(), path.as_os_str()]);
    } else if cfg!(windows) {
        let _ = ingrain_core::process::command("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    } else if let Some(dir) = path.parent() {
        system_open(&[dir.as_os_str()]);
    }
}

/// Files a source may open with their default app: documents and code, never programs.
const READABLE: &[&str] = &[
    "md", "markdown", "txt", "rst", "adoc", "yml", "yaml", "json", "toml", "rs", "ts", "tsx", "js",
    "jsx", "py", "go", "java", "kt", "swift", "rb", "c", "h", "cc", "cpp", "hpp", "cs", "css",
    "html", "sql", "proto",
];

/// What a source opens: an http(s) URL as written, or an existing, readable file inside the
/// repository (resolved, so `../` and symlinks can't leave it). Anything else opens nothing.
/// Reads the disk: call it off the UI thread.
pub fn source_target(root: Option<&Path>, source: &str) -> Option<String> {
    let s = source.trim();
    if s.starts_with("https://") || s.starts_with("http://") {
        return Some(s.to_owned());
    }
    let root = std::fs::canonicalize(root?).ok()?;
    let rel = s.split(['#', ' ']).next()?;
    let path = std::fs::canonicalize(root.join(rel)).ok()?;
    let readable = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| READABLE.contains(&e.to_ascii_lowercase().as_str()));
    (path.starts_with(&root) && path.is_file() && readable)
        .then(|| path.to_string_lossy().into_owned())
}

/// Recent repositories, newest first, in `<global store>/viewer.json`.
pub mod recents {
    use std::path::{Path, PathBuf};

    fn file(home: Option<&Path>) -> Option<PathBuf> {
        home.map(|d| d.join("viewer.json"))
    }

    /// From the global store `home` (`ingrain_core::layout::global_dir`, or a test's).
    pub fn load(home: Option<&Path>) -> Vec<PathBuf> {
        let Some(text) = file(home).and_then(|f| std::fs::read_to_string(f).ok()) else {
            return Vec::new();
        };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        v["recent"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| p.as_str())
                    .map(PathBuf::from)
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Put `root` first and save (on a worker: the caller is the UI thread).
    pub fn push(list: &mut Vec<PathBuf>, root: &Path, home: Option<&Path>) {
        let file = file(home);
        list.retain(|p| p != root);
        list.insert(0, root.to_path_buf());
        list.truncate(12);
        let list = list.clone();
        std::thread::spawn(move || {
            let Some(f) = file else { return };
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let json = serde_json::json!({ "recent": list.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>() });
            let _ = std::fs::write(f, serde_json::to_string_pretty(&json).unwrap_or_default());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_open_as_urls_or_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("PLAN.md"), "x").unwrap();
        assert_eq!(
            source_target(None, "https://x.y/pull/1").as_deref(),
            Some("https://x.y/pull/1")
        );
        assert!(source_target(Some(dir.path()), "PLAN.md").is_some());
        assert!(source_target(Some(dir.path()), "missing.md").is_none());
        std::fs::write(dir.path().join("run.command"), "x").unwrap();
        assert!(
            source_target(Some(dir.path()), "run.command").is_none(),
            "programs never open"
        );
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert!(
            source_target(Some(&dir.path().join("sub")), "../PLAN.md").is_none(),
            "nothing outside the repo"
        );
    }
}
