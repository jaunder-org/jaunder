//! The Elisp consumer fails closed without producer evidence, mutating nothing.
#![cfg(feature = "host")]

use std::path::Path;
use std::process::{Command, Output};

fn git(dir: &Path, args: &[&str]) -> Output {
    let output = repository_policy::git::at(dir)
        .args(args)
        .env("GIT_EDITOR", "true")
        .output()
        .expect("run fixture git command");
    assert!(output.status.success(), "git {args:?}: {output:?}");
    output
}

#[test]
fn elisp_consume_fails_closed_without_producer_evidence() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "--quiet"]);
    std::fs::create_dir_all(dir.path().join("tools")).unwrap();
    std::fs::write(dir.path().join("flake.nix"), "{}\n").unwrap();
    std::fs::write(dir.path().join("tools/Cargo.toml"), "[workspace]\n").unwrap();
    git(dir.path(), &["add", "flake.nix", "tools/Cargo.toml"]);
    let index_before = git(dir.path(), &["write-tree"]).stdout;
    let mut command = Command::new(env!("CARGO_BIN_EXE_coverage-check"));
    command.arg("elisp-consume").current_dir(dir.path());
    for variable in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
    ] {
        command.env_remove(variable);
    }
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert!(!dir.path().join(".xtask/last-result.json").exists());
    assert_eq!(git(dir.path(), &["write-tree"]).stdout, index_before);
}
