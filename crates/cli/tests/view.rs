//! `ingrain view` starts the viewer program with the repository, and says how to get it when
//! it isn't installed.

use std::path::Path;
use std::process::Command;

fn ingrain(dir: &Path, viewer: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ingrain"))
        .args(args)
        .current_dir(dir)
        .env("INGRAIN_VIEWER", viewer)
        .output()
        .unwrap()
}

#[cfg(unix)]
#[test]
fn starts_the_viewer_on_the_repository() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    let log = dir.path().join("args.txt");
    let fake = dir.path().join("fake-viewer");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    let out = ingrain(&repo, &fake, &["view", "src", "--select", "D-0004"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut args = String::new();
    for _ in 0..100 {
        args = std::fs::read_to_string(&log).unwrap_or_default();
        if args.ends_with("--select=D-0004\n") {
            break; // written in full
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let src = std::fs::canonicalize(repo.join("src")).unwrap();
    assert_eq!(
        args,
        format!("--\n--repo\n{}\n--select=D-0004\n", src.display())
    );
}

#[test]
fn says_how_to_install_a_missing_viewer() {
    let dir = tempfile::tempdir().unwrap();
    let out = ingrain(dir.path(), &dir.path().join("nope"), &["view"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("cargo install --locked --git"), "{err}");
}
