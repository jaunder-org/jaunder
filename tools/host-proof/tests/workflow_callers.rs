//! The Git hooks and CI workflow call `dev` and keep each job's own verdict.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

#[test]
fn hooks_invoke_only_their_dev_workflow_and_preserve_its_status() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (hook, recipe) in [("pre-commit", "precommit"), ("pre-push", "prepush")] {
        for status in [0, 17] {
            let fixture = tempfile::tempdir().unwrap();
            let dev = fixture.path().join("dev");
            fs::write(
                &dev,
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOOK_ARGS\"\nexit \"$HOOK_STATUS\"\n",
            )
            .unwrap();
            fs::set_permissions(&dev, fs::Permissions::from_mode(0o755)).unwrap();
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
    let source = include_str!("../../../.github/workflows/ci.yml");
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
            .filter(|run| run.contains("dev ci-validate-"))
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            [format!(
                "nix develop .#ci --accept-flake-config -c dev ci-validate-{lane}"
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
        serde_yaml::to_value([
            "e2e-catalog",
            "e2e-chromium",
            "e2e-firefox-lane",
            "e2e-firefox-reconcile",
        ])
        .unwrap()
    );
    assert_eq!(jobs["e2e-gate"]["if"], "always()");
    for producer in ["e2e-chromium", "e2e-firefox-lane"] {
        assert_eq!(
            jobs[producer]["needs"],
            serde_yaml::to_value(["e2e-catalog"]).unwrap()
        );
    }
    let reconcile = &jobs["e2e-firefox-reconcile"];
    assert_eq!(
        reconcile["needs"],
        serde_yaml::to_value(["e2e-catalog", "e2e-firefox-lane"]).unwrap()
    );
    assert_eq!(reconcile["if"], "always()");
    let lines = jobs["e2e-gate"]["steps"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|step| step["run"].as_str())
        .flat_map(str::lines)
        .map(str::trim)
        .collect::<std::collections::BTreeSet<_>>();
    for binding in [
        "chromium=\"${{ needs.e2e-chromium.result }}\"",
        "firefox_lanes=\"${{ needs.e2e-firefox-lane.result }}\"",
        "firefox_reconcile=\"${{ needs.e2e-firefox-reconcile.result }}\"",
    ] {
        assert!(lines.contains(binding), "aggregate missing {binding}");
    }
    assert_eq!(
        lines
            .iter()
            .copied()
            .filter(|line| line.starts_with("test "))
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            "test \"${{ needs.e2e-catalog.result }}\" = \"success\"",
            "test \"$chromium\" = \"success\"",
            "test \"$firefox_lanes\" = \"success\"",
            "test \"$firefox_reconcile\" = \"success\"",
        ])
    );
}
