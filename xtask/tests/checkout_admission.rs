//! Observe checkout admission at the actual executable's pre-lifecycle boundary.

use std::fs;
use std::process::Command;

#[test]
fn another_repository_is_refused_before_its_hook_configuration_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("another-repository");
    fs::create_dir(&root).unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "core.hooksPath", "caller-owned-hooks"],
    ] {
        assert!(xtask::git::at(&root).args(args).status().unwrap().success());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["--json", "traces", "analyze", "absent.jsonl"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let hooks = xtask::git::at(&root)
        .args(["config", "--get", "core.hooksPath"])
        .output()
        .unwrap();
    assert!(hooks.status.success());
    assert_eq!(hooks.stdout, b"caller-owned-hooks\n");
    assert!(String::from_utf8_lossy(&output.stderr).contains("Jaunder checkout marker"));
}
