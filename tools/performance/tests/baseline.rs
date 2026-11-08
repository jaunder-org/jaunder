#![cfg(unix)]

mod support;
use anyhow::{Context, Result};
use performance::{
    Backend, Baseline, Browser, CountOverrides, DatasetProfile, GitHubProvenance,
    NamedDerivationIdentity, Provenance, RunEnvelope, RunEvidence, RunSelection, assemble_run,
};
use serde_json::{Value, json};
use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

fn write_executable(path: &Path, contents: &str) -> Result<()> {
    // Generated scripts are not visited by Nix's patchShebangs hook. Resolve
    // the supplied Bash instead of assuming /usr/bin/env exists in the sandbox.
    let bash = env::split_paths(&env::var_os("PATH").context("test PATH")?)
        .map(|directory| directory.join("bash"))
        .find(|path| path.is_file())
        .context("fixture requires Bash on PATH")?;
    let bash = fs::canonicalize(bash)?;
    let contents = contents.replacen("#!/usr/bin/env bash", &format!("#!{}", bash.display()), 1);
    fs::write(path, contents)?;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn canonical_run() -> Result<RunEnvelope> {
    run_for_profile(DatasetProfile::Medium)
}

fn run_for_profile(profile: DatasetProfile) -> Result<RunEnvelope> {
    let mut fragments = Vec::new();
    let mut identities = Vec::new();
    for backend in [Backend::Sqlite, Backend::Postgres] {
        for browser in [None, Some(Browser::Chromium)] {
            fragments.push(support::fragment(profile, backend, browser)?);
            identities.push(NamedDerivationIdentity {
                name: format!("{backend:?}-{browser:?}"),
                identity: "/nix/store/fresh-producer".into(),
            });
        }
    }
    identities.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(assemble_run(
        RunSelection {
            backends: vec![Backend::Sqlite, Backend::Postgres],
            browsers: vec![Browser::Chromium],
            count_overrides: CountOverrides::default(),
            storage: true,
            browser: true,
        },
        Provenance::Github(GitHubProvenance {
            repository: "jaunder-org/jaunder".into(),
            workflow: "Performance".into(),
            job: "performance".into(),
            reference: "refs/heads/main".into(),
            head_sha: "abc123".into(),
            run_id: 42,
            attempt: 2,
        }),
        RunEvidence {
            measured_at_unix_ms: 2,
            freshness_nonce: "fresh".into(),
            producer_derivation_identities: identities,
        },
        &fragments,
    )?)
}

struct Fixture {
    directory: TempDir,
    baseline: PathBuf,
    run: PathBuf,
}
impl Fixture {
    fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let fake_bin = root.join("bin");
        fs::create_dir_all(&fake_bin)?;
        let baseline = root.join("tools/performance/baseline-v1.json");
        fs::create_dir_all(baseline.parent().context("baseline parent")?)?;
        let run = root.join("result.json");
        let candidate = canonical_run()?;
        let mut previous = candidate.clone();
        previous.evidence.measured_at_unix_ms = 1;
        write_json(
            &baseline,
            &json!(Baseline {
                schema_version: 1,
                run: previous
            }),
        )?;
        write_json(&run, &json!(candidate))?;
        write_json(
            &root.join("metadata.json"),
            &json!({"id": 42, "repository": {"full_name": "jaunder-org/jaunder"},
            "head_repository": {"full_name": "jaunder-org/jaunder"}, "path": ".github/workflows/performance.yml", "name": "Performance",
            "event": "workflow_dispatch", "status": "completed", "conclusion": "success", "head_branch": "main", "head_sha": "abc123", "run_attempt": 2}),
        )?;
        write_json(
            &root.join("jobs.json"),
            &json!({"jobs": [{"name": "performance", "status": "completed", "conclusion": "success", "head_sha": "abc123", "run_attempt": 2}]}),
        )?;
        write_json(
            &root.join("artifacts.json"),
            &json!({"artifacts": [{"name": "performance-result", "expired": false, "id": 7, "size_in_bytes": 100,
            "workflow_run": {"id": 42, "head_branch": "main", "head_sha": "abc123"}}]}),
        )?;
        let gh = fake_bin.join("gh");
        write_executable(
            &gh,
            r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$PERF_FIXTURE/calls"
if [[ ${1-} == api ]]; then
  [[ ${PERF_FAIL-} != api ]] || exit 73
  case "$2" in
    */jobs) source=jobs.json ;;
    */artifacts) source=artifacts.json ;;
    /repos/jaunder-org/jaunder/actions/runs/42) source=metadata.json ;;
    *) exit 91 ;;
  esac
  cat "$PERF_FIXTURE/$source"
  exit 0
