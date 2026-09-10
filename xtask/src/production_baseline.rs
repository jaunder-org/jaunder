//! Host-only identity, lease, and evidence boundaries for the opt-in production baseline.
//!
//! This module intentionally stops before package construction or VM lifecycle work.  Its
//! preflight is nevertheless load-bearing: later lifecycle code may only run after these
//! checks and while its [`RunLease`] is held.

use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    cli::ProductionBaselineCommand,
    git,
    result::{CommandResult, StepResult},
};

const UPSTREAM_URLS: [&str; 4] = [
    "https://github.com/jaunder-org/jaunder.git",
    "https://github.com/jaunder-org/jaunder",
    "git@github.com:jaunder-org/jaunder.git",
    "ssh://git@github.com/jaunder-org/jaunder.git",
];
const HARNESS_PATHS: [&str; 14] = [
    "docs/production-baseline.schema.json",
    "xtask/Cargo.toml",
    "xtask/build.rs",
    "xtask/src/main.rs",
    "xtask/src/lib.rs",
    "xtask/src/cli.rs",
    "xtask/src/dispatch.rs",
    "xtask/src/production_baseline.rs",
    "end2end/tests/production-baseline-flow.spec.ts",
    "end2end/tests/production-baseline.ts",
    "test-support/Cargo.toml",
    "test-support/build.rs",
    "test-support/src/lib.rs",
    "test-support/src/main.rs",
];

/// An immutable product source accepted by the baseline harness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRevision {
    pub commit: String,
    pub flake_ref: String,
}

/// The harness source identity that makes an evidence record reproducible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessIdentity {
    pub commit: String,
    pub seeded_manifest: ManifestIdentity,
    pub operation_manifest: ManifestIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestIdentity {
    pub version: u32,
    pub sha256: String,
}

