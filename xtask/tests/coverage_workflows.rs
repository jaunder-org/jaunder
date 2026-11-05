//! Exercise the real Just entrypoints without realizing expensive Nix producers.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

struct Workflow {
    root: tempfile::TempDir,
}

impl Workflow {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let git = xtask::git::at(root.path())
            .args(["init", "--quiet"])
            .output()
            .unwrap();
        assert!(git.status.success());
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../Justfile"),
            root.path().join("Justfile"),
        )
        .unwrap();
        fs::write(
            root.path().join(".git/info/exclude"),
            "Justfile\nbin/\nrecord\n.xtask/\n",
        )
        .unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        for program in ["nix", "cargo"] {
            let path = root.path().join("bin").join(program);
            fs::write(
                &path,
                format!(
                    "#!/usr/bin/env bash\nset -eu\nprintf '%s\\n' \"{program} $*\" >> record\ncase \"$*\" in *\"${{FAIL_MATCH:-never-match}}\"*) exit 23;; esac\n"
                ),
            )
            .unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { root }
    }

    fn run(&self, recipe: &str, fail: &str) -> Output {
        let mut command = Command::new("just");
        command
            .arg(recipe)
            .current_dir(self.root.path())
            .env("FAIL_MATCH", fail)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            );
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
        ] {
            command.env_remove(key);
        }
        command.output().unwrap()
    }

    fn record(&self) -> String {
        fs::read_to_string(self.root.path().join("record")).unwrap_or_default()
    }
}

#[test]
fn coverage_policy_cli_is_a_non_orchestrating_report_leaf() {
    let workflow = Workflow::new();
    // The native executable still admits a Jaunder checkout before dispatch.
    fs::write(workflow.root.path().join("flake.nix"), "fixture marker").unwrap();
    fs::create_dir(workflow.root.path().join("xtask")).unwrap();
    fs::write(
        workflow.root.path().join("xtask/Cargo.toml"),
        "fixture marker",
    )
    .unwrap();
    let out = workflow.root.path().join("reports");
    fs::create_dir(&out).unwrap();
    let source = workflow.root.path().join("sample.rs");
    fs::write(&source, "fn sample() {}\n").unwrap();
    fs::write(out.join("crap-report.json"), r#"{"entries":[]}"#).unwrap();
    for (count, passes) in [("1", true), ("0", false)] {
        fs::write(
            out.join("coverage-report.txt"),
            format!("{}:\n1|{count}|fn sample() {{}}\n", source.display()),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["coverage-policy", "--out"])
            .arg(&out)
            .current_dir(workflow.root.path())
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "core.hooksPath")
            .env("GIT_CONFIG_VALUE_0", ".githooks")
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    workflow.root.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            passes,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            workflow.record(),
            "",
            "the policy leaf must never run Nix or cargo"
        );
    }
}

#[test]
fn workflows_stop_on_each_nonzero_boundary() {
    for (recipe, failure, expected_commands) in [
        ("coverage", "--out-link", 1),
        ("coverage", "coverage-gate", 2),
        ("coverage", "coverage-policy", 3),
        ("doctests", "--out-link", 1),
        ("doctests", "doctests-gate", 2),
    ] {
        let workflow = Workflow::new();
        let output = workflow.run(recipe, failure);
        assert!(!output.status.success(), "{recipe}: {failure}");
        assert_eq!(
            workflow.record().lines().count(),
            expected_commands,
            "{recipe}: {failure}"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("23"));
    }
}

#[test]
fn doctests_runs_producer_then_gate_without_host_policy() {
    let workflow = Workflow::new();
    let output = workflow.run("doctests", "never-match");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        workflow.record(),
        concat!(
            "nix build --no-update-lock-file --no-write-lock-file -L .#checks.x86_64-linux.doctests --out-link .xtask/gcroots/doctests\n",
            "nix build --no-update-lock-file --no-write-lock-file -L .#checks.x86_64-linux.doctests-gate --no-link\n",
        )
    );
}

#[test]
fn public_workflows_refuse_dirty_or_unavailable_git_status() {
    for recipe in ["coverage", "doctests"] {
        for staged in [false, true] {
            let workflow = Workflow::new();
            fs::write(workflow.root.path().join("dirty"), "changed").unwrap();
            if staged {
                assert!(
                    xtask::git::at(workflow.root.path())
                        .args(["add", "dirty"])
                        .status()
                        .unwrap()
                        .success()
                );
            }
            assert!(!workflow.run(recipe, "never-match").status.success());
            assert_eq!(workflow.record(), "");
        }
        let workflow = Workflow::new();
        let git = workflow.root.path().join("bin/git");
        fs::write(&git, "#!/usr/bin/env bash\nexit 24\n").unwrap();
        fs::set_permissions(git, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!workflow.run(recipe, "never-match").status.success());
        assert_eq!(workflow.record(), "");
    }
}

#[test]
fn coverage_runs_producer_then_gate_then_policy() {
    let workflow = Workflow::new();
    let output = workflow.run("coverage", "never-match");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        workflow.record(),
        concat!(
            "nix build --no-update-lock-file --no-write-lock-file -L .#checks.x86_64-linux.coverage --out-link .xtask/gcroots/coverage\n",
            "nix build --no-update-lock-file --no-write-lock-file -L .#checks.x86_64-linux.coverage-gate --no-link\n",
            "cargo xtask coverage-policy --out .xtask/gcroots/coverage\n",
        )
    );
}