fi
[[ ${1-} == run && ${2-} == download ]] || exit 92
[[ ${PERF_FAIL-} != download ]] || exit 74
[[ ${PERF_FAIL-} != missing ]] || exit 0
cp "$PERF_FIXTURE/result.json" "${!#}/performance-result-v1.json"
"#,
        )?;
        let cargo = fake_bin.join("cargo");
        write_executable(
            &cargo,
            "#!/usr/bin/env bash\nset -euo pipefail\nwhile (($#)); do\n  if [[ $1 == -- ]]; then shift; exec \"$PERF_DATA_BIN\" \"$@\"; fi\n  shift\ndone\nexit 93\n",
        )?;
        Ok(Self {
            directory,
            baseline,
            run,
        })
    }
    fn command(&self) -> Result<Command> {
        let mut command = Command::new(env!("CARGO_BIN_EXE_perf-data"));
        command
            .current_dir(self.directory.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.directory.path().join("bin").display(),
                    env::var("PATH")?
                ),
            )
            .env("PERF_FIXTURE", self.directory.path())
            .env_remove("PERF_FAIL");
        Ok(command)
    }
    fn import(&self) -> Result<Output> {
        Ok(self.command()?.args(["import-baseline", "42"]).output()?)
    }
    fn just(&self, recipe: &str, argument: &str) -> Result<Output> {
        Ok(Command::new("just")
            .arg("--justfile")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Justfile"))
            .arg("--working-directory")
            .arg(self.directory.path())
            .args([recipe, argument])
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.directory.path().join("bin").display(),
                    env::var("PATH")?
                ),
            )
            .env("PERF_FIXTURE", self.directory.path())
            .env("PERF_DATA_BIN", env!("CARGO_BIN_EXE_perf-data"))
            .env_remove("PERF_FAIL")
            .output()?)
    }
    fn compare(&self) -> Result<Output> {
        Ok(self
            .command()?
            .arg("compare")
            .arg("--result")
            .arg(&self.run)
            .output()?)
    }
    fn assert_rejected(&self, message: &str) -> Result<()> {
        let before = fs::read(&self.baseline)?;
        let output = self.import()?;
        assert!(!output.status.success(), "accepted invalid {message}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(&self.baseline)?, before);
        Ok(())
    }
}
fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn change(path: &Path, pointer: &str, value: Value) -> Result<()> {
    let mut document: Value = serde_json::from_slice(&fs::read(path)?)?;
    *document
        .pointer_mut(pointer)
        .context("fixture JSON pointer")? = value;
    write_json(path, &document)
}

#[test]
fn valid_trusted_canonical_import_atomically_replaces_baseline() -> Result<()> {
    for event in ["workflow_dispatch", "schedule"] {
        let fixture = Fixture::new()?;
        change(
            &fixture.directory.path().join("metadata.json"),
            "/event",
            json!(event),
        )?;
        let before = fs::read(&fixture.baseline)?;
        let output = fixture.import()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bytes = fs::read(&fixture.baseline)?;
        assert_ne!(bytes, before);
        let baseline: Baseline = serde_json::from_slice(&bytes)?;
        assert_eq!(baseline.run, canonical_run()?);
        performance::validate_run(&baseline.run)?;
        assert_eq!(
            fs::read_dir(fixture.baseline.parent().context("baseline parent")?)?.count(),
            1
        );
        let calls = fs::read_to_string(fixture.directory.path().join("calls"))?;
        assert!(calls.contains(
            "run download 42 --repo jaunder-org/jaunder --name performance-result --dir "
        ));
    }
    let fixture = Fixture::new()?;
    fs::remove_file(&fixture.baseline)?;
    assert!(fixture.import()?.status.success());
    let baseline: Baseline = serde_json::from_slice(&fs::read(&fixture.baseline)?)?;
    assert_eq!(baseline.run, canonical_run()?);
    Ok(())
}

