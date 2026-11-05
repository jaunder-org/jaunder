//! Exercise the actual pinned Just parser with fake tools, never the real index.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

struct Workflow {
    root: tempfile::TempDir,
}

impl Workflow {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../Justfile"),
            root.path().join("Justfile"),
        )
        .unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        for tool in ["cargo", "nix", "devtool", "prettier"] {
            let path = bin.join(tool);
            fs::write(
                &path,
                r#"#!/bin/sh
set -eu
name=${0##*/}
printf '%s\t' "$name" "$@" >> "$FAKE_LOG"
printf '\n' >> "$FAKE_LOG"
if [ "$name $*" = "cargo xtask test-local" ]; then
    touch producer-resource
    trap 'rm producer-resource; printf "cleaned\n" >> producer-cleanup' EXIT
fi
if [ "$name $*" = "${FAIL_COMMAND:-}" ]; then exit 17; fi
"#,
            )
            .unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { root }
    }

    fn run(&self, args: &[&str], failure: &str) -> (bool, Vec<Vec<String>>, String) {
        let log = self.root.path().join("commands");
        fs::write(&log, "").unwrap();
        let output = Command::new("just")
            .args(args)
            .current_dir(self.root.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("FAKE_LOG", &log)
            .env("FAIL_COMMAND", failure)
            .output()
            .unwrap();
        let commands = fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|line| {
                line.trim_end_matches('\t')
                    .split('\t')
                    .map(str::to_owned)
                    .collect()
            })
            .collect();
        (
            output.status.success(),
            commands,
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

fn broad(mode: &str, shared_docs: bool) -> Vec<Vec<String>> {
    let mut expected = Vec::new();
    for name in [
        "fmt",
        "leptosfmt",
        "prettier-markdown",
        "prettier-end2end",
        "prettier-css",
        "elisp-fmt",
        "tools-fmt",
        "ast-grep-tests",
        "no-full-reload",
    ] {
        if name == "prettier-markdown" && shared_docs {
            expected.extend(commands(&["cargo xtask check-leaf prepush-docs"]));
        } else {
            expected.push(devtool(name, mode));
        }
    }
    expected.extend(commands(&[if mode == "fix" {
        "cargo fmt --manifest-path xtask/Cargo.toml"
    } else {
        "cargo fmt --manifest-path xtask/Cargo.toml --check"
    }]));
    // Literal required population from the existing gate, not Justfile introspection.
    for name in [
        "sequence-check",
        "adr-format",
        "adr-readme-parity",
        "adr-view-parity",
        "doc-links",
        "flow-docs",
        "test-patterns",
        "server-fn-registrar",
        "server-fn-tracing",
        "server-fn-coverage",
        "server-fn-wire-arg-error",
        "traced-context",
        "e2e-telemetry-boundary",
        "e2e-collector-config",
        "proffered-secret",
        "e2e-goto-wrapper",
        "e2e-server-fn-endpoints",
        "target-arch-placement",
        "write-transaction-contract",
        "common-host-target-closure",
        "lint-suppression",
        "thin-components",
        "sqlx-newtype-bind",
        "sqlx-newtype-decode",
        "doctest-fences",
        "raw-html-door",
        "html-sink",
        "e2e-scaffold",
        "xlang-literal",
    ] {
        expected.push(vec![
            "cargo".into(),
            "xtask".into(),
            "check-leaf".into(),
            name.into(),
        ]);
    }
    for name in [
        "byte-compile",
        "tsc",
        "cargo-deny",
        "product-core-clippy",
        "web-server-clippy",
        "web-no-server-clippy",
        "client-host-clippy",
        "wasm-clippy",
        "tools-clippy",
    ] {
        expected.push(devtool(name, mode));
    }
    expected.extend(commands(&[
        "cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings",
    ]));
    expected.push(devtool("ert", mode));
    expected
}

fn devtool(name: &str, mode: &str) -> Vec<String> {
    let mut argv = vec!["devtool".into(), "check".into(), name.into()];
    if mode == "fix" {
        argv.push("--fix".into());
    }
    argv
}

fn host_tests() -> Vec<Vec<String>> {
    commands(&[
        "cargo test --manifest-path xtask/Cargo.toml --lib --bins --tests",
        "cargo test --manifest-path tools/Cargo.toml --lib --bins --tests",
    ])
}

fn nix(check: &str) -> Vec<String> {
    vec![
        "nix".into(),
        "build".into(),
        "-L".into(),
        "--accept-flake-config".into(),
        "--out-link".into(),
        format!(".xtask/gcroots/{check}"),
        format!(".#checks.x86_64-linux.{check}"),
    ]
}

fn platform() -> Vec<String> {
    vec![
        "nix".into(),
        "eval".into(),
        "--impure".into(),
        "--expr".into(),
        "import ./nix/platform-contract.nix { flake = builtins.getFlake (toString ./.); }".into(),
    ]
}

fn static_checks() -> Vec<Vec<String>> {
    [
        "static-docs",
        "static-typescript",
        "rust-format",
        "static-code",
        "static-elisp",
        "static-e2e-census",
        "static-ert",
        "xtask-format",
        "xtask-clippy",
    ]
    .map(nix)
    .to_vec()
}

fn coverage() -> Vec<Vec<String>> {
    let mut argv = vec![nix("coverage"), nix("coverage-gate")];
    argv.extend(commands(&["cargo xtask check-leaf coverage-consume"]));
    argv
}

fn doctests() -> Vec<Vec<String>> {
    vec![nix("doctests"), nix("doctests-gate")]
}
fn elisp() -> Vec<Vec<String>> {
    let mut argv = vec![nix("elisp-coverage-producer")];
    argv.extend(commands(&["cargo xtask check-leaf elisp-coverage-consume"]));
    argv
}

fn commands(lines: &[&str]) -> Vec<Vec<String>> {
    lines
        .iter()
        .map(|line| line.split(' ').map(str::to_owned).collect())
        .collect()
}

#[test]
fn broad_fix_and_verify_preserve_the_actual_complete_success_order() {
    let workflow = Workflow::new();
    let mut check = broad("fix", false);
    check.extend(host_tests());
    for recipe in ["check-no-test", "precommit-broad-host"] {
        let (ok, actual, stderr) = workflow.run(&[recipe], "");
        assert!(ok, "{recipe}: {stderr}");
        assert_eq!(actual, check, "{recipe}");
    }
    check.extend(commands(&["cargo xtask test-local"]));
    check.push(nix("wasm-tests"));
    check.extend(doctests());
    let (ok, actual, stderr) = workflow.run(&["check"], "");
    assert!(ok, "{stderr}");
    assert_eq!(actual, check);

    let mut prepush = commands(&["cargo xtask check-leaf clean-tree"]);
    prepush.extend(broad(
        "verify",
        cfg!(all(target_os = "linux", target_arch = "x86_64")),
    ));
    prepush.extend(host_tests());
    prepush.extend(commands(&[
        "cargo xtask test-local",
        "cargo xtask check-leaf workspace-doctests",
    ]));
    let (ok, actual, stderr) = workflow.run(&["prepush"], "");
    assert!(ok, "{stderr}");
    assert_eq!(actual, prepush);
}

#[test]
fn validation_and_ci_lanes_preserve_required_populations_and_argument_boundaries() {
    let workflow = Workflow::new();
    let clean = commands(&["cargo xtask check-leaf clean-tree"]);
    let mut host = broad("verify", false);
    host.push(platform());
    let mut hermetic = static_checks();
    hermetic.extend(commands(&["cargo xtask check-leaf wasm-budget"]));
    let mut test_checks = vec![nix("wasm-tests")];
    test_checks.extend(doctests());
    test_checks.extend(elisp());
    for (recipe, mut expected) in [
        ("ci-validate-host", {
            let mut argv = host.clone();
            argv.extend(host_tests());
            argv
        }),
        ("ci-validate-hermetic", hermetic.clone()),
        ("ci-validate-test-checks", test_checks),
        ("ci-validate-coverage", coverage()),
    ] {
        expected.splice(0..0, clean.clone());
        let (ok, actual, stderr) = workflow.run(&[recipe], "");
        assert!(ok, "{recipe}: {stderr}");
        assert_eq!(actual, expected, "{recipe}");
        assert!(
            !workflow
                .root
                .path()
                .join(".xtask/last-result.json")
                .exists()
        );
    }
    let mut validate = clean;
    validate.extend(host);
    validate.extend(hermetic);
    validate.extend(host_tests());
    validate.push(nix("wasm-tests"));
    validate.extend(coverage());
    validate.extend(doctests());
    validate.extend(elisp());
    let (ok, actual, stderr) = workflow.run(&["validate-no-e2e"], "");
    assert!(ok, "{stderr}");
    assert_eq!(actual, validate);
    validate.extend(commands(&[
        "cargo xtask e2e-lane sqlite-chromium-unsplit",
        "cargo xtask e2e-lane postgres-chromium-unsplit",
        "cargo xtask e2e sqlite firefox",
        "cargo xtask e2e postgres firefox",
        "cargo xtask check-leaf server-fn-flow",
    ]));
    let (ok, actual, stderr) = workflow.run(&["validate"], "");
    assert!(ok, "{stderr}");
    assert_eq!(actual, validate);
}

#[test]
fn every_required_leaf_failure_is_nonzero_and_stops_subsequent_work() {
    let workflow = Workflow::new();
    let mut tested = std::collections::BTreeSet::new();
    for recipe in [
        "precommit-markdown-host",
        "check",
        "prepush",
        "validate",
        "ci-validate-host",
        "ci-validate-hermetic",
        "ci-validate-test-checks",
        "ci-validate-coverage",
    ] {
        let (ok, success, stderr) = workflow.run(&[recipe], "");
        assert!(ok, "{recipe}: {stderr}");
        for (index, command) in success.iter().enumerate() {
            // Shared leaves need one fault injection, plus each public recipe's
            // first and final boundaries. Avoid repeating the whole host surface.
            if !tested.insert(command.clone()) && index != 0 && index + 1 != success.len() {
                continue;
            }
            let (ok, failed, stderr) = workflow.run(&[recipe], &command.join(" "));
            assert!(!ok, "{recipe} ignored {command:?}: {stderr}");
            assert_eq!(failed, success[..=index], "{recipe}: {command:?}");
            assert!(stderr.contains("17"), "child failure lost: {stderr}");
        }
    }
}

#[test]
fn synchronous_producer_cleanup_finishes_before_just_returns_success_or_failure() {
    for failure in ["", "cargo xtask test-local"] {
        let workflow = Workflow::new();
        let (ok, _, stderr) = workflow.run(&["check"], failure);
        assert_eq!(ok, failure.is_empty(), "{stderr}");
        assert!(!workflow.root.path().join("producer-resource").exists());
        assert_eq!(
            fs::read_to_string(workflow.root.path().join("producer-cleanup")).unwrap(),
            "cleaned\n"
        );
    }
}

#[test]
fn precommit_calls_only_the_focused_single_process_safety_owner() {
    let workflow = Workflow::new();
    for failure in ["", "cargo xtask precommit-safety"] {
        let (ok, actual, stderr) = workflow.run(&["precommit"], failure);
        assert_eq!(ok, failure.is_empty(), "{stderr}");
        assert_eq!(actual, commands(&["cargo xtask precommit-safety"]));
    }
}

#[test]
fn actual_just_parser_rejects_old_option_spellings_instead_of_silently_forwarding() {
    let workflow = Workflow::new();
    for argv in [
        vec!["check", "--no-test"],
        vec!["validate", "--no-e2e"],
        vec!["ci-validate", "host"],
    ] {
        let (ok, actual, _) = workflow.run(&argv, "");
        assert!(!ok, "unexpectedly accepted {argv:?}");
        assert!(actual.is_empty(), "invalid syntax started work: {argv:?}");
    }
}

#[test]
fn hooks_invoke_only_their_just_workflow_and_preserve_its_status() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    for (hook, recipe) in [("pre-commit", "precommit"), ("pre-push", "prepush")] {
        for status in [0, 17] {
            let fixture = tempfile::tempdir().unwrap();
            let just = fixture.path().join("just");
            fs::write(
                &just,
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOOK_ARGS\"\nexit \"$HOOK_STATUS\"\n",
            )
            .unwrap();
            fs::set_permissions(&just, fs::Permissions::from_mode(0o755)).unwrap();
            let args = fixture.path().join("args");
            let output = Command::new("bash")
                .arg(root.join(".githooks").join(hook))
                .current_dir(fixture.path())
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        fixture.path().display(),
                        std::env::var("PATH").unwrap()
                    ),
                )
                .env("HOOK_ARGS", &args)
                .env("HOOK_STATUS", status.to_string())
                .env("SKIP_PRE_COMMIT", "0")
                .env("SKIP_PRE_PUSH", "0")
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(status), "{hook}: {output:?}");
            assert_eq!(fs::read_to_string(args).unwrap(), format!("{recipe}\n"));
        }
    }
}

