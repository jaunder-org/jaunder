//! The Git hooks and CI workflows call `dev` and keep each job's own verdict.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

#[test]
fn hooks_invoke_only_their_dev_workflow_and_preserve_its_status() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (hook, workflow) in [("pre-commit", "precommit"), ("pre-push", "prepush")] {
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
            assert_eq!(fs::read_to_string(args).unwrap(), format!("{workflow}\n"));
        }
    }
}

#[test]
fn ci_keeps_independent_job_verdicts_and_producer_artifacts_not_gate_sidecars() {
    let source = include_str!("../../../.github/workflows/ci.yml");
    let workflow: serde_yaml::Value = serde_yaml::from_str(source).unwrap();
    let jobs = &workflow["jobs"];
    // Only the host lane runs host tools beyond Nix and `dev`; the others take
    // the lean shell.
    for (lane, shell, gate) in [
        ("host", "ci", "ci-validate-host"),
        ("hermetic", "ci-nix", "ci-validate-hermetic"),
        ("test-checks", "ci-nix", "ci-validate-test-checks"),
        ("coverage", "ci-nix", "coverage"),
    ] {
        let id = format!("validate-{lane}");
        let job = &jobs[&id];
        let steps = job["steps"].as_sequence().unwrap();
        let commands = steps
            .iter()
            .filter_map(|step| step["run"].as_str())
            .map(|run| run.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|run| run.contains("nix develop "))
            .collect::<Vec<_>>();
        // Every step enters the lane's shell, the workflow and its trace
        // summary among them.
        let entry = format!("nix develop .#{shell} --accept-flake-config -c ");
        assert!(
            commands.iter().all(|run| run.starts_with(&entry)),
            "{id}: {commands:?}"
        );
        assert!(
            commands.contains(&format!("{entry}dev {gate}")),
            "{id}: {commands:?}"
        );
        assert!(
            commands
                .iter()
                .any(|run| run.starts_with(&format!("{entry}sh -c 'dev trace summary"))),
            "{id}: {commands:?}"
        );
        let encoded = serde_yaml::to_string(job).unwrap();
        assert!(!encoded.contains("last-result.json"));
        assert!(!encoded.contains(&format!("{lane}-result.json")));
    }
    // The aggregates keep always(): otherwise a failed lane would skip a
    // required check, which branch protection accepts. Only a push of a
    // merge-queue-tested commit skips them.
    let aggregate_if = "always() && needs.merge-queue-verdict.outputs.tested != 'true'";
    for name in ["validate-no-e2e", "e2e-reconcile", "e2e-gate"] {
        assert_eq!(jobs[name]["if"], aggregate_if, "{name}");
    }
    // Pull-request and merge-queue runs can never skip, whatever the script does.
    assert!(
        jobs["merge-queue-verdict"]["outputs"]["tested"]
            .as_str()
            .unwrap()
            .starts_with("${{ github.event_name == 'push' && ")
    );
    let aggregate = &jobs["validate-no-e2e"];
    assert_eq!(aggregate["name"], "Validate (no e2e)");
    assert_eq!(
        aggregate["needs"],
        serde_yaml::to_value([
            "merge-queue-verdict",
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
            "merge-queue-verdict",
            "e2e-catalog",
            "e2e-lane",
            "e2e-reconcile"
        ])
        .unwrap()
    );
    assert_eq!(
        jobs["e2e-lane"]["needs"],
        serde_yaml::to_value(["merge-queue-verdict", "e2e-catalog"]).unwrap()
    );
    assert_eq!(
        jobs["e2e-reconcile"]["needs"],
        serde_yaml::to_value(["merge-queue-verdict", "e2e-catalog", "e2e-lane"]).unwrap()
    );
    let lines = jobs["e2e-gate"]["steps"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|step| step["run"].as_str())
        .flat_map(str::lines)
        .map(str::trim)
        .collect::<std::collections::BTreeSet<_>>();
    for binding in [
        "lanes=\"${{ needs.e2e-lane.result }}\"",
        "reconcile=\"${{ needs.e2e-reconcile.result }}\"",
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
            "test \"$lanes\" = \"success\"",
            "test \"$reconcile\" = \"success\"",
        ])
    );
}