#[test]
fn each_run_metadata_predicate_rejects_before_download_and_preserves_baseline() -> Result<()> {
    for (file, pointer, value, error) in [
        ("metadata", "/id", json!(43), "run ID mismatch"),
        (
            "metadata",
            "/repository/full_name",
            json!("other/repo"),
            "another repository",
        ),
        (
            "metadata",
            "/head_repository/full_name",
            json!("fork/repo"),
            "another repository",
        ),
        (
            "metadata",
            "/path",
            json!(".github/workflows/other.yml"),
            "approved performance workflow",
        ),
        (
            "metadata",
            "/name",
            json!("Other"),
            "approved performance workflow",
        ),
        (
            "metadata",
            "/event",
            json!("pull_request"),
            "unapproved workflow events",
        ),
        (
            "metadata",
            "/event",
            json!("push"),
            "unapproved workflow events",
        ),
        (
            "metadata",
            "/status",
            json!("in_progress"),
            "completed successful run",
        ),
        (
            "metadata",
            "/conclusion",
            json!("failure"),
            "completed successful run",
        ),
        (
            "metadata",
            "/head_branch",
            json!("feature"),
            "requires main",
        ),
        ("metadata", "/head_sha", json!("  "), "no head SHA"),
        ("metadata", "/run_attempt", json!(0), "no attempt"),
    ] {
        let fixture = Fixture::new()?;
        change(
            &fixture.directory.path().join(format!("{file}.json")),
            pointer,
            value,
        )?;
        fixture.assert_rejected(error)?;
        assert!(!fs::read_to_string(fixture.directory.path().join("calls"))?.contains("download"));
    }
    Ok(())
}

#[test]
fn each_job_predicate_rejects_before_download_and_preserves_baseline() -> Result<()> {
    for (file, pointer, value, error) in [
        ("jobs", "/jobs", json!([]), "approved performance job"),
        ("jobs", "/jobs", json!(null), "approved performance job"),
        (
            "jobs",
            "/jobs/0/name",
            json!("other"),
            "approved performance job",
        ),
        (
            "jobs",
            "/jobs/0/status",
            json!("in_progress"),
            "approved performance job",
        ),
        (
            "jobs",
            "/jobs/0/conclusion",
            json!("failure"),
            "approved performance job",
        ),
        (
            "jobs",
            "/jobs/0/head_sha",
            json!("other-head"),
            "approved performance job",
        ),
        (
            "jobs",
            "/jobs/0/run_attempt",
            json!(1),
            "approved performance job",
        ),
    ] {
        let fixture = Fixture::new()?;
        change(
            &fixture.directory.path().join(format!("{file}.json")),
            pointer,
            value,
        )?;
        fixture.assert_rejected(error)?;
        assert!(!fs::read_to_string(fixture.directory.path().join("calls"))?.contains("download"));
    }
    Ok(())
}

#[test]
fn each_artifact_predicate_rejects_before_download_and_preserves_baseline() -> Result<()> {
    for (file, pointer, value, error) in [
        (
            "artifacts",
            "/artifacts",
            json!([]),
            "complete performance-result artifact",
        ),
        (
            "artifacts",
            "/artifacts",
            json!(null),
            "complete performance-result artifact",
        ),
        (
            "artifacts",
            "/artifacts/0/name",
            json!("other"),
            "complete performance-result artifact",
        ),
        (
            "artifacts",
            "/artifacts/0/expired",
            json!(true),
            "complete performance-result artifact",
        ),
        (
            "artifacts",
            "/artifacts/0/id",
            json!(0),
            "complete performance-result artifact",
        ),
        (
            "artifacts",
            "/artifacts/0/size_in_bytes",
            json!(0),
            "complete performance-result artifact",
        ),
        (
            "artifacts",
            "/artifacts/0/workflow_run/id",
            json!(43),
            "complete performance-result artifact",
        ),
        (
            "artifacts",
            "/artifacts/0/workflow_run/head_branch",
            json!("feature"),
            "complete performance-result artifact",
        ),
        (
            "artifacts",
            "/artifacts/0/workflow_run/head_sha",
            json!("other-head"),
            "complete performance-result artifact",
        ),
    ] {
        let fixture = Fixture::new()?;
        change(
            &fixture.directory.path().join(format!("{file}.json")),
            pointer,
            value,
        )?;
        fixture.assert_rejected(error)?;
        assert!(
            !fs::read_to_string(fixture.directory.path().join("calls"))?.contains("download"),
            "download preceded admission: {pointer}"
        );
    }
    Ok(())
}

