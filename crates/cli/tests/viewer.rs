//! `ingrain viewer` is part of the binary. Opening a window needs a display, so this only checks
//! the command exists and refuses a folder that isn't there; the viewer's own tests render it
//! without a window (`crates/cli/src/viewer`).

use std::process::Command;

#[test]
fn the_viewer_is_a_command_of_the_binary() {
    let out = Command::new(env!("CARGO_BIN_EXE_ingrain"))
        .args(["viewer", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("--select"), "{help}");
    assert!(help.contains("--wait"), "{help}");
}

#[test]
fn a_missing_folder_is_an_error_not_a_window() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ingrain"))
        .args(["viewer", "nope"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no such folder"));
}
