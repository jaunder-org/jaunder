#![cfg(unix)]

use anyhow::{Context, Result};
use performance::{Backend, Browser, DatasetProfile, RESULT_FILENAME, canonical_plan};
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

const NONCE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const GITHUB_ENV: [&str; 7] = [
    "GITHUB_REPOSITORY",
    "GITHUB_WORKFLOW",
    "GITHUB_JOB",
    "GITHUB_REF",
    "GITHUB_SHA",
    "GITHUB_RUN_ID",
    "GITHUB_RUN_ATTEMPT",
];

struct Fixture {
    directory: TempDir,
    invocation: PathBuf,
    performance: PathBuf,
    fragment: PathBuf,
}

impl Fixture {
    fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join(".xtask/performance").join(NONCE);
        let name = "performance-browser-sqlite-chromium";
        let performance = root.join("producers").join(name).join("performance");
        let fragments = performance.join("fragments");
        fs::create_dir_all(&fragments)?;
        let fragment = fragments.join("browser-sqlite-chromium-v1.json");
        let envelope = browser_fragment()?;
        write_json(&fragment, &envelope)?;
        for reference in envelope["fragment"]["result"]["diagnostics"]["navigation_artifacts"]
            .as_array()
            .context("navigation array")?
            .iter()
            .chain(
                envelope["fragment"]["result"]["diagnostics"]["trace_artifacts"]
                    .as_array()
                    .context("trace array")?,
            )
            .chain(std::iter::once(
                &envelope["fragment"]["result"]["diagnostics"]["otel_trace_artifact"],
            ))
        {
            fs::write(
                fragments.join(reference.as_str().context("diagnostic path")?),
                "diagnostic",
            )?;
        }
        write_json(
            &performance.join("producer-status-v1.json"),
            &json!({"schema_version": 1, "ok": true, "detail": "success"}),
        )?;
        let selection = json!({"backends": ["sqlite"], "browsers": ["chromium"], "storage": false, "browser": true,
            "count_overrides": {"posts": null, "authors": null, "revisions": null}});
        let producer = json!({"name": name, "identity": "/nix/store/current-browser", "producer": "browser", "backend": "sqlite", "browser": "chromium"});
        write_json(
            &root.join("selection.json"),
            &json!({"profile": "small", "selection": selection, "producers": [producer]}),
        )?;
        let mut retained = producer;
        retained["retained_root"] = json!(performance);
        let invocation = root.join("invocation.json");
        write_json(
            &invocation,
            &json!({"profile": "small", "freshness_nonce": NONCE, "selection": selection, "producers": [retained]}),
        )?;
        for arguments in [
            vec!["init", "--quiet"],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "Fixture provenance",
            ],
        ] {
            let mut git = Command::new("git");
            git.current_dir(directory.path()).args(arguments);
            for key in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_OBJECT_DIRECTORY",
                "GIT_COMMON_DIR",
                "GIT_NAMESPACE",
            ] {
                git.env_remove(key);
            }
            let output = git.output()?;
            anyhow::ensure!(
                output.status.success(),
                "fixture Git setup: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(Self {
            directory,
            invocation,
            performance,
            fragment,
        })
    }

    fn command(&self, github: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_perf-data"));
        command
            .args(["assemble", "--invocation"])
            .arg(&self.invocation);
        command.current_dir(self.directory.path());
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
        ] {
            command.env_remove(key);
        }
        for key in GITHUB_ENV {
            command.env_remove(key);
        }
        if github {
            for (key, value) in GITHUB_ENV.into_iter().zip([
                "jaunder-org/jaunder",
                "Performance",
                "performance",
                "refs/heads/main",
                "abc123",
                "12",
                "2",
            ]) {
                command.env(key, value);
            }
        }
        command
    }

    fn run(&self, github: bool) -> Result<Output> {
        Ok(self.command(github).output()?)
    }

    fn result(&self) -> PathBuf {
        self.invocation.with_file_name(RESULT_FILENAME)
    }
    fn stable(&self) -> PathBuf {
        self.directory
            .path()
            .join(".xtask/performance")
            .join(RESULT_FILENAME)
    }
    fn seed_results(&self) -> Result<()> {
        fs::write(self.result(), "previous invocation")?;
        fs::write(self.stable(), "previous stable")?;
        Ok(())
    }
    fn assert_rejected(&self, message: &str) -> Result<()> {
        self.seed_results()?;
        let output = self.run(true)?;
        assert!(
            !output.status.success(),
            "accepted invalid evidence: {message}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read_to_string(self.result())?, "previous invocation");
        assert_eq!(fs::read_to_string(self.stable())?, "previous stable");
        Ok(())
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn mutate(path: &Path, change: impl FnOnce(&mut Value)) -> Result<()> {
    let mut value: Value = serde_json::from_slice(&fs::read(path)?)?;
    change(&mut value);
    write_json(path, &value)
}

// Minimal complete input data; the production planner/statistics/validator own its contract.
fn browser_fragment() -> Result<Value> {
    Ok(serde_json::to_value(support::fragment(
        DatasetProfile::Small,
        Backend::Sqlite,
        Some(Browser::Chromium),
    )?)?)
}

#[test]
fn complete_cli_assembly_publishes_identical_valid_results_and_provenance() -> Result<()> {
    for github in [false, true] {
        let fixture = Fixture::new()?;
        fixture.seed_results()?;
        let output = fixture.run(github)?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bytes = fs::read(fixture.result())?;
        assert_eq!(bytes, fs::read(fixture.stable())?);
        let run: performance::RunEnvelope = serde_json::from_slice(&bytes)?;
        performance::validate_run(&run)?;
        assert_eq!(run.evidence.freshness_nonce, NONCE);
        assert!(!run.canonical_selection);
        assert_eq!(run.workloads.len(), 6);
        match (&run.provenance, github) {
            (performance::Provenance::Github(provenance), true) => {
                assert_eq!(provenance.repository, "jaunder-org/jaunder");
                assert_eq!(provenance.workflow, "Performance");
                assert_eq!(provenance.job, "performance");
                assert_eq!(provenance.reference, "refs/heads/main");
                assert_eq!(provenance.head_sha, "abc123");
                assert_eq!(provenance.run_id, 12);
                assert_eq!(provenance.attempt, 2);
            }
            (performance::Provenance::Local(provenance), false) => {
                assert!(!provenance.git_commit.is_empty());
            }
            _ => panic!("provenance source does not match execution context"),
        }
    }
    Ok(())
}

#[test]
fn invalid_status_and_fragments_never_replace_previous_results() -> Result<()> {
    for (field, value, error) in [
        ("schema_version", json!(2), "status schema"),
        ("ok", json!(false), "producer reported failure"),
    ] {
        let fixture = Fixture::new()?;
        mutate(&fixture.performance.join("producer-status-v1.json"), |v| {
            v[field] = value;
        })?;
        fixture.assert_rejected(error)?;
    }
    let fixture = Fixture::new()?;
    fs::remove_file(fixture.performance.join("producer-status-v1.json"))?;
    fixture.assert_rejected("producer-status-v1.json")?;
    let fixture = Fixture::new()?;
    fs::write(
        fixture.performance.join("producer-status-v1.json"),
        "not JSON",
    )?;
    fixture.assert_rejected("parsing")?;
    let fixture = Fixture::new()?;
    fs::remove_file(&fixture.fragment)?;
    fixture.assert_rejected("resolving producer fragment")?;
    for broken in [json!({}), json!({"schema_version": 99})] {
        let fixture = Fixture::new()?;
        write_json(&fixture.fragment, &broken)?;
        fixture.assert_rejected("parsing")?;
    }
    let fixture = Fixture::new()?;
    mutate(&fixture.fragment, |v| {
        v["fragment"]["result"]["workloads"]
            .as_array_mut()
            .map(Vec::pop);
        for field in ["navigation_artifacts", "trace_artifacts"] {
            if let Some(paths) = v["fragment"]["result"]["diagnostics"][field].as_array_mut() {
                paths.truncate(100);
            }
        }
    })?;
    fixture.assert_rejected("missing workload")?;
    let fixture = Fixture::new()?;
    mutate(&fixture.fragment, |v| v["schema_version"] = json!(2))?;
    fixture.assert_rejected("fragment schema")?;
    Ok(())
}

#[test]
fn stale_or_inconsistent_invocation_metadata_is_rejected() -> Result<()> {
    for (field, value, error) in [
        ("freshness_nonce", json!("stale"), "nonce"),
        ("profile", json!("medium"), "selection"),
        (
            "selection",
            json!({"backends": ["postgres"], "browsers": ["chromium"], "storage": false, "browser": true, "count_overrides": {"posts": null, "authors": null, "revisions": null}}),
            "selection",
        ),
    ] {
        let fixture = Fixture::new()?;
        mutate(&fixture.invocation, |v| v[field] = value)?;
        fixture.assert_rejected(error)?;
    }
    for (field, value, error) in [
        ("identity", json!("/nix/store/stale-browser"), "identity"),
        ("browser", json!("firefox"), "identity"),
        ("backend", json!("postgres"), "identity"),
        ("name", json!("nested/producer"), "name"),
        (
            "retained_root",
            json!("/foreign/performance"),
            "retained producer root",
        ),
    ] {
        let fixture = Fixture::new()?;
        mutate(&fixture.invocation, |v| v["producers"][0][field] = value)?;
        fixture.assert_rejected(error)?;
    }
    let fixture = Fixture::new()?;
    mutate(&fixture.invocation, |v| {
        let p = v["producers"][0].clone();
        if let Some(producers) = v["producers"].as_array_mut() {
            producers.push(p);
        }
    })?;
    fixture.assert_rejected("selection")?;
    Ok(())
}

#[test]
fn diagnostics_must_exist_and_remain_confined() -> Result<()> {
    for reference in ["../escape", "/absolute", "missing.json"] {
        let fixture = Fixture::new()?;
        mutate(&fixture.fragment, |v| {
            v["fragment"]["result"]["diagnostics"]["otel_trace_artifact"] = json!(reference);
        })?;
        fixture.assert_rejected(if reference == "missing.json" {
            "resolving browser diagnostic"
        } else {
            "relative fragment path"
        })?;
    }
    let fixture = Fixture::new()?;
    let trace = fixture.performance.join("fragments/otel-traces.jsonl");
    fs::remove_file(&trace)?;
    let outside = fixture.directory.path().join("outside");
    fs::write(&outside, "outside")?;
    std::os::unix::fs::symlink(outside, trace)?;
    fixture.assert_rejected("outside producer fragments")?;
    Ok(())
}

#[test]
fn plan_selection_and_browser_identity_must_agree_with_fragments() -> Result<()> {
    let fixture = Fixture::new()?;
    mutate(&fixture.fragment, |v| {
        v["manifest"]["plan"]["posts"] = json!(0);
    })?;
    fixture.assert_rejected("manifest is invalid")?;

    let fixture = Fixture::new()?;
    mutate(&fixture.fragment, |v| {
        v["manifest"]["plan"] = json!(canonical_plan(DatasetProfile::Medium));
        if let Some(workloads) = v["fragment"]["result"]["workloads"].as_array_mut() {
            for item in workloads {
                item["key"]["profile"] = json!("medium");
            }
        }
    })?;
    let fragment: performance::FragmentEnvelope =
        serde_json::from_slice(&fs::read(&fixture.fragment)?)?;
    performance::validate_fragment(&fragment)?;
    fixture.assert_rejected("producer profile does not match invocation")?;

    let fixture = Fixture::new()?;
    for path in [
        &fixture.invocation,
        &fixture.invocation.with_file_name("selection.json"),
    ] {
        mutate(path, |v| {
            v["selection"]["count_overrides"] =
                json!({"posts": 120, "authors": 12, "revisions": 777});
        })?;
    }
    fixture.assert_rejected("run selection is invalid")?;

    let fixture = Fixture::new()?;
    for path in [
        &fixture.invocation,
        &fixture.invocation.with_file_name("selection.json"),
    ] {
        mutate(path, |v| {
            v["selection"]["browsers"] = json!(["firefox"]);
            v["producers"][0]["browser"] = json!("firefox");
        })?;
    }
    fs::rename(
        &fixture.fragment,
        fixture
            .fragment
            .with_file_name("browser-sqlite-firefox-v1.json"),
    )?;
    fixture.assert_rejected("fragment browser does not match selection")?;
    Ok(())
}

#[test]
fn different_valid_manifests_cannot_be_assembled_together() -> Result<()> {
    let fixture = Fixture::new()?;
    let name = "performance-browser-sqlite-firefox";
    let root = fixture.invocation.parent().context("invocation parent")?;
    let performance = root.join("producers").join(name).join("performance");
    fs::create_dir_all(performance.join("fragments"))?;
    fs::copy(
        fixture.performance.join("producer-status-v1.json"),
        performance.join("producer-status-v1.json"),
    )?;
    for entry in fs::read_dir(fixture.performance.join("fragments"))? {
        let entry = entry?;
        fs::copy(
            entry.path(),
            performance.join("fragments").join(entry.file_name()),
        )?;
    }
    let fragment = performance.join("fragments/browser-sqlite-firefox-v1.json");
    fs::rename(
        performance.join("fragments/browser-sqlite-chromium-v1.json"),
        &fragment,
    )?;
    mutate(&fragment, |v| {
        v["manifest"]["subjects"]["username"] = json!("another-user");
        if let Some(workloads) = v["fragment"]["result"]["workloads"].as_array_mut() {
            for item in workloads {
                item["key"]["browser"] = json!("firefox");
            }
        }
    })?;
    for path in [&fixture.invocation, &root.join("selection.json")] {
        mutate(path, |v| {
            v["selection"]["browsers"] = json!(["chromium", "firefox"]);
            let mut producer = v["producers"][0].clone();
            producer["name"] = json!(name);
            producer["identity"] = json!("/nix/store/current-firefox");
            producer["browser"] = json!("firefox");
            if producer.get("retained_root").is_some() {
                producer["retained_root"] = json!(performance);
            }
            if let Some(producers) = v["producers"].as_array_mut() {
                producers.push(producer);
            }
        })?;
    }
    fixture.assert_rejected("fragment manifests differ")?;
    Ok(())
}

#[test]
fn foreign_retained_tree_and_missing_selection_are_rejected() -> Result<()> {
    let fixture = Fixture::new()?;
    let outside = fixture.directory.path().join("foreign-performance");
    fs::rename(&fixture.performance, &outside)?;
    std::os::unix::fs::symlink(outside, &fixture.performance)?;
    fixture.assert_rejected("retained producer root is outside invocation")?;
    let fixture = Fixture::new()?;
    fs::remove_file(fixture.invocation.with_file_name("selection.json"))?;
    fixture.assert_rejected("selection.json")?;
    Ok(())
}

#[test]
fn incomplete_or_invalid_github_provenance_never_falls_back_to_local() -> Result<()> {
    for key in GITHUB_ENV.into_iter().skip(1) {
        let fixture = Fixture::new()?;
        fixture.seed_results()?;
        let output = fixture.command(true).env_remove(key).output()?;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(key));
        assert_eq!(fs::read_to_string(fixture.result())?, "previous invocation");
        assert_eq!(fs::read_to_string(fixture.stable())?, "previous stable");
    }
    let fixture = Fixture::new()?;
    let output = fixture.command(true).env("GITHUB_RUN_ID", "0").output()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("provenance is invalid"));
    Ok(())
}