#[test]
fn each_imported_identity_predicate_preserves_baseline_on_rejection() -> Result<()> {
    for (pointer, value, error) in [
        (
            "/selection/backends",
            json!(["sqlite"]),
            "noncanonical selection",
        ),
        (
            "/selection/browsers",
            json!(["firefox"]),
            "noncanonical selection",
        ),
        ("/selection/storage", json!(false), "noncanonical selection"),
        ("/selection/browser", json!(false), "noncanonical selection"),
        (
            "/selection/count_overrides/posts",
            json!(5000),
            "noncanonical selection",
        ),
        (
            "/provenance",
            json!({"source": "local", "details": {"git_commit": "abc123"}}),
            "local provenance",
        ),
        (
            "/provenance/details/repository",
            json!("other/repo"),
            "artifact provenance",
        ),
        (
            "/provenance/details/workflow",
            json!("Other"),
            "artifact provenance",
        ),
        (
            "/provenance/details/job",
            json!("other"),
            "artifact provenance",
        ),
        (
            "/provenance/details/reference",
            json!("refs/heads/feature"),
            "artifact provenance",
        ),
        (
            "/provenance/details/run_id",
            json!(41),
            "artifact provenance",
        ),
        (
            "/provenance/details/head_sha",
            json!("other-head"),
            "artifact provenance",
        ),
        (
            "/provenance/details/attempt",
            json!(1),
            "artifact provenance",
        ),
    ] {
        let fixture = Fixture::new()?;
        change(&fixture.run, pointer, value)?;
        fixture.assert_rejected(error)?;
    }
    for profile in [DatasetProfile::Small, DatasetProfile::Large] {
        let fixture = Fixture::new()?;
        write_json(&fixture.run, &json!(run_for_profile(profile)?))?;
        fixture.assert_rejected("requires the medium profile")?;
    }
    Ok(())
}

#[test]
fn invalid_incomplete_or_incompatible_evidence_preserves_baseline() -> Result<()> {
    for (pointer, value, error) in [
        (
            "/schema_version",
            json!(2),
            "fragment schema is unsupported",
        ),
        (
            "/canonical_selection",
            json!(false),
            "run selection is invalid",
        ),
        ("/manifest/plan/posts", json!(0), "manifest is invalid"),
        ("/setup", json!([]), "setup records"),
        ("/workloads", json!([]), "missing workload"),
        ("/evidence/freshness_nonce", json!(""), "freshness evidence"),
        (
            "/evidence/measured_at_unix_ms",
            json!(0),
            "freshness evidence",
        ),
        (
            "/evidence/producer_derivation_identities",
            json!([]),
            "freshness evidence",
        ),
        (
            "/workloads/0/key/runner_image",
            json!("different-image"),
            "compatibility keys differ",
        ),
        (
            "/workloads/0/summary/median_us",
            json!(99),
            "workload is incompatible",
        ),
    ] {
        let fixture = Fixture::new()?;
        change(&fixture.run, pointer, value)?;
        fixture.assert_rejected(error)?;
    }
    let fixture = Fixture::new()?;
    fs::write(&fixture.run, "not JSON")?;
    fixture.assert_rejected("parsing")?;
    for (pointer, value, error) in [
        ("/schema_version", json!(2), "existing baseline schema"),
        ("/run/workloads", json!([]), "existing baseline violates"),
    ] {
        let fixture = Fixture::new()?;
        change(&fixture.baseline, pointer, value)?;
        fixture.assert_rejected(error)?;
    }
    let fixture = Fixture::new()?;
    fs::write(&fixture.baseline, "not JSON")?;
    fixture.assert_rejected("parsing")?;
    Ok(())
}

#[test]
fn transport_and_artifact_failures_and_api_error_bodies_preserve_baseline() -> Result<()> {
    for mode in ["api", "download", "missing"] {
        let fixture = Fixture::new()?;
        let before = fs::read(&fixture.baseline)?;
        let output = fixture
            .command()?
            .env("PERF_FAIL", mode)
            .args(["import-baseline", "42"])
            .output()?;
        assert!(!output.status.success());
        assert_eq!(fs::read(&fixture.baseline)?, before);
    }
    let fixture = Fixture::new()?;
    let path = fixture.directory.path().join("metadata.json");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&path)?)?;
    metadata["errors"] = json!([{"message": "injected API error"}]);
    write_json(&path, &metadata)?;
    fixture.assert_rejected("API response contains errors")?;
    let fixture = Fixture::new()?;
    fs::write(fixture.directory.path().join("metadata.json"), "not JSON")?;
    fixture.assert_rejected("parsing GitHub API response")?;
    Ok(())
}