#[test]
fn ci_keeps_independent_job_verdicts_and_producer_artifacts_not_gate_sidecars() {
    let source = include_str!("../../.github/workflows/ci.yml");
    let workflow: serde_yaml::Value = serde_yaml::from_str(source).unwrap();
    let jobs = &workflow["jobs"];
    for lane in ["host", "hermetic", "test-checks", "coverage"] {
        let id = format!("validate-{lane}");
        let job = &jobs[&id];
        let steps = job["steps"].as_sequence().unwrap();
        let commands = steps
            .iter()
            .filter_map(|step| step["run"].as_str())
            .map(|run| run.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|run| run.contains("just ci-validate-"))
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            [format!(
                "nix develop .#ci --accept-flake-config -c just ci-validate-{lane}"
            )]
        );
        let encoded = serde_yaml::to_string(job).unwrap();
        assert!(!encoded.contains("last-result.json"));
        assert!(!encoded.contains(&format!("{lane}-result.json")));
    }
    let aggregate = &jobs["validate-no-e2e"];
    assert_eq!(aggregate["name"], "Validate (no e2e)");
    assert_eq!(aggregate["if"], "always()");
    assert_eq!(
        aggregate["needs"],
        serde_yaml::to_value([
            "validate-host",
            "validate-hermetic",
            "validate-test-checks",
            "validate-coverage"
        ])
        .unwrap()
    );
    let verdict = aggregate["steps"][0]["run"].as_str().unwrap();
    for (lane, variable) in [
        ("host", "host"),
        ("hermetic", "hermetic"),
        ("test-checks", "test_checks"),
        ("coverage", "coverage"),
    ] {
        assert!(verdict.contains(&format!("needs.validate-{lane}.result")));
        assert!(verdict.contains(&format!("test \"${variable}\" = \"success\"")));
    }
    assert!(source.contains(".xtask/gcroots/coverage/status.json"));
    assert!(source.contains(".xtask/gcroots/elisp-coverage-producer/elisp-coverage/"));
    assert_eq!(
        jobs["e2e-gate"]["needs"],
        serde_yaml::to_value(["e2e-chromium", "e2e-firefox-lane", "e2e-firefox-reconcile"])
            .unwrap()
    );
}

#[test]
fn markdown_host_runs_the_complete_narrow_fix_surface_in_order() {
    let workflow = Workflow::new();
    let (ok, actual, stderr) = workflow.run(&["precommit-markdown-host"], "");
    assert!(ok, "{stderr}");
    assert_eq!(
        actual,
        commands(&[
            "devtool check prettier-markdown --fix",
            "devtool check prettier-end2end --fix",
            "cargo xtask check-leaf sequence-check",
            "cargo xtask check-leaf adr-format",
            "cargo xtask check-leaf adr-readme-parity",
            "cargo xtask check-leaf adr-view-parity",
            "cargo xtask check-leaf doc-links",
            "cargo xtask check-leaf flow-docs",
        ])
    );
}
