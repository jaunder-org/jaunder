//! Exercise Just at the external Nix and verifier boundaries, without VM builds.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
struct Workflow {
    root: tempfile::TempDir,
    path: String,
}
impl Workflow {
    fn new() -> Self {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        for dir in ["scripts", "end2end", "bin"] {
            fs::create_dir(root.path().join(dir)).unwrap();
        }
        for file in [
            "Justfile",
            "scripts/e2e-workflow",
            "scripts/e2e-matrix.jq",
            "scripts/step-report.jq",
            "end2end/e2e-lanes.json",
        ] {
            fs::copy(repo.join(file), root.path().join(file)).unwrap();
        }
        executable(
            &root.path().join("bin/nix"),
            r##"#!/usr/bin/env bash
set -eu
if [[ $1 == build ]]; then
  lane=${!#}; lane=${lane##*.e2e-}
  echo "producer $lane" >> "$EVENTS"
  echo "producer exit ${BUILD_STATUS:-23}" >&2
  if [[ -f $EVENTS.kept-note ]]; then
    while IFS= read -r note; do printf '%s\n' "$note" >&2; done < "$EVENTS.kept-note"
  fi
  if [[ ${BUILD_STATUS:-23} == 0 ]]; then
    while [[ $1 != --out-link ]]; do shift; done
    ln -sfn "$OUTPUT" "$2"
    printf '[{"outputs":{"out":"%s"}}]\n' "$OUTPUT"
  fi
  exit "${BUILD_STATUS:-23}"
fi
[[ -n ${OUTPUT-} ]] || exit 1
printf '%s' "$OUTPUT"
"##,
        );
        executable(
            &root.path().join("bin/cargo"),
            r##"#!/usr/bin/env bash
set -eu
echo "verifier $*" >> "$EVENTS"
if [[ -e $EVENTS.fail-reconcile && $* == *"-- reconcile "* ]]; then exit 31; fi
if [[ ${REAL_VERIFIER:-0} == 1 ]]; then
  while [[ $1 != -- ]]; do shift; done
  shift
  exec "$EVIDENCE_BIN" "$@"
fi
exit "${VERIFY_STATUS:-1}"
"##,
        );
        let path = format!(
            "{}:{}",
            root.path().join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        Self { root, path }
    }
    fn run(
        &self,
        args: &[&str],
        output: Option<&Path>,
        status: &str,
        real_verifier: bool,
    ) -> std::process::Output {
        let mut command = Command::new("just");
        command
            .args(args)
            .current_dir(self.root.path())
            .env("PATH", &self.path)
            .env("EVENTS", self.root.path().join("events"))
            .env("BUILD_STATUS", status)
            .env("VERIFY_STATUS", "0")
            .env("REAL_VERIFIER", if real_verifier { "1" } else { "0" })
            .env("EVIDENCE_BIN", env!("CARGO_BIN_EXE_e2e-evidence"));
        if let Some(output) = output {
            command.env("OUTPUT", output);
        }
        let output = command.output().unwrap();
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        output
    }
    fn fail_copy(&self, name: &str) {
        let cp = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|directory| directory.join("cp"))
            .find(|path| path.is_file())
            .unwrap();
        executable(
            &self.root.path().join("bin/cp"),
            &format!(
                r##"#!/usr/bin/env bash
set -eu
if [[ ${{@: -2:1}} == */{name} ]]; then
  echo 'injected copy failure: {name}' >&2
  exit 37
fi
exec '{}' "$@"
"##,
                cp.display()
            ),
        );
    }
    fn events(&self) -> String {
        fs::read_to_string(self.root.path().join("events")).unwrap()
    }
    fn capture(&self) -> PathBuf {
        self.root.path().join(
            ".xtask/diagnostics/e2e-sqlite-chromium-unsplit/capture-sqlite-chromium-unsplit.tar.gz",
        )
    }
}
fn immutable_fixture() -> PathBuf {
    immutable_fixture_with(&[(
        "capture-sqlite-chromium-unsplit.tar.gz",
        "current diagnostic",
    )])
}
fn immutable_fixture_with(artifacts: &[(&str, &str)]) -> PathBuf {
    let temp = tempfile::tempdir().unwrap();
    for (name, bytes) in artifacts {
        fs::write(temp.path().join(name), bytes).unwrap();
    }
    let output = Command::new("nix")
        .args(["store", "add-path", "--name", "e2e-workflow-fixture"])
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
#[test]
fn early_producer_failure_still_settles_all_lanes_and_both_reconciliations() {
    for (jobs, failure) in [
        ("", "23"),
        ("1", "23"),
        ("2", "255"),
        ("12", "23"),
        ("18446744073709551615", "23"),
    ] {
        let workflow = Workflow::new();
        fs::create_dir_all(workflow.capture().parent().unwrap()).unwrap();
        fs::write(workflow.capture(), "stale").unwrap();
        fs::write(workflow.root.path().join(".xtask/not-owned-keep"), "keep").unwrap();
        let args = if jobs.is_empty() {
            vec!["e2e"]
        } else {
            vec!["e2e", jobs]
        };
        let output = workflow.run(&args, None, failure, false);
        assert!(!output.status.success());
        let events = workflow.events();
        assert_eq!(
            events
                .lines()
                .filter(|s| s.starts_with("producer "))
                .count(),
            8,
            "{events}"
        );
        assert!(
            events.contains("producer postgres-firefox-serial-special"),
            "{events}"
        );
        let lines: Vec<_> = events.lines().collect();
        assert!(
            lines[8..]
                .iter()
                .any(|line| line.contains("reconcile sqlite")),
            "{events}"
        );
        assert!(
            lines[8..]
                .iter()
                .any(|line| line.contains("reconcile postgres")),
            "{events}"
        );
        assert!(!events.contains("server-fn-coverage"), "{events}");
        assert!(
            !workflow.capture().exists(),
            "failed producer reused stale evidence"
        );
        assert_eq!(
            fs::read_to_string(workflow.root.path().join(".xtask/not-owned-keep")).unwrap(),
            "keep"
        );
        for lane in ["sqlite-chromium-unsplit", "postgres-firefox-serial-special"] {
            let log = workflow
                .root
                .path()
                .join(format!(".xtask/diagnostics/e2e-{lane}/build.log"));
            assert!(
                fs::read_to_string(log)
                    .unwrap()
                    .contains(&format!("producer exit {failure}"))
            );
        }
    }
}
#[test]
fn producer_rejects_unknown_disabled_and_internal_mode_names_without_building() {
    for identity in ["unknown", "sqlite-firefox-unsplit", "all"] {
        let workflow = Workflow::new();
        let output = workflow.run(&["e2e-producer", identity], None, "23", false);
        assert!(!output.status.success());
        assert!(
            !workflow.root.path().join("events").exists(),
            "invalid producer {identity} dispatched work"
        );
    }
}

#[test]
fn reconciliation_failure_is_blocking_even_when_all_producers_succeed() {
    let workflow = Workflow::new();
    let selected = immutable_fixture();
    fs::write(workflow.root.path().join("events.fail-reconcile"), "").unwrap();
    let output = workflow.run(&["e2e"], Some(&selected), "0", false);
    assert!(!output.status.success());
    let events = workflow.events();
    assert_eq!(
        events
            .lines()
            .filter(|line| line.starts_with("producer "))
            .count(),
        8
    );
    assert_eq!(
        events
            .lines()
            .filter(|line| line.contains("e2e-evidence -- reconcile"))
            .count(),
        2
    );
    assert_eq!(
        events
            .lines()
            .filter(|line| line.contains("server-fn-coverage verify"))
            .count(),
        1
    );
    assert!(
        !events.contains("regenerate"),
        "normal full workflow regenerated coverage"
    );
    assert_eq!(
        events
            .lines()
            .filter(|line| line.contains("e2e-evidence -- verify"))
            .count(),
        7,
        "non-authoritative lane verification changed"
    );
}

#[test]
fn successful_selection_keeps_root_lifts_diagnostics_and_passes_selected_output_to_coverage() {
    let workflow = Workflow::new();
    let selected = immutable_fixture();
    fs::create_dir_all(workflow.capture().parent().unwrap()).unwrap();
    fs::write(workflow.capture(), "stale").unwrap();
    fs::set_permissions(workflow.capture(), fs::Permissions::from_mode(0o444)).unwrap();
    let output = workflow.run(
        &["e2e-producer", "sqlite-chromium-unsplit"],
        Some(&selected),
        "0",
        false,
    );
    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(workflow.capture()).unwrap(),
        "current diagnostic"
    );
    assert_eq!(
        fs::read_link(
            workflow
                .root
                .path()
                .join(".xtask/gcroots/e2e-sqlite-chromium-unsplit")
        )
        .unwrap(),
        selected
    );
    let events = workflow.events();
    assert!(
        events.contains(&format!(
            "--capture {}/capture-sqlite-chromium-unsplit.tar.gz --producer-output {}",
            selected.display(),
            selected.display()
        )),
        "{events}"
    );
    assert!(!events.contains("--capture .xtask/diagnostics"));
}
#[test]
fn explicit_regeneration_requires_success_and_normal_authoritative_production_verifies_once() {
    let selected = immutable_fixture();
    for (recipe, status, action) in [
        ("e2e-regenerate-coverage", "0", "regenerate"),
        ("e2e-regenerate-coverage", "23", ""),
        ("e2e-producer", "0", "verify"),
    ] {
        let workflow = Workflow::new();
        let args = if recipe == "e2e-producer" {
            vec![recipe, "sqlite-chromium-unsplit"]
        } else {
            vec![recipe]
        };
        let output = workflow.run(&args, Some(&selected), status, false);
        assert_eq!(
            output.status.success(),
            status == "0",
            "{recipe}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let events = workflow.events();
        assert!(
            !events.contains("e2e-evidence -- verify"),
            "authoritative lane checked twice: {events}"
        );
        if action.is_empty() {
            assert!(
                !events.contains("server-fn-coverage"),
                "failed producer regenerated: {events}"
            );
        } else {
            assert_eq!(
                events
                    .lines()
                    .filter(|line| line.contains(&format!("server-fn-coverage {action}")))
                    .count(),
                1,
                "{events}"
            );
        }
        if action != "regenerate" {
            assert!(!events.contains("regenerate"), "{events}");
        }
    }
}

#[test]
fn optional_provenance_copy_failure_warns_without_blocking_successful_production() {
    let workflow = Workflow::new();
    let marker = "producer-input-sqlite-chromium-unsplit.json";
    let selected = immutable_fixture_with(&[
        (
            "capture-sqlite-chromium-unsplit.tar.gz",
            "current diagnostic",
        ),
        (marker, "attribution only"),
    ]);
    workflow.fail_copy(marker);
    let output = workflow.run(
        &["e2e-producer", "sqlite-chromium-unsplit"],
        Some(&selected),
        "0",
        false,
    );
    assert!(
        output.status.success(),
        "optional provenance failure blocked producer"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("warning: optional attribution"), "{stderr}");
    assert!(stderr.contains(marker), "{stderr}");
    assert_eq!(
        fs::read_to_string(workflow.capture()).unwrap(),
        "current diagnostic"
    );
    assert!(workflow.events().contains("server-fn-coverage verify"));
    assert!(
        selected.join(marker).is_file(),
        "immutable attribution source was removed"
    );
}

#[test]
fn required_artifact_copy_failures_block_success_but_preserve_an_existing_producer_failure() {
    let marker = "producer-input-sqlite-chromium-unsplit.json";
    let required = [
        "capture-sqlite-chromium-unsplit.tar.gz",
        "playwright-report-sqlite-chromium-unsplit.json",
        "duration-budget-manifest-sqlite-chromium-unsplit.json",
    ];
    let selected = immutable_fixture_with(&[
        (required[0], "current diagnostic"),
        (required[1], "report"),
        (required[2], "duration"),
        (marker, "attribution only"),
    ]);
    for (status, failures) in [
        ("0", required.as_slice()),
        ("23", &[marker, required[0]][..]),
    ] {
        for artifact in failures {
            let workflow = Workflow::new();
            workflow.fail_copy(artifact);
            let output = workflow.run(
                &["e2e-producer", "sqlite-chromium-unsplit"],
                Some(&selected),
                status,
                false,
            );
            assert!(
                !output.status.success(),
                "{artifact}: failed copy became green"
            );
            assert!(
                !workflow.events().contains("verifier"),
                "{artifact}: failed producer verified evidence"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains(&format!("injected copy failure: {artifact}")),
                "{stderr}"
            );
            if status == "23" {
                assert!(
                    stderr.contains("exit code 23"),
                    "original producer status was replaced: {stderr}"
                );
            }
            let retained = if *artifact == required[0] {
                required[1]
            } else {
                required[0]
            };
            assert!(
                workflow
                    .root
                    .path()
                    .join(format!(
                        ".xtask/diagnostics/e2e-sqlite-chromium-unsplit/{retained}"
                    ))
                    .is_file(),
                "other diagnostics were not lifted"
            );
        }
    }
}

#[test]
fn failed_producer_lifts_available_evidence_but_never_verifies_it() {
    let workflow = Workflow::new();
    let selected = immutable_fixture();
    let output = workflow.run(
        &["e2e-producer", "sqlite-chromium-unsplit"],
        Some(&selected),
        "23",
        false,
    );
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(workflow.capture()).unwrap(),
        "current diagnostic"
    );
    assert!(!workflow.events().contains("verifier"));
}
#[test]
fn failed_producer_recovers_only_its_logged_modern_nix_build_and_selected_lane() {
    let workflow = Workflow::new();
    let kept = workflow
        .root
        .path()
        .join("nix/var/nix/builds/nix-184917-2991942680/build");
    let unrelated = workflow
        .root
        .path()
        .join("nix/var/nix/builds/nix-999999-1234567890/build");
    for root in [&kept, &unrelated] {
        fs::create_dir_all(root.join("emit-out/diagnostics")).unwrap();
    }
    fs::write(
        kept.join("emit-out/diagnostics/capture-sqlite-chromium-unsplit.tar.gz"),
        "current failed diagnostic",
    )
    .unwrap();
    fs::write(
        kept.join("emit-out/diagnostics/capture-postgres-chromium-unsplit.tar.gz"),
        "other lane",
    )
    .unwrap();
    fs::write(
        unrelated.join("emit-out/diagnostics/playwright-report-sqlite-chromium-unsplit.json"),
        "stale unrelated report",
    )
    .unwrap();
    fs::create_dir_all(workflow.capture().parent().unwrap()).unwrap();
    fs::write(
        workflow.capture().parent().unwrap().join("build.log"),
        format!(
            "note: keeping build directory \"{}\"\n",
            unrelated.display()
        ),
    )
    .unwrap();
    fs::write(
        workflow.root.path().join("events.kept-note"),
        format!("note: keeping build directory \"{}\"\n", kept.display()),
    )
    .unwrap();
    let output = workflow.run(
        &["e2e-producer", "sqlite-chromium-unsplit"],
        None,
        "23",
        false,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exit code 23"));
    assert_eq!(
        fs::read_to_string(workflow.capture()).unwrap(),
        "current failed diagnostic"
    );
    let diagnostics = workflow.capture().parent().unwrap().to_owned();
    assert!(
        !diagnostics
            .join("playwright-report-sqlite-chromium-unsplit.json")
            .exists(),
        "borrowed another build's stale evidence"
    );
    assert!(
        !diagnostics
            .join("capture-postgres-chromium-unsplit.tar.gz")
            .exists(),
        "borrowed another lane's evidence"
    );
    assert!(
        unrelated
            .join("emit-out/diagnostics/playwright-report-sqlite-chromium-unsplit.json")
            .is_file()
    );
    assert!(!workflow.events().contains("verifier"));
}

#[test]
fn absent_malformed_or_ambiguous_kept_paths_never_replace_failure_or_borrow_old_evidence() {
    for variant in [
        "absent",
        "missing",
        "relative",
        "unquoted",
        "traversal",
        "ambiguous",
    ] {
        let workflow = Workflow::new();
        let kept = workflow
            .root
            .path()
            .join("nix/var/nix/builds/nix-184917-2991942680/build");
        fs::create_dir_all(kept.join("emit-out/diagnostics")).unwrap();
        fs::write(
            kept.join("emit-out/diagnostics/capture-sqlite-chromium-unsplit.tar.gz"),
            "unselected stale diagnostic",
        )
        .unwrap();
        fs::create_dir_all(workflow.capture().parent().unwrap()).unwrap();
        fs::write(
            workflow.capture().parent().unwrap().join("build.log"),
            format!("note: keeping build directory \"{}\"\n", kept.display()),
        )
        .unwrap();
        let note = match variant {
            "absent" => String::new(),
            "missing" => format!(
                "note: keeping build directory \"{}/missing\"\n",
                kept.display()
            ),
            "relative" => "note: keeping build directory \"relative/build\"\n".into(),
            "unquoted" => format!("note: keeping build directory {}\n", kept.display()),
            "traversal" => format!(
                "note: keeping build directory \"{}/../build\"\n",
                kept.display()
            ),
            "ambiguous" => format!(
                "note: keeping build directory \"{}\"\nnote: keeping build directory \"/another/build\"\n",
                kept.display()
            ),
            _ => unreachable!(),
        };
        fs::write(workflow.root.path().join("events.kept-note"), note).unwrap();
        let output = workflow.run(
            &["e2e-producer", "sqlite-chromium-unsplit"],
            None,
            "23",
            false,
        );
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("exit code 23"),
            "{variant}: producer failure replaced"
        );
        assert!(
            !workflow.capture().exists(),
            "{variant}: borrowed unselected evidence"
        );
        assert!(!workflow.events().contains("verifier"));
    }
}

#[test]
fn artifact_reconciliation_is_build_free_and_has_no_result_envelope() {
    let workflow = Workflow::new();
    let output = Command::new(env!("CARGO_BIN_EXE_e2e-evidence"))
        .args([
            "reconcile",
            "sqlite",
            "--catalog",
            "end2end/e2e-lanes.json",
            "--diagnostics-root",
            "missing",
        ])
        .current_dir(workflow.root.path())
        .env("PATH", &workflow.path)
        .env("EVENTS", workflow.root.path().join("events"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        !workflow.root.path().join("events").exists(),
        "artifact reconciliation invoked a build"
    );
    assert!(
        !workflow
            .root
            .path()
            .join(".xtask/last-result.json")
            .exists()
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert!(
        report["steps"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("reading expected census")
    );
}

#[test]
fn missing_successful_evidence_fails_real_verifier_and_never_reuses_stale_capture() {
    let selected = immutable_fixture();
    for args in [
        vec!["e2e-producer", "sqlite-chromium-unsplit"],
        vec!["e2e-regenerate-coverage"],
    ] {
        let workflow = Workflow::new();
        let output = workflow.run(&args, Some(&selected), "0", true);
        assert!(!output.status.success());
        assert!(!workflow.events().contains("e2e-evidence -- verify"));
        assert!(workflow.events().contains("server-fn-coverage"));
        assert!(String::from_utf8_lossy(&output.stdout).contains("reading report"));
        assert!(
            !workflow
                .root
                .path()
                .join("docs/coverage/server-fns.json")
                .exists(),
            "invalid evidence regenerated the snapshot"
        );
    }
}
