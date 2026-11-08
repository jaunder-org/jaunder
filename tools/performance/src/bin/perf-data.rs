#[path = "perf-data/baseline.rs"]
mod baseline;

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand, ValueEnum};
use performance::{
    Backend, Browser, BrowserDiagnostics, DatasetProfile, Fragment, FragmentEnvelope,
    LocalProvenance, NamedDerivationIdentity, Producer, Provenance, RESULT_FILENAME, RunEvidence,
    RunSelection, assemble_run, browser_fragment_filename, is_canonical_selection,
    is_valid_selection, plan, storage_fragment_filename, validate_fragment,
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    collections::BTreeSet,
    env, fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Parser)]
#[command(name = "perf-data")]
struct Cli {
    #[command(subcommand)]
    command: CommandLine,
}

#[derive(Subcommand)]
enum CommandLine {
    /// Compare a completed result with the committed baseline; timing deltas are advisory.
    Compare {
        #[arg(long)]
        result: PathBuf,
    },
    /// Import a canonical baseline from an explicitly authenticated Actions run.
    ImportBaseline {
        #[arg(value_parser = clap::value_parser!(u64).range(1..))]
        run_id: u64,
    },
    /// Produce a fresh invocation identifier for the Nix derivations.
    Nonce,
    /// Validate selection and count relationships before Just starts producer builds.
    ValidateSelection {
        #[arg(long)]
        selection: PathBuf,
    },
    /// Validate retained status, fragment, and diagnostic files before Just schedules another producer.
    ValidateProducer {
        #[arg(long)]
        performance_root: PathBuf,
        #[arg(long, value_enum)]
        producer: ProducerArg,
        #[arg(long, value_enum)]
        backend: BackendArg,
        #[arg(long, value_enum)]
        browser: Option<BrowserArg>,
    },
    /// Assemble selected, retained producer outputs into the canonical run artifact.
    Assemble {
        #[arg(long)]
        invocation: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ProducerArg {
    Storage,
    Browser,
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendArg {
    Sqlite,
    Postgres,
}

#[derive(Clone, Copy, ValueEnum)]
enum BrowserArg {
    Chromium,
    Firefox,
}

impl From<ProducerArg> for Producer {
    fn from(value: ProducerArg) -> Self {
        match value {
            ProducerArg::Storage => Self::Storage,
            ProducerArg::Browser => Self::Browser,
        }
    }
}

impl From<BackendArg> for Backend {
    fn from(value: BackendArg) -> Self {
        match value {
            BackendArg::Sqlite => Self::Sqlite,
            BackendArg::Postgres => Self::Postgres,
        }
    }
}

impl From<BrowserArg> for Browser {
    fn from(value: BrowserArg) -> Self {
        match value {
            BrowserArg::Chromium => Self::Chromium,
            BrowserArg::Firefox => Self::Firefox,
        }
    }
}

#[derive(Deserialize)]
struct ProducerStatus {
    schema_version: u64,
    ok: bool,
    detail: String,
}

#[derive(Deserialize)]
struct SelectionDocument {
    profile: DatasetProfile,
    selection: RunSelection,
    #[serde(default)]
    producers: Vec<SelectedProducer>,
}

// Only identity metadata is consumed here; Nix installables belong to Just.
#[derive(Deserialize)]
struct SelectedProducer {
    name: String,
    identity: String,
    producer: Producer,
    backend: Backend,
    browser: Option<Browser>,
}

#[derive(Deserialize)]
struct Invocation {
    profile: DatasetProfile,
    freshness_nonce: String,
    selection: RunSelection,
    producers: Vec<RetainedProducer>,
}

#[derive(Deserialize)]
struct RetainedProducer {
    name: String,
    identity: String,
    producer: Producer,
    backend: Backend,
    browser: Option<Browser>,
    retained_root: PathBuf,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        CommandLine::Compare { result } => baseline::compare(&result),
        CommandLine::ImportBaseline { run_id } => baseline::import(run_id),
        CommandLine::Nonce => {
            println!("{}", freshness_nonce()?);
            Ok(())
        }
        CommandLine::ValidateSelection { selection } => validate_selection(&selection),
        CommandLine::ValidateProducer {
            performance_root,
            producer,
            backend,
            browser,
        } => validate_producer_root(
            &performance_root,
            producer.into(),
            backend.into(),
            browser.map(Into::into),
        )
        .map(drop),
        CommandLine::Assemble { invocation } => assemble(&invocation),
    }
}

fn validate_selection(path: &Path) -> Result<()> {
    // Reject invalid fixture plans before Just realizes the first VM producer.
    let document: SelectionDocument = read_json(path)?;
    ensure!(
        is_valid_selection(&document.selection),
        "Nix selected an invalid performance producer selection"
    );
    plan(document.profile, document.selection.count_overrides)
        .context("invalid performance count overrides")?;
    Ok(())
}

fn validate_producer_root(
    performance_root: &Path,
    producer: Producer,
    backend: Backend,
    browser: Option<Browser>,
) -> Result<FragmentEnvelope> {
    ensure!(
        performance_root.is_dir(),
        "producer performance output is missing"
    );
    // A completed Nix derivation can still report a failed workload in its status file.
    let status: ProducerStatus = read_json(&performance_root.join("producer-status-v1.json"))?;
    ensure!(
        status.schema_version == 1,
        "producer status schema is unsupported"
    );
    ensure!(status.ok, "producer reported failure: {}", status.detail);

    // Select the fragment only after validating the same identity tuple; a
    // second match after separate guards would contain an impossible fallback.
    let filename = match (producer, browser) {
        (Producer::Storage, None) => storage_fragment_filename(backend),
        (Producer::Storage, Some(_)) => bail!("storage producer cannot select a browser"),
        (Producer::Browser, Some(browser)) => browser_fragment_filename(backend, browser),
        (Producer::Browser, None) => bail!("browser producer requires a browser"),
    };
    let performance_root =
        fs::canonicalize(performance_root).context("resolving retained producer")?;
    let fragments_root = fs::canonicalize(performance_root.join("fragments"))
        .context("resolving producer fragments")?;
    let fragment_path =
        fs::canonicalize(fragments_root.join(filename)).context("resolving producer fragment")?;
    ensure!(
        fragments_root.starts_with(&performance_root) && fragment_path.starts_with(&fragments_root),
        "producer fragment is outside retained performance evidence"
    );
    let fragment: FragmentEnvelope = read_json(&fragment_path)?;
    validate_fragment(&fragment).context("producer fragment violates the shared contract")?;
    let (actual_producer, setup) = match &fragment.fragment {
        Fragment::Storage(value) => (Producer::Storage, &value.setup),
        Fragment::Browser(value) => (Producer::Browser, &value.setup),
    };
    ensure!(
        actual_producer == producer,
        "producer fragment kind does not match selection"
    );
    ensure!(
        setup.backend == backend,
        "producer fragment backend does not match selection"
    );
    if let Fragment::Browser(value) = &fragment.fragment {
        ensure!(
            value
                .workloads
                .iter()
                .all(|item| item.key.browser == browser),
            "producer fragment browser does not match selection"
        );
        validate_diagnostic_artifacts(&fragments_root, &value.diagnostics)?;
    }
    Ok(fragment)
}

fn validate_diagnostic_artifacts(
    fragments_root: &Path,
    diagnostics: &BrowserDiagnostics,
) -> Result<()> {
    let fragments_root =
        fs::canonicalize(fragments_root).context("resolving producer fragments")?;
    for reference in diagnostics
        .navigation_artifacts
        .iter()
        .chain(&diagnostics.trace_artifacts)
        .chain(std::iter::once(&diagnostics.otel_trace_artifact))
    {
        let relative = Path::new(reference);
        ensure!(
            !reference.is_empty()
                && relative
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
            "browser diagnostic artifact must be a relative fragment path: {reference}"
        );
        let resolved = fs::canonicalize(fragments_root.join(relative))
            .with_context(|| format!("resolving browser diagnostic artifact {reference}"))?;
        ensure!(
            resolved.is_file() && resolved.starts_with(&fragments_root),
            "browser diagnostic artifact is outside producer fragments: {reference}"
        );
    }
    Ok(())
}

fn assemble(invocation_path: &Path) -> Result<()> {
    let invocation: Invocation = read_json(invocation_path)?;
    ensure!(
        !invocation.freshness_nonce.trim().is_empty(),
        "invocation nonce is blank"
    );
    ensure!(
        !invocation.producers.is_empty(),
        "invocation selected no producers"
    );
    let run_root = invocation_path
        .parent()
        .context("invocation path has no parent")?;

    ensure!(
        run_root.file_name().and_then(|name| name.to_str()) == Some(&invocation.freshness_nonce),
        "invocation nonce does not match its retained directory"
    );
    let selection: SelectionDocument = read_json(&run_root.join("selection.json"))?;
    ensure!(
        selection.profile == invocation.profile && selection.selection == invocation.selection,
        "invocation selection differs from Nix selection"
    );
    ensure!(
        selection.producers.len() == invocation.producers.len(),
        "retained producer set differs from Nix selection"
    );
    let canonical_run_root =
        fs::canonicalize(run_root).context("resolving invocation directory")?;
    let mut fragments = Vec::with_capacity(invocation.producers.len());
    let mut identities = Vec::with_capacity(invocation.producers.len());
    let mut names = BTreeSet::new();
    for producer in &invocation.producers {
        ensure!(
            names.insert(&producer.name),
            "duplicate producer name {}",
            producer.name
        );
        validate_retained_identity(
            producer,
            &selection.producers,
            run_root,
            &canonical_run_root,
        )?;
        let fragment = validate_producer_root(
            &producer.retained_root,
            producer.producer,
            producer.backend,
            producer.browser,
        )?;
        ensure!(
            fragment.manifest.plan.profile == invocation.profile,
            "producer profile does not match invocation"
        );
        identities.push(NamedDerivationIdentity {
            name: producer.name.clone(),
            identity: producer.identity.clone(),
        });
        fragments.push(fragment);
    }
    identities.sort_by(|left, right| left.name.cmp(&right.name));

    let run = assemble_run(
        invocation.selection,
        trusted_provenance()?,
        RunEvidence {
            measured_at_unix_ms: measurement_timestamp()?,
            freshness_nonce: invocation.freshness_nonce,
            producer_derivation_identities: identities,
        },
        &fragments,
    )
    .context("assembling strict performance result")?;
    let invocation_result = run_root.join(RESULT_FILENAME);
    write_json_atomic(&invocation_result, &run)?;
    write_json_atomic(&Path::new(".xtask/performance").join(RESULT_FILENAME), &run)?;
    println!(
        "assembled {} workload(s) at {} ({})",
        run.workloads.len(),
        invocation_result.display(),
        if is_canonical_selection(&run.selection) {
            "canonical selection"
        } else {
            "noncanonical selection"
        }
    );
    Ok(())
}

// Bind caller-supplied retention metadata to this invocation's Nix-selected identities.
fn validate_retained_identity(
    producer: &RetainedProducer,
    selected: &[SelectedProducer],
    run_root: &Path,
    canonical_run_root: &Path,
) -> Result<()> {
    ensure!(
        !producer.name.trim().is_empty()
            && Path::new(&producer.name).components().count() == 1
            && Path::new(&producer.name)
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "producer name is invalid"
    );
    ensure!(
        producer.identity.starts_with("/nix/store/"),
        "producer identity is not a Nix store path: {}",
        producer.identity
    );
    let selected = selected
        .iter()
        .find(|selected| selected.name == producer.name)
        .context("retained producer identity is absent from Nix selection")?;
    ensure!(
        selected.identity == producer.identity
            && selected.producer == producer.producer
            && selected.backend == producer.backend
            && selected.browser == producer.browser,
        "retained producer identity differs from Nix selection"
    );
    let expected_root = run_root
        .join("producers")
        .join(&producer.name)
        .join("performance");
    ensure!(
        producer.retained_root == expected_root,
        "retained producer root does not belong to this invocation: {}",
        producer.name
    );
    ensure!(
        fs::canonicalize(&producer.retained_root)
            .context("resolving retained producer root")?
            .starts_with(canonical_run_root),
        "retained producer root is outside invocation"
    );
    Ok(())
}

fn trusted_provenance() -> Result<Provenance> {
    if let Ok(repository) = env::var("GITHUB_REPOSITORY") {
        return Ok(Provenance::Github(performance::GitHubProvenance {
            repository,
            workflow: required_env("GITHUB_WORKFLOW")?,
            job: required_env("GITHUB_JOB")?,
            reference: required_env("GITHUB_REF")?,
            head_sha: required_env("GITHUB_SHA")?,
            run_id: required_env("GITHUB_RUN_ID")?
                .parse()
                .context("parsing GITHUB_RUN_ID")?,
            attempt: required_env("GITHUB_RUN_ATTEMPT")?
                .parse()
                .context("parsing GITHUB_RUN_ATTEMPT")?,
        }));
    }
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .context("reading local Git revision for performance provenance")?;
    ensure!(output.status.success(), "git rev-parse HEAD failed");
    let git_commit = String::from_utf8(output.stdout).context("Git revision is not UTF-8")?;
    let git_commit = git_commit.trim();
    ensure!(
        !git_commit.is_empty(),
        "local performance run requires a Git commit"
    );
    Ok(Provenance::Local(LocalProvenance {
        git_commit: git_commit.to_owned(),
    }))
}

fn required_env(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("CI performance provenance requires {name}"))
}

// A new nonce changes every producer derivation identity so prior measurements cannot be reused.
fn freshness_nonce() -> Result<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut bytes = [0_u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let mut nonce = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        nonce.push(char::from(HEX[usize::from(byte >> 4)]));
        nonce.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(nonce)
}

fn measurement_timestamp() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock predates the Unix epoch")?
        .as_millis()
        .try_into()
        .context("measurement timestamp exceeds u64")
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("reading {}", path.display()))?)
        .with_context(|| format!("parsing {}", path.display()))
}