#[test]
fn publication_failure_never_truncates_either_destination() -> Result<()> {
    let fixture = Fixture::new()?;
    fixture.seed_results()?;
    fs::remove_file(fixture.result())?;
    fs::create_dir(fixture.result())?;
    let output = fixture.run(true)?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("publishing"));
    assert!(fixture.result().is_dir());
    assert_eq!(fs::read_to_string(fixture.stable())?, "previous stable");

    let fixture = Fixture::new()?;
    fs::create_dir(fixture.stable())?;
    let output = fixture.run(true)?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("publishing"));
    // Publication is atomic per file, as in xtask, not a two-file transaction.
    let complete: performance::RunEnvelope = serde_json::from_slice(&fs::read(fixture.result())?)?;
    performance::validate_run(&complete)?;
    assert!(fixture.stable().is_dir());
    assert_eq!(
        fs::read_dir(fixture.stable().parent().context("evidence root")?)?.count(),
        2
    );
    Ok(())
}

#[test]
fn nonce_command_returns_fresh_invocation_identifiers() -> Result<()> {
    let mut values = Vec::new();
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_perf-data"))
            .arg("nonce")
            .output()?;
        assert!(output.status.success());
        let value = String::from_utf8(output.stdout)?;
        assert_eq!(value.trim().len(), 64);
        assert!(value.trim().bytes().all(|byte| byte.is_ascii_hexdigit()));
        values.push(value);
    }
    assert_ne!(values[0], values[1]);
    Ok(())
}
