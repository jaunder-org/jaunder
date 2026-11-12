//! Public binary boundary: admission refusals precede Nix and leave no reports.
use std::{
    path::Path,
    process::{Command, Output},
};

use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true");
    for variable in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(variable);
    }
    let output = command.output().expect("run fixture git command");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn checkout() -> TempDir {
    let dir = tempfile::tempdir().expect("temporary checkout");
    git(dir.path(), &["init", "--quiet"]);
    std::fs::create_dir_all(dir.path().join("tools/qualification")).unwrap();
    std::fs::create_dir_all(dir.path().join("docs/adr")).unwrap();
    std::fs::write(dir.path().join("flake.nix"), "{}\n").unwrap();
    std::fs::write(
        dir.path().join("tools/qualification/Cargo.toml"),
        "[package]\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), "# Test checkout\n").unwrap();
    git(
        dir.path(),
        &[
            "add",
            "flake.nix",
            "tools/qualification/Cargo.toml",
            "README.md",
        ],
    );
    git(
        dir.path(),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    dir
}

#[test]
fn qualification_host_binds_expected_clean_head_before_nix_without_reports() {
    for product in [false, true] {
        let dir = checkout();
        let mut command = Command::new(env!("CARGO_BIN_EXE_qualification-host"));
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
        ] {
            command.env_remove(variable);
        }
        command.args([
            "--expected-harness",
            "1111111111111111111111111111111111111111",
        ]);
        if product {
            command.args(["--source", "2222222222222222222222222222222222222222"]);
        }
        let output = command.current_dir(dir.path()).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("expected harness"), "{stderr}");
        assert!(!dir.path().join(".xtask/last-result.json").exists());
        assert!(!dir.path().join(".xtask/gcroots").exists());
    }
}

#[test]
fn qualification_host_rejects_expressions_and_admits_local_pin_without_upstream() {
    let dir = checkout();
    let expected = String::from_utf8(git(dir.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    for product in [
        Some("HEAD"),
        Some("2222222222222222222222222222222222222222"),
        None,
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_qualification-host"));
        command
            .current_dir(dir.path())
            .args(["--expected-harness", expected.trim()]);
        if let Some(product) = product {
            command.args(["--source", product]);
        }
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
        ] {
            command.env_remove(variable);
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        if product == Some("HEAD") {
            assert!(stderr.contains("normalized product revision"), "{stderr}");
        } else {
            assert!(stderr.contains("Nix-built tooling provenance"), "{stderr}");
        }
        assert!(!stderr.contains("origin"), "{stderr}");
        assert!(!dir.path().join(".xtask/last-result.json").exists());
    }
}