// Publish only complete JSON artifacts; a failed write must not truncate an earlier result.
fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let parent = path.parent().context("result path has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .with_context(|| format!("publishing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use performance::CountOverrides;
    use std::fs;

    fn status_file(ok: bool, schema_version: u64) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("temporary producer directory");
        let path = directory.path().join("producer-status-v1.json");
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": schema_version,
                "ok": ok,
                "detail": if ok { "success" } else { "injected producer failure" },
            })
            .to_string(),
        )
        .expect("write producer status");
        (directory, path)
    }

    #[test]
    fn selection_preflight_rejects_invalid_count_relationships() {
        let directory = tempfile::tempdir().expect("temporary selection directory");
        let path = directory.path().join("selection.json");
        fs::write(
            &path,
            serde_json::json!({
                "profile": "small",
                "selection": RunSelection {
                    backends: vec![Backend::Sqlite],
                    count_overrides: CountOverrides {
                        posts: Some(100),
                        authors: Some(12),
                        revisions: Some(777),
                    },
                    browsers: vec![],
                    storage: true,
                    browser: false,
                },
            })
            .to_string(),
        )
        .expect("write selection");

        assert!(validate_selection(&path).is_err());
    }

    #[test]
    fn selection_preflight_accepts_valid_noncanonical_count_relationships() {
        let directory = tempfile::tempdir().expect("temporary selection directory");
        let path = directory.path().join("selection.json");
        fs::write(
            &path,
            serde_json::json!({
                "profile": "small",
                "selection": RunSelection {
                    backends: vec![Backend::Sqlite],
                    count_overrides: CountOverrides {
                        posts: Some(120),
                        authors: Some(12),
                        revisions: Some(777),
                    },
                    browsers: vec![],
                    storage: true,
                    browser: false,
                },
            })
            .to_string(),
        )
        .expect("write selection");

        assert!(validate_selection(&path).is_ok());
    }

    #[test]
    fn failed_producer_status_is_not_accepted() {
        let (directory, _path) = status_file(false, 1);
        let error =
            validate_producer_root(directory.path(), Producer::Storage, Backend::Sqlite, None)
                .expect_err("failed status is rejected before fragment access");
        assert!(format!("{error:#}").contains("injected producer failure"));
    }

    #[test]
    fn unsupported_producer_status_schema_is_rejected() {
        let (directory, _path) = status_file(true, 2);
        let error =
            validate_producer_root(directory.path(), Producer::Storage, Backend::Sqlite, None)
                .expect_err("unsupported status schema is rejected");
        assert!(format!("{error:#}").contains("producer status schema is unsupported"));
    }

    #[test]
    fn successful_status_without_fragment_is_rejected() {
        let (directory, _path) = status_file(true, 1);
        let error =
            validate_producer_root(directory.path(), Producer::Storage, Backend::Sqlite, None)
                .expect_err("missing fragment is rejected");
        assert!(format!("{error:#}").contains("resolving producer fragments"));
    }

    #[test]
    fn diagnostic_references_must_remain_inside_fragment_root() {
        let directory = tempfile::tempdir().expect("temporary producer directory");
        let fragments_root = directory.path().join("fragments");
        fs::create_dir_all(&fragments_root).expect("create fragments directory");
        fs::write(fragments_root.join("trace.json"), "{}").expect("write trace");
        let diagnostics = BrowserDiagnostics {
            navigation_artifacts: vec!["trace.json".into()],
            trace_artifacts: vec!["trace.json".into()],
            otel_trace_artifact: "trace.json".into(),
        };
        assert!(validate_diagnostic_artifacts(&fragments_root, &diagnostics).is_ok());

        let escaped = BrowserDiagnostics {
            navigation_artifacts: vec!["../outside.json".into()],
            trace_artifacts: vec!["trace.json".into()],
            otel_trace_artifact: "trace.json".into(),
        };
        assert!(validate_diagnostic_artifacts(&fragments_root, &escaped).is_err());
    }

    struct FailingSerialization;

    impl serde::Serialize for FailingSerialization {
        fn serialize<S: serde::Serializer>(
            &self,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut map = serializer.serialize_map(Some(2))?;
            map.serialize_entry("written_before_failure", &true)?;
            Err(serde::ser::Error::custom("injected serialization failure"))
        }
    }

    #[test]
    fn atomic_publication_preserves_old_bytes_on_failure_and_cleans_temporary_files() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let result = directory.path().join(RESULT_FILENAME);
        fs::write(&result, "old result")?;
        assert!(write_json_atomic(&result, &FailingSerialization).is_err());
        assert_eq!(fs::read_to_string(&result)?, "old result");
        assert_eq!(fs::read_dir(directory.path())?.count(), 1);
        // A failed rename must not leave a staged partial file for the next invocation.
        let blocked = directory.path().join("blocked");
        fs::create_dir(&blocked)?;
        assert!(write_json_atomic(&blocked, &serde_json::json!({"complete": true})).is_err());
        assert!(blocked.is_dir());
        assert_eq!(fs::read_dir(directory.path())?.count(), 2);
        write_json_atomic(&result, &serde_json::json!({"complete": true}))?;
        assert_eq!(
            read_json::<serde_json::Value>(&result)?,
            serde_json::json!({"complete": true})
        );
        assert_eq!(fs::read_dir(directory.path())?.count(), 2);
        Ok(())
    }
}
