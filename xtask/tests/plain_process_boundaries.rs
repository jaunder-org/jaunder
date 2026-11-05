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
fn check_leaf_process_emits_plain_diagnostic_without_result_artifacts() {
    let dir = checkout();
    let summary = dir.path().join("github-summary.md");
    let output = run_leaf(dir.path(), "doc-links", &summary);

    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("[ ok ] doc-links"), "{stdout}");
    assert!(!stdout.contains("xtask PASSED"), "{stdout}");
    assert!(!dir.path().join(".xtask/last-result.json").exists());
    assert!(!summary.exists());
}

#[test]
fn consumer_leaves_fail_closed_without_producer_evidence_or_report_artifacts() {
    for leaf in ["coverage-consume", "elisp-coverage-consume"] {
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

#[test]
fn clean_admission_leaf_is_plain_and_refuses_untracked_state() {
    let dir = checkout();
    let summary = dir.path().join("github-summary.md");
    assert!(
        run_leaf(dir.path(), "clean-tree", &summary)
            .status
            .success()
    );
    std::fs::write(dir.path().join("untracked"), "not admitted\n").unwrap();
    let output = run_leaf(dir.path(), "clean-tree", &summary);
    assert_eq!(output.status.code(), Some(1));
    assert!(!dir.path().join(".xtask/last-result.json").exists());
    assert!(!summary.exists());
}

#[cfg(unix)]
#[test]
fn precommit_safety_process_routes_once_and_emits_no_report_envelope() {
    use std::os::unix::fs::PermissionsExt;

    let dir = checkout();
    std::fs::write(dir.path().join("guide.md"), "# Guide\n").unwrap();
    git(dir.path(), &["add", "guide.md"]);
    let fake_env = tempfile::tempdir().expect("fake command directory");
    let bin = fake_env.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let just = bin.join("just");
    std::fs::write(
        &just,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$JUST_ARGS\"\nexit 23\n",
    )
    .unwrap();
    std::fs::set_permissions(&just, std::fs::Permissions::from_mode(0o755)).unwrap();
    let args_file = fake_env.path().join("just-args.txt");
    let summary = fake_env.path().join("github-summary.md");
    let mut search_path = vec![bin];
    search_path.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command
        .arg("precommit-safety")
        .current_dir(dir.path())
        .env("JUST_ARGS", &args_file)
        .env("GITHUB_STEP_SUMMARY", &summary)
        .env("PATH", std::env::join_paths(search_path).unwrap());
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
    let output = command.output().expect("run precommit safety process");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("precommit-host"), "{stderr}");
    assert!(stderr.contains("exit status: 23"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(args_file).unwrap(),
        "precommit-markdown-host\n"
    );
    assert!(!dir.path().join(".xtask/last-result.json").exists());
    assert!(!summary.exists());
    assert_eq!(
        git(dir.path(), &["diff", "--cached", "--name-only"]).stdout,
        b"guide.md\n"
    );
}

#[test]
fn failed_leaf_process_is_nonzero_and_keeps_its_diagnostic_only() {
    let dir = checkout();
    std::fs::write(
        dir.path().join("docs/adr/0001-broken.md"),
        "# Wrong heading\n\n- Status: proposed\n",
    )
    .unwrap();
    git(dir.path(), &["add", "docs/adr/0001-broken.md"]);
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
            "bad ADR fixture",
        ],
    );
    let summary = dir.path().join("github-summary.md");
    let output = run_leaf(dir.path(), "adr-format", &summary);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("[FAIL] adr-format"), "{stdout}");
    assert!(!dir.path().join(".xtask/last-result.json").exists());
    assert!(!summary.exists());
}
