//! The coverage policy leaf reports without orchestrating Nix or cargo. The
//! `dev` coverage workflows are tested in `tools/dev/tests/`.
#![cfg(feature = "host")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

struct Workflow {
    root: tempfile::TempDir,
}

impl Workflow {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let git = host_tool::git::at(root.path())
            .args(["init", "--quiet"])
            .output()
            .unwrap();
        assert!(git.status.success());
        fs::write(
            root.path().join(".git/info/exclude"),
            "bin/\nrecord\n.xtask/\n",
        )
        .unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        for program in ["nix", "cargo"] {
            let path = root.path().join("bin").join(program);
            fs::write(
                &path,
                format!("#!/usr/bin/env bash\nprintf '%s\\n' \"{program} $*\" >> record\n"),
            )
            .unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { root }
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
    fs::create_dir(workflow.root.path().join("tools")).unwrap();
    fs::write(
        workflow.root.path().join("tools/Cargo.toml"),
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
        let output = Command::new(env!("CARGO_BIN_EXE_coverage-check"))
            .args(["policy", "--out"])
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