/// Each named job runs a `dev` workflow, then summarises its trace into the step
/// summary and uploads `.xtask/run/`, both diagnostic only. The summary reads the
/// latest run, so no other `dev` step may sit between the workflow and it.
fn assert_jobs_keep_and_summarise_their_trace(source: &str, names: &[&str]) {
    let workflow: serde_yaml::Value = serde_yaml::from_str(source).unwrap();
    let run = |step: &serde_yaml::Value| {
        step["run"]
            .as_str()
            .map(|run| run.split_whitespace().collect::<Vec<_>>().join(" "))
    };
    for name in names {
        let steps = workflow["jobs"][name]["steps"].as_sequence().unwrap();
        let is_dev =
            |step: &serde_yaml::Value| run(step).is_some_and(|run| run.contains("-c dev "));
        let first_workflow = steps
            .iter()
            .position(is_dev)
            .unwrap_or_else(|| panic!("{name}: no dev workflow"));
        let uploads = steps
            .iter()
            .filter(|step| {
                step["uses"]
                    .as_str()
                    .is_some_and(|uses| uses.starts_with("actions/upload-artifact@"))
                    && step["with"]["path"]
                        .as_str()
                        .is_some_and(|path| path.lines().any(|line| line.trim() == ".xtask/run/"))
            })
            .collect::<Vec<_>>();
        assert_eq!(uploads.len(), 1, "{name}: one trace upload");
        assert_eq!(uploads[0]["if"].as_str(), Some("always()"), "{name}");
        // An empty upload warns and a failed one annotates the run, but neither
        // decides the job's result.
        assert_eq!(
            uploads[0]["continue-on-error"].as_bool(),
            Some(true),
            "{name}"
        );
        assert_eq!(
            uploads[0]["with"]["if-no-files-found"].as_str(),
            Some("warn"),
            "{name}"
        );
        assert_eq!(
            uploads[0]["with"]["include-hidden-files"].as_bool(),
            Some(true),
            "{name}"
        );
        assert_eq!(
            uploads[0]["with"]["retention-days"].as_u64(),
            Some(14),
            "{name}"
        );
        let summaries = steps
            .iter()
            .enumerate()
            .filter(|(_, step)| {
                run(step).is_some_and(|run| run.contains("dev trace summary --markdown"))
            })
            .collect::<Vec<_>>();
        assert_eq!(summaries.len(), 1, "{name}: one summary step");
        let (index, summary) = summaries[0];
        assert!(
            index > first_workflow,
            "{name}: the summary follows the workflow"
        );
        assert!(
            !steps[first_workflow + 1..index].iter().any(is_dev),
            "{name}: another dev step runs between the workflow and its summary"
        );
        assert_eq!(summary["if"].as_str(), Some("always()"), "{name}");
        let command = run(summary).unwrap();
        assert!(
            command.contains(">> \"$GITHUB_STEP_SUMMARY\"")
                && command.ends_with(
                    "|| echo \"::warning::Trace summary unavailable, see this step's log\""
                ),
            "{name}: the summary warns instead of failing: {command}"
        );
        assert_summary_warns_without_a_trace(name, summary["run"].as_str().unwrap());
    }
}

/// Runs a summary step as the runner does, with `nix develop` failing the way
/// `dev trace summary` does when `.xtask/run/` holds no trace: the step must
/// annotate the run and still succeed.
fn assert_summary_warns_without_a_trace(name: &str, command: &str) {
    let fixture = tempfile::tempdir().unwrap();
    let nix = fixture.path().join("nix");
    fs::write(
        &nix,
        "#!/bin/sh\necho 'Error: no traced runs' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&nix, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new("bash")
        .args(["-e", "-o", "pipefail", "-c", command])
        .current_dir(fixture.path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                fixture.path().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("GITHUB_STEP_SUMMARY", fixture.path().join("summary.md"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{name}: {output:?}");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .any(|line| line.starts_with("::warning::")),
        "{name}: no warning annotation"
    );
}

#[test]
fn lane_evidence_warns_when_empty_and_still_fails_the_lane_on_upload_failure() {
    let workflow: serde_yaml::Value =
        serde_yaml::from_str(include_str!("../../../.github/workflows/ci.yml")).unwrap();
    let evidence = workflow["jobs"]["e2e-lane"]["steps"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter(|step| step["name"].as_str() == Some("Upload lane-qualified e2e evidence"))
        .collect::<Vec<_>>();
    assert_eq!(evidence.len(), 1);
    assert_eq!(
        evidence[0]["with"]["if-no-files-found"].as_str(),
        Some("warn")
    );
    assert!(evidence[0]["continue-on-error"].is_null());
}

#[test]
fn every_validation_and_e2e_job_keeps_its_trace_and_summarises_it_without_a_verdict() {
    assert_jobs_keep_and_summarise_their_trace(
        include_str!("../../../.github/workflows/ci.yml"),
        &[
            "validate-host",
            "validate-hermetic",
            "validate-test-checks",
            "validate-coverage",
            "e2e-lane",
            "e2e-reconcile",
        ],
    );
}

#[test]
fn the_performance_job_keeps_its_trace_and_summarises_it_without_a_verdict() {
    assert_jobs_keep_and_summarise_their_trace(
        include_str!("../../../.github/workflows/performance.yml"),
        &["performance"],
    );
}
