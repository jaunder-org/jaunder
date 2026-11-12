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
    std::fs::create_dir_all(dir.path().join("xtask")).unwrap();
    std::fs::create_dir_all(dir.path().join("docs/adr")).unwrap();
    std::fs::write(dir.path().join("flake.nix"), "{}\n").unwrap();
    std::fs::write(dir.path().join("xtask/Cargo.toml"), "[package]\n").unwrap();
    std::fs::write(dir.path().join("README.md"), "# Test checkout\n").unwrap();
    git(
        dir.path(),
        &["add", "flake.nix", "xtask/Cargo.toml", "README.md"],
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
fn old_qualification_routes_are_retired_but_other_nix_routes_remain() {
    for args in [
        vec!["production-baseline", "discover"],
        vec!["production-baseline", "accept"],
        vec!["production-baseline", "qualify-styling"],
        vec!["nix", "production-baseline-smoke"],
        vec!["qualification-leaf"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["nix", "probe-source", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
}

fn run_leaf(dir: &Path, leaf: &str, summary: &Path) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command
        .args(["check-leaf", leaf])
        .current_dir(dir)
        .env("GITHUB_STEP_SUMMARY", summary);
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
    command.output().expect("run xtask leaf")
}

#[test]
fn migrated_policy_leaf_is_retired_without_a_compatibility_forwarder() {
    let dir = checkout();
    let summary = dir.path().join("github-summary.md");
    let output = run_leaf(dir.path(), "doc-links", &summary);

    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown check leaf"));
    assert!(output.stdout.is_empty());
    assert!(!dir.path().join(".xtask/last-result.json").exists());
    assert!(!summary.exists());
}

#[test]
fn consumer_leaves_fail_closed_without_producer_evidence_or_report_artifacts() {
    {
        let leaf = "elisp-coverage-consume";
        let dir = checkout();
        let summary = dir.path().join("github-summary.md");
        let index_before = git(dir.path(), &["write-tree"]).stdout;
        let output = run_leaf(dir.path(), leaf, &summary);
        assert_eq!(output.status.code(), Some(1), "{leaf}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("[FAIL]"));
        assert!(!dir.path().join(".xtask/last-result.json").exists());
        assert!(!summary.exists());
        assert_eq!(git(dir.path(), &["write-tree"]).stdout, index_before);
    }
}