/// Product or runtime package identity recorded by durable evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageIdentity {
    pub installable: String,
    pub derivation: String,
    pub output_path: String,
    pub nar_hash: String,
    pub executable_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeIdentity {
    pub deployment_id: String,
    pub backend: StorageBackend,
    pub revision: ResolvedRevision,
    pub executable: String,
    pub exec_start: String,
    pub package: PackageIdentity,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupIdentity {
    pub source_backend: StorageBackend,
    pub target_backend: StorageBackend,
    pub format_version: u32,
    pub source_schema_version: u32,
    pub target_schema_version: u32,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedOutcome {
    pub id: String,
    pub outcome: TimedOutcomeStatus,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimedOutcomeStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailureClass {
    Product,
    Harness,
    Infrastructure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub id: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub disposition: FindingDisposition,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingDisposition {
    Blocking,
    Accepted,
    Deferred,
}

/// Schema-v1 evidence. JSON is authoritative; Markdown is generated solely by [`render_markdown`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub schema_version: u32,
    pub operation: String,
    pub outcome: Outcome,
    pub harness: HarnessIdentity,
    pub source: ResolvedRevision,
    pub target: Option<ResolvedRevision>,
    pub runtime: Vec<RuntimeIdentity>,
    pub backups: Vec<BackupIdentity>,
    pub lifecycle: Vec<TimedOutcome>,
    pub checks: Vec<TimedOutcome>,
    pub gaps: Vec<Gap>,
    pub findings: Vec<Finding>,
    pub failure_classes: Vec<FailureClass>,
}

pub const CHECK_IDS: [&str; 12] = [
    "browser-create",
    "browser-read-only",
    "atompub",
    "feeds",
    "service-restart",
    "vm-reboot",
    "upgrade",
    "backup",
    "restore-sqlite-sqlite",
    "restore-sqlite-postgres",
    "restore-postgres-sqlite",
    "restore-postgres-postgres",
];

impl Evidence {
    /// Enforces the semantic subset of schema v1 before any bytes may be published.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "unsupported production-baseline evidence schema {}",
                self.schema_version
            );
        }
        if !matches!(self.operation.as_str(), "discover" | "accept") {
            bail!("unknown baseline operation {}", self.operation);
        }
        if self.operation == "accept" && self.target.is_none() {
            bail!("accept evidence requires a target revision");
        }
        if self.operation == "discover" && self.target.is_some() {
            bail!("discover evidence must not contain a target revision");
        }
        if matches!(self.outcome, Outcome::Failed) && self.failure_classes.is_empty() {
            bail!("failed evidence must classify its failure");
        }
        validate_commit(&self.harness.commit, "harness commit")?;
        validate_revision(&self.source)?;
        if let Some(target) = &self.target {
            validate_revision(target)?;
            if target.commit == self.source.commit {
                bail!("evidence source and target commits must differ");
            }
        }
        for identity in [
            &self.harness.seeded_manifest,
            &self.harness.operation_manifest,
        ] {
            if identity.version == 0 {
                bail!("manifest version must be positive");
            }
            validate_digest(&identity.sha256, "manifest SHA-256")?;
        }
        for runtime in &self.runtime {
            if runtime.executable.is_empty() || runtime.exec_start.is_empty() {
                bail!("runtime identity must record executable and ExecStart");
            }
            if runtime.deployment_id.is_empty() {
                bail!("runtime identity requires a deployment identity");
            }
            validate_revision(&runtime.revision)?;
            let package = &runtime.package;
            if [
                package.installable.as_str(),
                package.derivation.as_str(),
                package.output_path.as_str(),
                package.nar_hash.as_str(),
            ]
            .iter()
            .any(|v| v.is_empty())
            {
                bail!("package identity is incomplete");
            }
            validate_digest(&package.executable_sha256, "package executable SHA-256")?;
        }
        for backup in &self.backups {
            if backup.format_version == 0
                || backup.source_schema_version == 0
                || backup.target_schema_version == 0
            {
                bail!("backup versions must be positive");
            }
            validate_digest(&backup.sha256, "backup SHA-256")?;
            if matches!(self.outcome, Outcome::Passed)
                && backup.source_schema_version != backup.target_schema_version
            {
                bail!("passing evidence requires backup source and target schema equality");
            }
        }
        let mut deployment_ids = BTreeSet::new();
        if self
            .runtime
            .iter()
            .any(|runtime| !deployment_ids.insert(runtime.deployment_id.as_str()))
        {
            bail!("runtime deployment identities must be unique");
        }
        validate_timed("lifecycle", &self.lifecycle, None)?;
        validate_timed("checks", &self.checks, Some(&CHECK_IDS))?;
        if matches!(self.outcome, Outcome::Passed) {
            if self.runtime.is_empty() || self.backups.is_empty() {
                bail!("passing evidence requires runtime and backup provenance");
            }
            if self.checks.len() != CHECK_IDS.len() {
                bail!("passing evidence requires every fixed check exactly once");
            }
            for check in &self.checks {
                if self.operation == "discover" && check.id == "upgrade" {
                    if !matches!(check.outcome, TimedOutcomeStatus::Skipped) {
                        bail!("passing discovery evidence must mark upgrade skipped");
                    }
                } else if !matches!(check.outcome, TimedOutcomeStatus::Passed) {
                    bail!(
                        "passing evidence has a non-passing required check {}",
                        check.id
                    );
                }
            }
            let runtime_pairs = self
                .runtime
                .iter()
                .map(|runtime| (runtime.revision.commit.as_str(), runtime.backend))
                .collect::<BTreeSet<_>>();
            let mut expected_runtime_pairs = BTreeSet::from([
                (self.source.commit.as_str(), StorageBackend::Sqlite),
                (self.source.commit.as_str(), StorageBackend::Postgres),
            ]);
            if let Some(target) = &self.target {
                expected_runtime_pairs.extend([
                    (target.commit.as_str(), StorageBackend::Sqlite),
                    (target.commit.as_str(), StorageBackend::Postgres),
                ]);
            }
            if runtime_pairs != expected_runtime_pairs {
                bail!(
                    "passing evidence runtime deployments do not cover each operation revision and backend"
                );
            }
            let directions = self
                .backups
                .iter()
                .map(|backup| (backup.source_backend, backup.target_backend))
                .collect::<BTreeSet<_>>();
            let expected_directions = BTreeSet::from([
                (StorageBackend::Sqlite, StorageBackend::Sqlite),
                (StorageBackend::Sqlite, StorageBackend::Postgres),
                (StorageBackend::Postgres, StorageBackend::Sqlite),
                (StorageBackend::Postgres, StorageBackend::Postgres),
            ]);
            if self.backups.len() != expected_directions.len() || directions != expected_directions
            {
                bail!("passing evidence requires every backend restore direction exactly once");
            }
        }
        for gap in &self.gaps {
            if gap.id.is_empty() || gap.rationale.is_empty() {
                bail!("gaps need an id and rationale");
            }
        }
        for finding in &self.findings {
            if finding.id.is_empty() || finding.rationale.is_empty() {
                bail!("findings need an id and rationale");
            }
        }
        Ok(())
    }
}