#[test]
fn comparison_reports_absolute_statistics_and_advisory_regressions_without_mutation() -> Result<()>
{
    let fixture = Fixture::new()?;
    let baseline = fs::read(&fixture.baseline)?;
    let mut run = canonical_run()?;
    for workload in &mut run.workloads {
        for sample in &mut workload.samples {
            sample.duration_us *= 2;
        }
        workload.summary = performance::summarize(&workload.samples)?;
    }
    write_json(&fixture.run, &json!(run))?;
    let output = fixture.compare()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout)?;
    assert!(text.contains("advisory only"));
    assert!(text.contains("baseline median=1us p95=1us spread=1..1us"));
    assert!(text.contains("candidate median=2us p95=2us spread=2..2us"));
    assert!(text.contains("AtLeastTwentyPercent"));
    assert_eq!(fs::read(&fixture.baseline)?, baseline);
    write_json(&fixture.run, &json!(canonical_run()?))?;
    let output = fixture.compare()?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("median=None p95=None"));
    let mut noncanonical = canonical_run()?;
    noncanonical.selection.count_overrides.posts = Some(noncanonical.manifest.plan.posts);
    noncanonical.canonical_selection = false;
    write_json(&fixture.run, &json!(noncanonical))?;
    assert!(fixture.compare()?.status.success());
    assert_eq!(fs::read(&fixture.baseline)?, baseline);
    Ok(())
}

#[test]
fn comparison_rejects_malformed_incomplete_and_incompatible_evidence() -> Result<()> {
    for (pointer, value, error) in [
        ("/workloads", json!([]), "candidate run violates"),
        (
            "/workloads/0/key/runner_image",
            json!("different"),
            "compatibility keys differ",
        ),
    ] {
        let fixture = Fixture::new()?;
        change(&fixture.run, pointer, value)?;
        let output = fixture.compare()?;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(error));
    }
    let fixture = Fixture::new()?;
    write_json(
        &fixture.run,
        &json!(run_for_profile(DatasetProfile::Small)?),
    )?;
    let output = fixture.compare()?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("dataset plan differs"));
    let fixture = Fixture::new()?;
    fs::write(&fixture.run, "not JSON")?;
    assert!(!fixture.compare()?.status.success());
    for (pointer, value) in [("/schema_version", json!(2)), ("/run/workloads", json!([]))] {
        let fixture = Fixture::new()?;
        change(&fixture.baseline, pointer, value)?;
        assert!(!fixture.compare()?.status.success());
    }
    Ok(())
}

#[test]
fn native_just_parameters_reach_the_real_compare_and_import_leaves() -> Result<()> {
    let fixture = Fixture::new()?;
    let spaced_path = fixture.directory.path().join("candidate evidence.json");
    fs::copy(&fixture.run, &spaced_path)?;
    let output = fixture.just(
        "perf-compare",
        spaced_path.to_str().context("fixture path")?,
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("advisory only"));
    let before = fs::read(&fixture.baseline)?;
    let output = fixture.just("perf-import-baseline", "0")?;
    assert!(!output.status.success());
    assert_eq!(fs::read(&fixture.baseline)?, before);
    let output = fixture.just("perf-import-baseline", "42")?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let baseline: Baseline = serde_json::from_slice(&fs::read(&fixture.baseline)?)?;
    assert_eq!(baseline.run, canonical_run()?);
    Ok(())
}

#[test]
fn import_requires_a_positive_run_id_before_transport() -> Result<()> {
    for run_id in ["0", "-1", "abc", "18446744073709551616"] {
        let fixture = Fixture::new()?;
        let before = fs::read(&fixture.baseline)?;
        let output = fixture
            .command()?
            .args(["import-baseline", run_id])
            .output()?;
        assert!(!output.status.success());
        assert_eq!(fs::read(&fixture.baseline)?, before);
        assert!(!fixture.directory.path().join("calls").exists());
    }
    Ok(())
}
