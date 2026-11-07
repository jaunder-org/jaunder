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
fn qualification_leaf_rejects_json_before_admission_without_report_artifacts() {
    let harness = "1111111111111111111111111111111111111111";
    let source = "2222222222222222222222222222222222222222";
    let target = "3333333333333333333333333333333333333333";
    for inputs in [
        vec![],
        vec!["--expected-harness", harness],
        vec!["--expected-harness", harness, "--source", source],
        vec![
            "--expected-harness",
            harness,
            "--source",
            source,
            "--target",
            target,
        ],
    ] {
        // No checkout is needed: JSON refusal must precede source admission.
        let dir = tempfile::tempdir().unwrap();
        let summary = dir.path().join("github-summary.md");
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["--json", "qualification-leaf"])
            .args(inputs)
            .current_dir(dir.path())
            .env("GITHUB_STEP_SUMMARY", &summary)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(stderr, "--json is not supported for `qualification-leaf`\n");
        assert!(!dir.path().join(".xtask").exists());
        assert!(!summary.exists());
    }
}

#[test]
fn qualification_leaf_binds_expected_clean_head_before_nix_without_reports() {
    for product in [false, true] {
        let dir = checkout();
        let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
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
            "qualification-leaf",
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
        assert!(!stderr.contains("xtask-done:"), "{stderr}");
        assert!(!dir.path().join(".xtask/last-result.json").exists());
        assert!(!dir.path().join(".xtask/gcroots").exists());
    }
}

#[test]
fn qualification_leaf_rejects_expressions_and_admits_local_pin_without_upstream() {
    let dir = checkout();
    let expected = String::from_utf8(git(dir.path(), &["rev-parse", "HEAD"]).stdout).unwrap();
    for product in [
        Some("HEAD"),
        Some("2222222222222222222222222222222222222222"),
        None,
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
        command.current_dir(dir.path()).args([
            "qualification-leaf",
            "--expected-harness",
            expected.trim(),
        ]);
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
        assert!(!stderr.contains("xtask-done:"), "{stderr}");
        assert!(!dir.path().join(".xtask/last-result.json").exists());
    }
}

#[test]
fn old_qualification_routes_are_retired_but_other_nix_routes_remain() {
    for args in [
        vec!["production-baseline", "discover"],
        vec!["production-baseline", "accept"],
        vec!["production-baseline", "qualify-styling"],
        vec!["nix", "production-baseline-smoke"],
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