fn validate_timed(label: &str, entries: &[TimedOutcome], allowed: Option<&[&str]>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for entry in entries {
        if entry.id.is_empty() || !seen.insert(&entry.id) {
            bail!("{label} has an empty or duplicate id");
        }
        if let Some(allowed) = allowed
            && !allowed.contains(&entry.id.as_str())
        {
            bail!("unknown fixed check id {}", entry.id);
        }
    }
    Ok(())
}

fn validate_revision(revision: &ResolvedRevision) -> Result<()> {
    validate_commit(&revision.commit, "revision commit")?;
    if revision.flake_ref != format!("github:jaunder-org/jaunder/{}", revision.commit) {
        bail!("revision flake reference is not immutable");
    }
    Ok(())
}

fn validate_commit(value: &str, label: &str) -> Result<()> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} must be a full 40-character commit");
    }
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} must be a SHA-256 digest");
    }
    Ok(())
}

/// The actual-origin boundary is injected so fixture repositories remain networkless.
trait Origin {
    fn verify_commits(&self, root: &Path, commits: &[&str]) -> Result<()>;
}

struct GitOrigin;

impl Origin for GitOrigin {
    fn verify_commits(&self, root: &Path, commits: &[&str]) -> Result<()> {
        let remote = git::at(root)
            .args(["remote", "get-url", "origin"])
            .output()
            .context("reading production-baseline origin")?;
        if !remote.status.success()
            || !UPSTREAM_URLS.contains(&String::from_utf8_lossy(&remote.stdout).trim())
        {
            bail!("production-baseline requires origin to be jaunder-org/jaunder");
        }
        let status = git::at(root)
            .args(["fetch", "--quiet", "--no-write-fetch-head", "origin"])
            .args(commits)
            .status()
            .context("proving production-baseline commits at origin")?;
        if !status.success() {
            bail!("one or more production-baseline commits are absent from origin");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
struct BinaryBuildIdentity {
    commit: String,
    clean: bool,
}

/// Binary provenance is injected so stale, dirty, or mixed executable rejection is testable
/// without compiling or spawning fixture binaries.
trait BinaryProvenance {
    fn xtask(&self) -> Result<BinaryBuildIdentity>;
    fn test_support(&self, root: &Path) -> Result<BinaryBuildIdentity>;
}

struct ExecutingBinaries;

impl BinaryProvenance for ExecutingBinaries {
    fn xtask(&self) -> Result<BinaryBuildIdentity> {
        Ok(BinaryBuildIdentity {
            commit: env!("JAUNDER_BUILD_COMMIT").to_owned(),
            clean: env!("JAUNDER_BUILD_DIRTY") == "0",
        })
    }

    fn test_support(&self, root: &Path) -> Result<BinaryBuildIdentity> {
        let binary = std::env::var_os("JAUNDER_TEST_SUPPORT_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("target/debug/test-support"));
        let output = Command::new(&binary)
            .arg("build-commit")
            .output()
            .with_context(|| {
                format!(
                    "reading test-support build provenance from {}",
                    binary.display()
                )
            })?;
        if !output.status.success() {
            bail!(
                "test-support build-commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        serde_json::from_slice(&output.stdout).context("parsing test-support build provenance")
    }
}

/// An injected Git boundary. Tests use fixture repositories without network access.
trait Repository {
    fn output(&self, args: &[&str]) -> Result<String>;
    fn status(&self, args: &[&str]) -> Result<bool>;
    fn root(&self) -> &Path;
}

struct GitRepository {
    root: PathBuf,
}
impl GitRepository {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }
}
impl Repository for GitRepository {
    fn output(&self, args: &[&str]) -> Result<String> {
        let output = git::at(&self.root)
            .args(args)
            .output()
            .context("running git for production baseline")?;
        if !output.status.success() {
            bail!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
    fn status(&self, args: &[&str]) -> Result<bool> {
        Ok(git::at(&self.root)
            .args(args)
            .status()
            .context("running git for production baseline")?
            .success())
    }
    fn root(&self) -> &Path {
        &self.root
    }
}

fn resolve_revision(repo: &impl Repository, input: &str) -> Result<ResolvedRevision> {
    let commit = repo.output(&["rev-parse", "--verify", &format!("{input}^{{commit}}")])?;
    validate_commit(&commit, "resolved revision")?;
    Ok(ResolvedRevision {
        flake_ref: format!("github:jaunder-org/jaunder/{commit}"),
        commit,
    })
}

fn resolve_pair(
    repo: &impl Repository,
    source: &str,
    target: &str,
) -> Result<(ResolvedRevision, ResolvedRevision)> {
    let source = resolve_revision(repo, source)?;
    let target = resolve_revision(repo, target)?;
    if source.commit == target.commit {
        bail!("production-baseline accept requires distinct resolved source and target commits");
    }
    Ok((source, target))
}

fn validate_harness(
    repo: &impl Repository,
    binaries: &impl BinaryProvenance,
) -> Result<HarnessIdentity> {
    let dirty = repo.output(&["status", "--porcelain"])?;
    if !dirty.is_empty() {
        bail!("production-baseline requires a clean harness checkout");
    }
    let commit = repo.output(&["rev-parse", "HEAD"])?;
    validate_commit(&commit, "harness commit")?;
    let xtask = binaries.xtask()?;
    let test_support = binaries.test_support(repo.root())?;
    validate_commit(&xtask.commit, "executing xtask build commit")?;
    validate_commit(&test_support.commit, "test-support build commit")?;
    if !xtask.clean || !test_support.clean {
        bail!("production-baseline binaries were built from a dirty harness tree");
    }
    if xtask.commit != commit || test_support.commit != commit {
        bail!(
            "executing harness binaries are stale or mixed: harness={commit} xtask={} test-support={}",
            xtask.commit,
            test_support.commit
        );
    }
    for path in HARNESS_PATHS {
        if !repo.root().join(path).is_file() {
            bail!("required harness input {path} is absent");
        }
        if !repo.status(&["cat-file", "-e", &format!("HEAD:{path}")])? {
            bail!("required harness input {path} is not present in harness commit {commit}");
        }
    }
    Ok(HarnessIdentity {
        commit,
        seeded_manifest: manifest_identity(
            repo.root().join("test-support/src/lib.rs"),
            "seeded",
            "pub fn sandbox_profile_manifest",
        )?,
        operation_manifest: manifest_identity(
            repo.root().join("end2end/tests/production-baseline.ts"),
            "operation",
            "export const OPERATION_MANIFEST",
        )?,
    })
}

fn manifest_identity(path: PathBuf, kind: &str, authority: &str) -> Result<ManifestIdentity> {
    let bytes = fs::read(&path)
        .with_context(|| format!("reading {kind} manifest definition {}", path.display()))?;
    let text = std::str::from_utf8(&bytes)
        .with_context(|| format!("{kind} manifest definition is not UTF-8"))?;
    let authority = text
        .find(authority)
        .ok_or_else(|| anyhow::anyhow!("{kind} manifest authority is absent"))?;
    let version = text[authority..]
        .lines()
        .find_map(|line| line.trim().strip_prefix("version:"))
        .ok_or_else(|| anyhow::anyhow!("{kind} manifest authority has no version"))?
        .trim()
        .trim_end_matches(',')
        .parse()
        .with_context(|| format!("{kind} manifest authority has an invalid version"))?;
    if version == 0 {
        bail!("{kind} manifest authority version must be positive");
    }
    Ok(ManifestIdentity {
        version,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LeaseRecord {
    pid: u32,
    started_ticks: u64,
}

/// Exclusive host ownership is the kernel lock, not the diagnostic record. The
/// inode is intentionally retained after release so no process can unlink a live lease.
pub struct RunLease {
    _file: std::fs::File,
}
impl RunLease {
    fn acquire(_root: &Path) -> Result<Self> {
        Self::acquire_at(&global_lease_path()?)
    }

    fn acquire_at(path: &Path) -> Result<Self> {
        fs::create_dir_all(path.parent().context("host lease has no parent")?)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("opening host-global lease {}", path.display()))?;
        if let Err(error) = file.try_lock() {
            let owner = read_lease(path).ok();
            let detail = owner.map_or_else(
                || "unknown owner".to_owned(),
                |owner| format!("pid {}", owner.pid),
            );
            return Err(error).context(format!("production-baseline is already owned by {detail}"));
        }
        let record = LeaseRecord {
            pid: std::process::id(),
            started_ticks: process_start_ticks(std::process::id())?,
        };
        file.set_len(0)?;
        file.write_all(&serde_json::to_vec(&record)?)?;
        file.sync_all()?;
        Ok(Self { _file: file })
    }
}

impl Drop for RunLease {
    fn drop(&mut self) {
        let _ = self._file.unlock();
    }
}

fn global_lease_path() -> Result<PathBuf> {
    Ok(PathBuf::from("/tmp/jaunder-production-baseline.lock"))
}

fn read_lease(path: &Path) -> Result<LeaseRecord> {
    serde_json::from_slice(
        &fs::read(path).with_context(|| format!("reading lease {}", path.display()))?,
    )
    .context("parsing production-baseline lease")
}
fn process_start_ticks(pid: u32) -> Result<u64> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat"))
        .with_context(|| format!("reading owner process {pid}"))?;
    let close = text.rfind(')').context("malformed /proc stat")?;
    text[close + 2..]
        .split_whitespace()
        .nth(19)
        .context("missing /proc process start time")?
        .parse()
        .context("parsing /proc process start time")
}

/// Render every durable field deterministically from JSON-authoritative evidence.
pub fn render_markdown(evidence: &Evidence) -> Result<String> {
    evidence.validate()?;
    let mut out = format!("# Production baseline {}\n\n", evidence.operation);
    out.push_str(&format!(
        "- Outcome: {:?}\n- Harness commit: `{}`\n- Source: `{}`\n- Manifests: seeded v{} `{}`, operation v{} `{}`\n",
        evidence.outcome, evidence.harness.commit, evidence.source.commit,
        evidence.harness.seeded_manifest.version, evidence.harness.seeded_manifest.sha256,
        evidence.harness.operation_manifest.version, evidence.harness.operation_manifest.sha256,
    ));
    if let Some(target) = &evidence.target {
        out.push_str(&format!("- Target: `{}`\n", target.commit));
    }
    out.push_str("\n## Runtime identities\n\n");
    for runtime in &evidence.runtime {
        out.push_str(&format!("- deployment `{}`; backend {:?}; revision `{}`; executable `{}`; ExecStart `{}`; installable `{}`; derivation `{}`; output `{}`; NAR `{}`; SHA-256 `{}`\n", runtime.deployment_id, runtime.backend, runtime.revision.commit, runtime.executable, runtime.exec_start, runtime.package.installable, runtime.package.derivation, runtime.package.output_path, runtime.package.nar_hash, runtime.package.executable_sha256));
    }
    out.push_str("\n## Backups\n\n");
    for backup in &evidence.backups {
        out.push_str(&format!(
            "- {:?} → {:?}; format {}; source schema {}; target schema {}; SHA-256 `{}`\n",
            backup.source_backend,
            backup.target_backend,
            backup.format_version,
            backup.source_schema_version,
            backup.target_schema_version,
            backup.sha256
        ));
    }
    out.push_str("\n## Failure classes\n\n");
    for class in &evidence.failure_classes {
        out.push_str(&format!("- {:?}\n", class));
    }
    out.push_str("\n## Lifecycle\n\n| ID | Outcome | Duration (ms) |\n| --- | --- | ---: |\n");
    for entry in &evidence.lifecycle {
        out.push_str(&format!(
            "| {} | {:?} | {} |\n",
            entry.id, entry.outcome, entry.duration_ms
        ));
    }
    out.push_str("\n## Checks\n\n| ID | Outcome | Duration (ms) |\n| --- | --- | ---: |\n");
    for entry in &evidence.checks {
        out.push_str(&format!(
            "| {} | {:?} | {} |\n",
            entry.id, entry.outcome, entry.duration_ms
        ));
    }
    out.push_str("\n## Gaps\n\n");
    for gap in &evidence.gaps {
        out.push_str(&format!("- {}: {}\n", gap.id, gap.rationale));
    }
    out.push_str("\n## Findings\n\n");
    for finding in &evidence.findings {
        out.push_str(&format!(
            "- {} ({:?}): {}\n",
            finding.id, finding.disposition, finding.rationale
        ));
    }
    Ok(out)
}

/// Run-owned sensitive values which must never enter either durable report.
#[derive(Debug, Clone)]
pub struct EvidenceCanaries {
    credentials: Vec<String>,
    cookies: Vec<String>,
    private_keys: Vec<String>,
}

impl EvidenceCanaries {
    pub fn new(
        credentials: Vec<String>,
        cookies: Vec<String>,
        private_keys: Vec<String>,
    ) -> Result<Self> {
        let registry = Self {
            credentials,
            cookies,
            private_keys,
        };
        registry.validate()?;
        Ok(registry)
    }

    fn validate(&self) -> Result<()> {
        for (category, values) in [
            ("credentials", &self.credentials),
            ("cookies", &self.cookies),
            ("private keys", &self.private_keys),
        ] {
            if values.is_empty() || values.iter().any(|value| value.is_empty()) {
                bail!("evidence canary registry requires nonempty {category}");
            }
        }
        Ok(())
    }

    fn values(&self) -> impl Iterator<Item = &str> {
        self.credentials
            .iter()
            .chain(&self.cookies)
            .chain(&self.private_keys)
            .map(String::as_str)
    }
}

/// Publish exactly the two sanitized files by atomically moving a validated directory out of
/// the restricted workspace. Any failure before the final rename publishes nothing.
pub fn publish(
    workspace: &Path,
    destination: &Path,
    evidence: &Evidence,
    canaries: &EvidenceCanaries,
) -> Result<()> {
    canaries.validate()?;
    evidence.validate()?;
    if destination.exists() {
        bail!(
            "evidence destination already exists: {}",
            destination.display()
        );
    }
    let json = serde_json::to_string_pretty(evidence).context("serializing evidence")? + "\n";
    validate_serialized_evidence(&json)?;
    let markdown = render_markdown(evidence)?;
    scan_retained("summary.json", &json, canaries)?;
    scan_retained("summary.md", &markdown, canaries)?;
    let staging = workspace.join("publish-ready");
    if staging.exists() {
        bail!("restricted evidence staging path already exists");
    }
    fs::create_dir_all(&staging).context("creating restricted evidence staging")?;
    fs::write(staging.join("summary.json"), json).context("writing restricted summary JSON")?;
    fs::write(staging.join("summary.md"), markdown)
        .context("writing restricted summary Markdown")?;
    validate_allowlist(&staging)?;
    fs::rename(&staging, destination).context("atomically publishing sanitized evidence")
}

fn validate_serialized_evidence(json: &str) -> Result<()> {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../docs/production-baseline.schema.json"))
            .context("parsing committed production-baseline schema")?;
    let evidence: serde_json::Value =
        serde_json::from_str(json).context("parsing serialized evidence")?;
    jsonschema::draft202012::validate(&schema, &evidence).map_err(|error| {
        anyhow::anyhow!("serialized evidence violates committed schema: {error}")
    })?;
    let evidence: Evidence =
        serde_json::from_value(evidence).context("deserializing serialized evidence")?;
    evidence.validate()
}

fn scan_retained(name: &str, text: &str, canaries: &EvidenceCanaries) -> Result<()> {
    for forbidden in [
        "password",
        "cookie",
        "credential",
        "private_key",
        "authorization",
    ] {
        if text.to_ascii_lowercase().contains(forbidden) {
            bail!("{name} contains forbidden sensitive field or value");
        }
    }
    for canary in canaries.values() {
        if text.contains(canary) {
            bail!("{name} contains a registered canary");
        }
    }
    Ok(())
}
fn validate_allowlist(dir: &Path) -> Result<()> {
    let names: BTreeSet<_> = fs::read_dir(dir)?
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<std::result::Result<_, _>>()?;
    let expected = BTreeSet::from(["summary.json".to_owned(), "summary.md".to_owned()]);
    if names != expected {
        bail!("durable evidence violates the two-file allowlist");
    }
    Ok(())
}

pub fn run(command: ProductionBaselineCommand) -> Result<CommandResult> {
    let root = PathBuf::from(git::toplevel(Path::new("."))?);
    let repo = GitRepository::new(root.clone());
    let origin = GitOrigin;
    let binaries = ExecutingBinaries;
    let start = Instant::now();
    let (name, preflight) = match command {
        ProductionBaselineCommand::Discover { revision } => (
            "production-baseline-discover",
            resolve_revision(&repo, &revision).map(|source| (source, None)),
        ),
        ProductionBaselineCommand::Accept { source, target } => (
            "production-baseline-accept",
            resolve_pair(&repo, &source, &target).map(|(source, target)| (source, Some(target))),
        ),
    };
    let (source, target) = preflight?;
    let harness = validate_harness(&repo, &binaries)?;
    let mut commits = vec![harness.commit.as_str(), source.commit.as_str()];
    if let Some(target) = &target {
        commits.push(target.commit.as_str());
    }
    origin.verify_commits(&root, &commits)?;
    let mut result = CommandResult::new(name);
    let _lease = RunLease::acquire(&root)?;
    result.push(
        StepResult::ok("production-baseline-preflight")
            .detail(format!(
                "harness={} source={} target={}",
                harness.commit,
                source.commit,
                target.as_ref().map_or("-", |r| r.commit.as_str())
            ))
            .with_duration(start.elapsed()),
    );
    result.push(
        StepResult::fail("production-baseline-lifecycle")
            .detail("package and VM lifecycle are not implemented by this interface task")
            .with_duration(start.elapsed()),
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    struct FixtureOrigin {
        upstream: BTreeSet<String>,
    }

    impl Origin for FixtureOrigin {
        fn verify_commits(&self, _: &Path, commits: &[&str]) -> Result<()> {
            if commits.iter().all(|commit| self.upstream.contains(*commit)) {
                Ok(())
            } else {
                bail!("fixture origin does not contain every commit")
            }
        }
    }

    struct FixtureBinaries {
        xtask: BinaryBuildIdentity,
        test_support: BinaryBuildIdentity,
    }

    impl BinaryProvenance for FixtureBinaries {
        fn xtask(&self) -> Result<BinaryBuildIdentity> {
            Ok(self.xtask.clone())
        }

        fn test_support(&self, _: &Path) -> Result<BinaryBuildIdentity> {
            Ok(self.test_support.clone())
        }
    }

    fn binaries(root: &Path) -> FixtureBinaries {
        let commit = crate::git::at(root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout
            .into_iter()
            .map(char::from)
            .collect::<String>()
            .trim()
            .to_owned();
        FixtureBinaries {
            xtask: BinaryBuildIdentity {
                commit: commit.clone(),
                clean: true,
            },
            test_support: BinaryBuildIdentity {
                commit,
                clean: true,
            },
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let output = crate::git::at(root).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn repo() -> tempfile::TempDir {
        let temp = tempdir().unwrap();
        let root = temp.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.email", "test@example.test"]);
        git(root, &["config", "user.name", "Test"]);
        git(root, &["remote", "add", "origin", UPSTREAM_URLS[0]]);
        for relative in HARNESS_PATHS {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let contents = match relative {
                "test-support/src/lib.rs" => {
                    "pub fn sandbox_profile_manifest() {\nSandboxSeedManifest {\nversion: 1,\n}\n}\n"
                }
                "end2end/tests/production-baseline.ts" => {
                    "export const OPERATION_MANIFEST = {\nversion: 1,\n};\n"
                }
                _ => "fixture\n",
            };
            fs::write(path, contents).unwrap();
        }
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "fixture"]);
        let head = crate::git::at(root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let head = String::from_utf8_lossy(&head.stdout).trim().to_owned();
        git(root, &["update-ref", "refs/remotes/origin/main", &head]);
        temp
    }
    #[test]
    fn upstream_verification_rejects_a_local_only_resolved_commit() {
        let repo = repo();
        let repository = GitRepository::new(repo.path().to_path_buf());
        let upstream = resolve_revision(&repository, "HEAD").unwrap();
        let origin = FixtureOrigin {
            upstream: BTreeSet::from([upstream.commit.clone()]),
        };
        origin
            .verify_commits(repo.path(), &[upstream.commit.as_str()])
            .unwrap();
        git(repo.path(), &["commit", "--allow-empty", "-m", "local"]);
        let local = resolve_revision(&repository, "HEAD").unwrap();
        assert!(
            origin
                .verify_commits(
                    repo.path(),
                    &[upstream.commit.as_str(), local.commit.as_str()]
                )
                .is_err()
        );
    }
    #[test]
    fn rejects_equal_resolved_pair() {
        let repo = repo();
        let r = GitRepository::new(repo.path().to_path_buf());
        assert!(resolve_pair(&r, "HEAD", "HEAD").is_err());
    }
    #[test]
    fn rejects_dirty_harness() {
        let repo = repo();
        let r = GitRepository::new(repo.path().to_path_buf());
        let binaries = binaries(repo.path());
        assert!(validate_harness(&r, &binaries).is_ok());
        fs::write(repo.path().join("xtask/src/main.rs"), "dirty\n").unwrap();
        assert!(validate_harness(&r, &binaries).is_err());
    }

    #[test]
    fn rejects_stale_dirty_or_mixed_binary_provenance() {
        let repo = repo();
        let repository = GitRepository::new(repo.path().to_path_buf());
        let mut stale = binaries(repo.path());
        stale.xtask.commit = "0".repeat(40);
        assert!(validate_harness(&repository, &stale).is_err());

        let mut mixed = binaries(repo.path());
        mixed.test_support.commit.push('0');
        assert!(validate_harness(&repository, &mixed).is_err());

        let mut dirty_at_build = binaries(repo.path());
        dirty_at_build.xtask.clean = false;
        assert!(validate_harness(&repository, &dirty_at_build).is_err());
    }
    #[test]
    fn host_global_lease_excludes_live_owner_without_unlinking() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("production-baseline.lock");
        let first = RunLease::acquire_at(&path).unwrap();
        assert!(RunLease::acquire_at(&path).is_err());
        drop(first);
        assert!(RunLease::acquire_at(&path).is_ok());
    }
    fn evidence() -> Evidence {
        Evidence {
            schema_version: 1,
            operation: "discover".into(),
            outcome: Outcome::Failed,
            harness: HarnessIdentity {
                commit: "a".repeat(40),
                seeded_manifest: ManifestIdentity {
                    version: 1,
                    sha256: "b".repeat(64),
                },
                operation_manifest: ManifestIdentity {
                    version: 1,
                    sha256: "c".repeat(64),
                },
            },
            source: ResolvedRevision {
                commit: "d".repeat(40),
                flake_ref: format!("github:jaunder-org/jaunder/{}", "d".repeat(40)),
            },
            target: None,
            runtime: vec![],
            backups: vec![],
            lifecycle: vec![TimedOutcome {
                id: "deploy".into(),
                outcome: TimedOutcomeStatus::Passed,
                duration_ms: 7,
            }],
            checks: vec![TimedOutcome {
                id: "feeds".into(),
                outcome: TimedOutcomeStatus::Passed,
                duration_ms: 11,
            }],
            gaps: vec![],
            findings: vec![],
            failure_classes: vec![FailureClass::Harness],
        }
    }
    #[test]
    fn evidence_validates_and_markdown_keeps_durations() {
        let evidence = evidence();
        evidence.validate().unwrap();
        let markdown = render_markdown(&evidence).unwrap();
        assert!(markdown.contains("| deploy | Passed | 7 |"));
        assert!(markdown.contains("| feeds | Passed | 11 |"));
    }
    #[test]
    fn committed_schema_validates_serialized_evidence_and_names_fixed_checks() {
        let json = serde_json::to_string_pretty(&evidence()).unwrap();
        validate_serialized_evidence(&json).unwrap();
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../docs/production-baseline.schema.json"))
                .unwrap();
        assert_eq!(schema["properties"]["schema_version"]["const"], 1);
        let check_ids = schema["$defs"]["check"]["allOf"][1]["properties"]["id"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(check_ids.len(), CHECK_IDS.len());
    }

    #[test]
    fn publication_requires_complete_canaries_and_is_allowlisted_and_sanitized() {
        let safe_canaries = EvidenceCanaries::new(
            vec!["unpublished-credential".into()],
            vec!["unpublished-session".into()],
            vec!["unpublished-private-material".into()],
        )
        .unwrap();
        assert!(
            EvidenceCanaries::new(Vec::new(), vec!["session".into()], vec!["key".into()]).is_err()
        );

        let temp = tempdir().unwrap();
        let destination = temp.path().join("durable");
        publish(temp.path(), &destination, &evidence(), &safe_canaries).unwrap();
        validate_allowlist(&destination).unwrap();

        let leaked_operation = EvidenceCanaries::new(
            vec!["discover".into()],
            vec!["unpublished-session".into()],
            vec!["unpublished-private-material".into()],
        )
        .unwrap();
        assert!(
            publish(
                temp.path(),
                &temp.path().join("bad"),
                &evidence(),
                &leaked_operation,
            )
            .is_err()
        );

        let mut failed = evidence();
        failed.outcome = Outcome::Failed;
        failed.failure_classes = vec![FailureClass::Harness];
        failed.gaps.push(Gap {
            id: "secret".into(),
            rationale: "run-canary".into(),
        });
        let leaked_failure = EvidenceCanaries::new(
            vec!["run-canary".into()],
            vec!["unpublished-session".into()],
            vec!["unpublished-private-material".into()],
        )
        .unwrap();
        assert!(
            publish(
                temp.path(),
                &temp.path().join("failed"),
                &failed,
                &leaked_failure,
            )
            .is_err()
        );
        assert!(!temp.path().join("failed").exists());

        let mut redacted = evidence();
        redacted.gaps.push(Gap {
            id: "redaction".into(),
            rationale: "password must never leave the workspace".into(),
        });
        assert!(
            publish(
                temp.path(),
                &temp.path().join("redacted"),
                &redacted,
                &safe_canaries,
            )
            .is_err()
        );
        assert!(!temp.path().join("redacted").exists());
    }
}
