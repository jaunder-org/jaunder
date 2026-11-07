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
        CommandLine::Nonce => println!("{}", freshness_nonce()?),
        CommandLine::ValidateSelection { selection } => validate_selection(&selection)?,
        CommandLine::ValidateProducer {
            performance_root,
            producer,
            backend,
            browser,
        } => {
            validate_producer_root(
                &performance_root,
                producer.into(),
                backend.into(),
                browser.map(Into::into),
            )?;
        }
        CommandLine::Assemble { invocation } => assemble(&invocation)?,
    }
    Ok(())
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

    match producer {
        Producer::Storage => ensure!(
            browser.is_none(),
            "storage producer cannot select a browser"
        ),
        Producer::Browser => ensure!(browser.is_some(), "browser producer requires a browser"),
    }
    let filename = match (producer, browser) {
        (Producer::Storage, None) => storage_fragment_filename(backend),
        (Producer::Browser, Some(browser)) => browser_fragment_filename(backend, browser),
        _ => bail!("producer identity is inconsistent"),
    };
    let fragments_root = performance_root.join("fragments");
    let fragment: FragmentEnvelope = read_json(&fragments_root.join(filename))?;
    validate_fragment(&fragment).context("producer fragment violates the shared contract")?;
    let (actual_producer, setup, diagnostics) = match &fragment.fragment {
        Fragment::Storage(value) => (Producer::Storage, &value.setup, None),
        Fragment::Browser(value) => (Producer::Browser, &value.setup, Some(&value.diagnostics)),
    };
    ensure!(
        actual_producer == producer,
        "producer fragment kind does not match selection"
    );
    ensure!(
        setup.backend == backend,
        "producer fragment backend does not match selection"
    );
    if let Some(diagnostics) = diagnostics {
        ensure!(
            browser.is_some(),
            "browser diagnostic fragment has no browser selection"
        );
        validate_diagnostic_artifacts(&fragments_root, diagnostics)?;
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

    let mut fragments = Vec::with_capacity(invocation.producers.len());
    let mut identities = Vec::with_capacity(invocation.producers.len());
    let mut names = BTreeSet::new();
    for producer in &invocation.producers {
        ensure!(
            !producer.name.trim().is_empty()
                && Path::new(&producer.name)
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
            "producer name is invalid"
        );
        ensure!(
            names.insert(&producer.name),
            "duplicate producer name {}",
            producer.name
        );
        ensure!(
            producer.identity.starts_with("/nix/store/"),
            "producer identity is not a Nix store path: {}",
            producer.identity
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
    let temporary = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&temporary)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(temporary, path)?;
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
        assert!(format!("{error:#}").contains("storage-sqlite-v1.json"));
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
}
