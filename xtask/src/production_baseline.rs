//! Host-only identity, lease, evidence, immutable package, and VM lifecycle
//! boundaries for the opt-in production baseline.
//!
//! Preflight is load-bearing: lifecycle mutations occur only after these checks
//! and while the [`RunLease`] is held. Workflow sequencing remains a separate
//! layer over the lifecycle adapter.

use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
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
const HARNESS_PATHS: [&str; 16] = [
    "docs/production-baseline.schema.json",
    "xtask/Cargo.toml",
    "xtask/build.rs",
    "xtask/src/main.rs",
    "xtask/src/lib.rs",
    "xtask/src/cli.rs",
    "xtask/src/dispatch.rs",
    "xtask/src/production_baseline.rs",
    "xtask/src/production_baseline_lifecycle.rs",
    "end2end/tests/production-baseline-flow.spec.ts",
    "end2end/tests/production-baseline-harness.spec.ts",
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

/// Observed activation of a persisted source deployment under its target package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeIdentity {
    pub backend: StorageBackend,
    pub source_package: PackageIdentity,
    pub target_package: PackageIdentity,
    pub source_schema_version: u32,
    pub target_schema_version: u32,
    pub binary_changed: bool,
    pub schema_migrated: bool,
}

impl UpgradeIdentity {
    fn observe(
        backend: StorageBackend,
        source_package: PackageIdentity,
        target_package: PackageIdentity,
        source_schema_version: u32,
        target_schema_version: u32,
    ) -> Result<Self> {
        if source_schema_version == 0 || target_schema_version == 0 {
            bail!("upgrade schema observations must be positive");
        }
        Ok(Self {
            backend,
            binary_changed: source_package.executable_sha256 != target_package.executable_sha256,
            schema_migrated: source_schema_version != target_schema_version,
            source_package,
            target_package,
            source_schema_version,
            target_schema_version,
        })
    }
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
    pub upgrades: Vec<UpgradeIdentity>,
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
        for upgrade in &self.upgrades {
            if upgrade.source_schema_version == 0 || upgrade.target_schema_version == 0 {
                bail!("upgrade schema observations must be positive");
            }
            for package in [&upgrade.source_package, &upgrade.target_package] {
                if [
                    package.installable.as_str(),
                    package.derivation.as_str(),
                    package.output_path.as_str(),
                    package.nar_hash.as_str(),
                ]
                .iter()
                .any(|value| value.is_empty())
                {
                    bail!("upgrade package identity is incomplete");
                }
                validate_digest(&package.executable_sha256, "upgrade executable SHA-256")?;
            }
            if upgrade.binary_changed
                != (upgrade.source_package.executable_sha256
                    != upgrade.target_package.executable_sha256)
                || upgrade.schema_migrated
                    != (upgrade.source_schema_version != upgrade.target_schema_version)
            {
                bail!("upgrade classification disagrees with observed identities");
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
            let upgrade_backends = self
                .upgrades
                .iter()
                .map(|upgrade| upgrade.backend)
                .collect::<BTreeSet<_>>();
            if self.operation == "accept"
                && (self.upgrades.len() != 2
                    || upgrade_backends
                        != BTreeSet::from([StorageBackend::Sqlite, StorageBackend::Postgres]))
            {
                bail!("passing acceptance evidence requires both observed package activations");
            }
            if self.operation == "accept" {
                let target = self
                    .target
                    .as_ref()
                    .context("accept evidence target is absent")?;
                for upgrade in &self.upgrades {
                    let source_active = self.runtime.iter().any(|runtime| {
                        runtime.backend == upgrade.backend
                            && runtime.revision == self.source
                            && runtime.package == upgrade.source_package
                    });
                    let target_active = self.runtime.iter().any(|runtime| {
                        runtime.backend == upgrade.backend
                            && runtime.revision == *target
                            && runtime.package == upgrade.target_package
                    });
                    if !source_active || !target_active {
                        bail!(
                            "package activation does not match a recorded source and target runtime"
                        );
                    }
                }
            }
            if self.operation == "discover" && !self.upgrades.is_empty() {
                bail!("discovery evidence must not claim package activation");
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

/// Raw, unpublished output from one discovery graph execution.
///
/// The controller owns the decision to serialize or publish its [`Evidence`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryResult {
    pub runtime: Vec<RuntimeIdentity>,
    pub backups: Vec<BackupIdentity>,
    pub upgrades: Vec<UpgradeIdentity>,
    pub lifecycle: Vec<TimedOutcome>,
    pub checks: Vec<TimedOutcome>,
    pub failure_classes: Vec<FailureClass>,
    pub outcome: Outcome,
}

impl DiscoveryResult {
    /// Materializes discovery evidence without choosing a durable destination.
    pub fn into_evidence(self, harness: HarnessIdentity, source: ResolvedRevision) -> Evidence {
        Evidence {
            schema_version: 1,
            operation: "discover".into(),
            outcome: self.outcome,
            harness,
            source,
            target: None,
            runtime: self.runtime,
            backups: self.backups,
            upgrades: self.upgrades,
            lifecycle: self.lifecycle,
            checks: self.checks,
            gaps: Vec::new(),
            findings: Vec::new(),
            failure_classes: self.failure_classes,
        }
    }

    fn into_accept_evidence(
        self,
        harness: HarnessIdentity,
        source: ResolvedRevision,
        target: ResolvedRevision,
    ) -> Evidence {
        Evidence {
            schema_version: 1,
            operation: "accept".into(),
            outcome: self.outcome,
            harness,
            source,
            target: Some(target),
            runtime: self.runtime,
            backups: self.backups,
            upgrades: self.upgrades,
            lifecycle: self.lifecycle,
            checks: self.checks,
            gaps: Vec::new(),
            findings: Vec::new(),
            failure_classes: self.failure_classes,
        }
    }
}

/// A discovery workflow expressed solely through raw-recording operations.
#[derive(Deserialize)]
struct BehaviorCheck {
    id: String,
    outcome: TimedOutcomeStatus,
    duration_ms: u64,
}

pub trait DiscoveryGraph {
    fn run(&mut self, recorder: &mut DiscoveryRecorder) -> Result<()>;
}

/// Records the observable work of a discovery graph.
///
/// Every fallible operation is timed before its result is returned. Failures
/// are classified at their concrete boundary: product behavior/backup work,
/// infrastructure lifecycle work, or harness protocol violations.
#[derive(Debug, Default)]
pub struct DiscoveryRecorder {
    runtime: Vec<RuntimeIdentity>,
    backups: Vec<BackupIdentity>,
    upgrades: Vec<UpgradeIdentity>,
    lifecycle: Vec<TimedOutcome>,
    checks: Vec<TimedOutcome>,
    failure_classes: Vec<FailureClass>,
}

impl DiscoveryRecorder {
    pub fn lifecycle<T>(
        &mut self,
        id: impl Into<String>,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let result = Self::timed(&mut self.lifecycle, id.into(), operation);
        if result.is_err() {
            self.classify_failure(FailureClass::Infrastructure);
        }
        result
    }

    pub fn check<T>(
        &mut self,
        id: &'static str,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        if !CHECK_IDS.contains(&id) {
            bail!("unknown fixed check id {id}");
        }
        let start = Instant::now();
        let result = operation();
        let duration_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        if let Some(entry) = self.checks.iter_mut().find(|entry| entry.id == id) {
            entry.duration_ms = entry.duration_ms.saturating_add(duration_ms);
            if result.is_err() {
                entry.outcome = TimedOutcomeStatus::Failed;
            }
        } else {
            self.checks.push(TimedOutcome {
                id: id.into(),
                outcome: if result.is_ok() {
                    TimedOutcomeStatus::Passed
                } else {
                    TimedOutcomeStatus::Failed
                },
                duration_ms,
            });
        }
        if result.is_err() {
            self.classify_failure(match id {
                "backup"
                | "restore-sqlite-sqlite"
                | "restore-sqlite-postgres"
                | "restore-postgres-sqlite"
                | "restore-postgres-postgres" => FailureClass::Product,
                "service-restart" | "vm-reboot" | "upgrade" => FailureClass::Infrastructure,
                _ => FailureClass::Harness,
            });
        }
        result
    }

    fn behavior_checks(&mut self, checks: Vec<BehaviorCheck>) -> Result<()> {
        let mut seen = BTreeSet::new();
        for check in checks {
            if !CHECK_IDS.contains(&check.id.as_str()) || !seen.insert(check.id.clone()) {
                bail!("behavior flow emitted an unknown or duplicate fixed check");
            }
            if !matches!(check.outcome, TimedOutcomeStatus::Passed) {
                self.classify_failure(FailureClass::Product);
                bail!("behavior flow emitted a non-passing fixed check");
            }
            if let Some(entry) = self.checks.iter_mut().find(|entry| entry.id == check.id) {
                entry.duration_ms = entry.duration_ms.saturating_add(check.duration_ms);
            } else {
                self.checks.push(TimedOutcome {
                    id: check.id,
                    outcome: TimedOutcomeStatus::Passed,
                    duration_ms: check.duration_ms,
                });
            }
        }
        Ok(())
    }
    fn behavior(&mut self, operation: impl FnOnce() -> Result<Vec<BehaviorCheck>>) -> Result<()> {
        match operation() {
            Ok(checks) => self.behavior_checks(checks),
            Err(error) => {
                self.classify_failure(FailureClass::Product);
                Err(error)
            }
        }
    }

    pub fn runtime(&mut self, identity: RuntimeIdentity) {
        self.runtime.push(identity);
    }

    pub fn backup(&mut self, identity: BackupIdentity) {
        self.backups.push(identity);
    }

    pub fn upgrade(&mut self, identity: UpgradeIdentity) {
        self.upgrades.push(identity);
    }

    pub fn classify_failure(&mut self, class: FailureClass) {
        if !self.failure_classes.contains(&class) {
            self.failure_classes.push(class);
        }
    }

    fn timed<T>(
        entries: &mut Vec<TimedOutcome>,
        id: String,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        if id.is_empty() || entries.iter().any(|entry| entry.id == id) {
            bail!("discovery record has an empty or duplicate id");
        }
        let start = Instant::now();
        let result = operation();
        entries.push(TimedOutcome {
            id,
            outcome: if result.is_ok() {
                TimedOutcomeStatus::Passed
            } else {
                TimedOutcomeStatus::Failed
            },
            duration_ms: start.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        });
        result
    }

    fn finish(mut self, succeeded: bool) -> DiscoveryResult {
        let has_failures = self
            .lifecycle
            .iter()
            .chain(&self.checks)
            .any(|entry| matches!(entry.outcome, TimedOutcomeStatus::Failed));
        let complete = CHECK_IDS.iter().all(|id| {
            self.checks.iter().any(|entry| {
                entry.id == *id
                    && (entry.outcome == TimedOutcomeStatus::Passed
                        || (*id == "upgrade" && entry.outcome == TimedOutcomeStatus::Skipped))
            })
        });
        let outcome = if succeeded && !has_failures && complete {
            Outcome::Passed
        } else {
            if self.failure_classes.is_empty() {
                self.classify_failure(FailureClass::Harness);
            }
            Outcome::Failed
        };
        DiscoveryResult {
            runtime: self.runtime,
            backups: self.backups,
            upgrades: self.upgrades,
            lifecycle: self.lifecycle,
            checks: self.checks,
            failure_classes: self.failure_classes,
            outcome,
        }
    }
}

/// Runs a supplied graph and returns raw evidence ingredients only.
pub fn run_discovery_graph(graph: &mut impl DiscoveryGraph) -> DiscoveryResult {
    let mut recorder = DiscoveryRecorder::default();
    let succeeded = graph.run(&mut recorder).is_ok();
    recorder.finish(succeeded)
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
    out.push_str("\n## Package activations\n\n");
    for upgrade in &evidence.upgrades {
        out.push_str(&format!(
            "- {:?}; source SHA-256 `{}`; target SHA-256 `{}`; binary changed {}; source schema {}; target schema {}; schema migrated {}\n",
            upgrade.backend,
            upgrade.source_package.executable_sha256,
            upgrade.target_package.executable_sha256,
            upgrade.binary_changed,
            upgrade.source_schema_version,
            upgrade.target_schema_version,
            upgrade.schema_migrated,
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

fn evidence_destination(root: &Path, evidence: &Evidence) -> Result<PathBuf> {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("reading evidence publication date")?
        .as_secs()
        / 86_400;
    let (year, month, day) = civil_date(days as i64);
    let target = evidence
        .target
        .as_ref()
        .map_or("none", |revision| &revision.commit[..12]);
    Ok(root.join("docs/evidence/production-baseline").join(format!(
        "{year:04}-{month:02}-{day:02}-{}-{}-{target}-{}",
        evidence.operation,
        &evidence.source.commit[..12],
        &evidence.harness.commit[..12],
    )))
}

fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    (
        year + (mp >= 10) as i64,
        (mp + if mp < 10 { 3 } else { -9 }) as u32,
        (doy - (153 * mp + 2) / 5 + 1) as u32,
    )
}

fn publication_workspace(root: &Path) -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("reading evidence staging nonce")?
        .as_nanos();
    let path = root
        .join(".xtask/production-baseline")
        .join(format!("publication-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&path).context("creating restricted evidence staging workspace")?;
    restrict_evidence_workspace(&path)?;
    Ok(path)
}

#[cfg(unix)]
fn restrict_evidence_workspace(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .context("restricting evidence staging workspace")
}

#[cfg(not(unix))]
fn restrict_evidence_workspace(_: &Path) -> Result<()> {
    Ok(())
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
    fs::create_dir_all(
        destination
            .parent()
            .context("evidence destination has no parent")?,
    )
    .context("creating durable evidence parent")?;
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
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let metadata = entry
            .file_type()
            .context("reading retained evidence file type")?;
        if !metadata.is_file() || metadata.is_symlink() {
            bail!("durable evidence contains a prohibited file type");
        }
        names.insert(entry.file_name().to_string_lossy().into_owned());
    }
    let expected = BTreeSet::from(["summary.json".to_owned(), "summary.md".to_owned()]);
    if names != expected {
        bail!("durable evidence violates the two-file allowlist");
    }
    Ok(())
}

fn persist_workflow_error(
    lifecycle: &crate::production_baseline_lifecycle::BaselineLifecycle,
    error: &anyhow::Error,
) -> Result<()> {
    let path = lifecycle.private_path("workflow-error.txt")?;
    fs::write(&path, format!("{error:#}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn run_discovery(
    root: &Path,
    harness: &HarnessIdentity,
    source: ResolvedRevision,
) -> (DiscoveryResult, Result<EvidenceCanaries>, Option<PathBuf>) {
    use crate::production_baseline_lifecycle::BaselineLifecycle;

    let mut workflow_ok = false;
    let mut retained_workspace = None;
    let mut recorder = DiscoveryRecorder::default();
    let workflow = (|| -> Result<EvidenceCanaries> {
        let mut lifecycle = recorder.lifecycle("workspace-create", || {
            BaselineLifecycle::create_immutable(root, &harness.commit)
        })?;
        let canary_path = lifecycle.private_path("run-canaries.jsonl")?;
        fs::File::create(&canary_path)?;
        let result: Result<()> = (|| {
            for (source_id, source_backend) in [
                ("discover-source-sqlite", StorageBackend::Sqlite),
                ("discover-source-postgres", StorageBackend::Postgres),
            ] {
                let runtime = recorder.lifecycle(format!("{source_id}-start"), || {
                    lifecycle.start(source_id, source_backend, source.clone())
                })?;
                let deployment_id = runtime.deployment_id.clone();
                recorder.runtime(runtime);
                recorder.lifecycle(format!("{source_id}-configure-base-url"), || {
                    lifecycle.configure_base_url(&deployment_id)
                })?;
                let state = lifecycle.private_path(&format!("{source_id}.json"))?;
                let seed_process = lifecycle.seed_process(source_id)?;
                recorder.behavior(|| {
                    run_shared_behavior(&state, &canary_path, "create", Some(&seed_process))
                })?;
                recorder.check("service-restart", || lifecycle.restart_service(source_id))?;
                lifecycle.select_proxy(source_id)?;
                recorder
                    .behavior(|| run_shared_behavior(&state, &canary_path, "read-only", None))?;
                recorder.check("vm-reboot", || lifecycle.reboot(source_id))?;
                lifecycle.select_proxy(source_id)?;

                recorder
                    .behavior(|| run_shared_behavior(&state, &canary_path, "read-only", None))?;
                let backup = recorder.check("backup", || lifecycle.backup(source_id))?;
                for (target_backend, direction, check_id) in [
                    (StorageBackend::Sqlite, "sqlite", "restore-sqlite-sqlite"),
                    (
                        StorageBackend::Postgres,
                        "postgres",
                        "restore-sqlite-postgres",
                    ),
                ] {
                    let check_id = if source_backend == StorageBackend::Postgres {
                        if target_backend == StorageBackend::Sqlite {
                            "restore-postgres-sqlite"
                        } else {
                            "restore-postgres-postgres"
                        }
                    } else {
                        check_id
                    };
                    let target_id = format!("discover-target-{source_id}-{direction}");
                    let target_runtime = recorder
                        .lifecycle(format!("{target_id}-start"), || {
                            lifecycle.start(&target_id, target_backend, source.clone())
                        })?;
                    recorder.runtime(target_runtime);
                    let identity = recorder.check(check_id, || {
                        lifecycle.restore(&target_id, &backup)?;
                        let target_schema = lifecycle.observe_schema(&target_id)?;
                        if target_schema != backup.schema_version {
                            bail!("restored target schema differs from backup schema");
                        }
                        lifecycle.select_proxy(&target_id)?;
                        Ok(BackupIdentity {
                            source_backend,
                            target_backend,
                            format_version: backup.format_version,
                            source_schema_version: backup.schema_version,
                            target_schema_version: target_schema,
                            sha256: backup.sha256.clone(),
                        })
                    })?;
                    recorder.backup(identity);
                    recorder
                        .behavior(|| run_shared_behavior(&state, &canary_path, "restored", None))?;
                    recorder
                        .lifecycle(format!("{target_id}-park"), || lifecycle.park(&target_id))?;
                }
                recorder.lifecycle(format!("{source_id}-park"), || lifecycle.park(source_id))?;
            }
            recorder.checks.push(TimedOutcome {
                id: "upgrade".into(),
                outcome: TimedOutcomeStatus::Skipped,
                duration_ms: 0,
            });
            Ok(())
        })();
        let canaries = lifecycle.evidence_canaries(&[&canary_path]);
        if let Err(error) = &result {
            let _ = persist_workflow_error(&lifecycle, error);
        }
        let shutdown = if result.is_ok() {
            lifecycle.cleanup().map(|()| None)
        } else {
            lifecycle.retain_for_diagnostics().map(Some)
        };
        workflow_ok = result.is_ok() && shutdown.is_ok();
        retained_workspace = shutdown?;
        canaries
    })();
    let evidence = recorder.finish(workflow_ok);
    (evidence, workflow, retained_workspace)
}
fn run_acceptance(
    root: &Path,
    harness: &HarnessIdentity,
    source: ResolvedRevision,
    target: ResolvedRevision,
) -> (DiscoveryResult, Result<EvidenceCanaries>, Option<PathBuf>) {
    use crate::production_baseline_lifecycle::BaselineLifecycle;

    let mut workflow_ok = false;
    let mut retained_workspace = None;
    let mut recorder = DiscoveryRecorder::default();
    let workflow = (|| -> Result<EvidenceCanaries> {
        let mut lifecycle = recorder.lifecycle("workspace-create", || {
            BaselineLifecycle::create_immutable(root, &harness.commit)
        })?;
        let canary_path = lifecycle.private_path("run-canaries.jsonl")?;
        fs::File::create(&canary_path)?;
        let result: Result<()> = (|| {
            for (source_id, source_backend) in [
                ("accept-source-sqlite", StorageBackend::Sqlite),
                ("accept-source-postgres", StorageBackend::Postgres),
            ] {
                let mut source_runtime = recorder
                    .lifecycle(format!("{source_id}-start"), || {
                        lifecycle.start(source_id, source_backend, source.clone())
                    })?;
                source_runtime.deployment_id = format!("{source_id}-package-source");
                recorder.runtime(source_runtime.clone());
                recorder.lifecycle(format!("{source_id}-configure-base-url"), || {
                    lifecycle.configure_base_url(source_id)
                })?;
                let state = lifecycle.private_path(&format!("{source_id}.json"))?;
                let seed_process = lifecycle.seed_process(source_id)?;
                recorder.behavior(|| {
                    run_shared_behavior(&state, &canary_path, "create", Some(&seed_process))
                })?;
                recorder.check("service-restart", || lifecycle.restart_service(source_id))?;
                lifecycle.select_proxy(source_id)?;
                recorder
                    .behavior(|| run_shared_behavior(&state, &canary_path, "read-only", None))?;
                recorder.check("vm-reboot", || lifecycle.reboot(source_id))?;
                lifecycle.select_proxy(source_id)?;
                recorder
                    .behavior(|| run_shared_behavior(&state, &canary_path, "read-only", None))?;
                let source_schema = recorder
                    .lifecycle(format!("{source_id}-source-schema"), || {
                        lifecycle.observe_schema(source_id)
                    })?;
                let mut target_runtime =
                    recorder.check("upgrade", || lifecycle.upgrade(source_id, target.clone()))?;
                target_runtime.deployment_id = format!("{source_id}-package-target");
                let target_schema = recorder
                    .lifecycle(format!("{source_id}-target-schema"), || {
                        lifecycle.observe_schema(source_id)
                    })?;
                recorder.upgrade(UpgradeIdentity::observe(
                    source_backend,
                    source_runtime.package.clone(),
                    target_runtime.package.clone(),
                    source_schema,
                    target_schema,
                )?);
                recorder.runtime(target_runtime);
                lifecycle.select_proxy(source_id)?;
                recorder
                    .behavior(|| run_shared_behavior(&state, &canary_path, "read-only", None))?;
                recorder
                    .behavior(|| run_shared_behavior(&state, &canary_path, "restored", None))?;
                let backup = recorder.check("backup", || lifecycle.backup(source_id))?;
                for target_backend in [StorageBackend::Sqlite, StorageBackend::Postgres] {
                    let check_id = match (source_backend, target_backend) {
                        (StorageBackend::Sqlite, StorageBackend::Sqlite) => "restore-sqlite-sqlite",
                        (StorageBackend::Sqlite, StorageBackend::Postgres) => {
                            "restore-sqlite-postgres"
                        }
                        (StorageBackend::Postgres, StorageBackend::Sqlite) => {
                            "restore-postgres-sqlite"
                        }
                        (StorageBackend::Postgres, StorageBackend::Postgres) => {
                            "restore-postgres-postgres"
                        }
                    };
                    let target_id = format!(
                        "accept-target-{}-{}",
                        source_id,
                        match target_backend {
                            StorageBackend::Sqlite => "sqlite",
                            StorageBackend::Postgres => "postgres",
                        }
                    );
                    let mut restore_runtime = recorder
                        .lifecycle(format!("{target_id}-start"), || {
                            lifecycle.start(&target_id, target_backend, target.clone())
                        })?;
                    restore_runtime.deployment_id = format!("{target_id}-package-target");
                    recorder.runtime(restore_runtime);
                    let identity = recorder.check(check_id, || {
                        lifecycle.restore(&target_id, &backup)?;
                        let target_schema = lifecycle.observe_schema(&target_id)?;
                        if target_schema != backup.schema_version {
                            bail!("restored target schema differs from target backup schema");
                        }
                        lifecycle.select_proxy(&target_id)?;
                        Ok(BackupIdentity {
                            source_backend,
                            target_backend,
                            format_version: backup.format_version,
                            source_schema_version: backup.schema_version,
                            target_schema_version: target_schema,
                            sha256: backup.sha256.clone(),
                        })
                    })?;
                    recorder.backup(identity);
                    recorder
                        .behavior(|| run_shared_behavior(&state, &canary_path, "restored", None))?;
                    recorder
                        .lifecycle(format!("{target_id}-park"), || lifecycle.park(&target_id))?;
                }
                recorder.lifecycle(format!("{source_id}-park"), || lifecycle.park(source_id))?;
            }
            Ok(())
        })();
        let canaries = lifecycle.evidence_canaries(&[&canary_path]);
        if let Err(error) = &result {
            let _ = persist_workflow_error(&lifecycle, error);
        }
        let shutdown = if result.is_ok() {
            lifecycle.cleanup().map(|()| None)
        } else {
            lifecycle.retain_for_diagnostics().map(Some)
        };
        workflow_ok = result.is_ok() && shutdown.is_ok();
        retained_workspace = shutdown?;
        canaries
    })();
    let evidence = recorder.finish(workflow_ok);
    (evidence, workflow, retained_workspace)
}

/// Invoke the Task 1 shared Playwright behavior flow against the current stable
/// proxy origin. Its state path is lifecycle-owned and never published.
fn run_shared_behavior(
    state: &Path,
    canary_path: &Path,
    phase: &str,
    seed_process: Option<&Path>,
) -> Result<Vec<BehaviorCheck>> {
    #[derive(Deserialize)]
    struct BehaviorResult {
        phase: String,
        checks: Vec<BehaviorCheck>,
    }
    let mut command = Command::new("playwright");
    command
        .args([
            "test",
            "tests/production-baseline-harness.spec.ts",
            "--project=chromium",
            "--workers=1",
            "--no-deps",
        ])
        .current_dir("end2end")
        .env("JAUNDER_E2E_BASE_URL", "https://localhost:8443")
        .env("JAUNDER_PRODUCTION_BASELINE_TLS", "1")
        .env("JAUNDER_PRODUCTION_BASELINE_STATE", state)
        .env("JAUNDER_PRODUCTION_BASELINE_PHASE", phase)
        .env("JAUNDER_PRODUCTION_BASELINE_CANARY_PATH", canary_path);
    if let Some(seed_process) = seed_process {
        command.env("JAUNDER_E2E_SEED_PROCESS", seed_process);
    }
    let output = command
        .output()
        .context("running shared production-baseline behavior flow")?;
    if !output.status.success() {
        retain_behavior_output(state, phase, "stdout", &output.stdout)?;
        retain_behavior_output(state, phase, "stderr", &output.stderr)?;
        bail!("shared production-baseline behavior flow failed during {phase}");
    }
    let stdout = String::from_utf8(output.stdout).context("behavior flow output was not UTF-8")?;
    let result = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("production-baseline-result="))
        .map(serde_json::from_str::<BehaviorResult>)
        .next_back()
        .context("behavior flow omitted machine-readable result")??;
    if result.phase != phase || result.checks.is_empty() {
        bail!("behavior flow emitted an invalid machine-readable result");
    }
    Ok(result.checks)
}

fn retain_behavior_output(state: &Path, phase: &str, stream: &str, bytes: &[u8]) -> Result<()> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("reading behavior diagnostic nonce")?
        .as_nanos();
    let path = state
        .parent()
        .context("baseline state path has no workspace")?
        .join(format!("behavior-{phase}-{nonce}.{stream}"));
    fs::write(&path, bytes).context("retaining restricted behavior diagnostics")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .context("restricting behavior diagnostics")?;
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
    result.push(StepResult::ok("production-baseline-preflight").with_duration(start.elapsed()));
    let (raw, canaries, retained_workspace) = match target {
        Some(target) => {
            let (raw, canaries, retained_workspace) =
                run_acceptance(&root, &harness, source.clone(), target.clone());
            (
                raw.into_accept_evidence(harness.clone(), source, target),
                canaries,
                retained_workspace,
            )
        }
        None => {
            let (raw, canaries, retained_workspace) =
                run_discovery(&root, &harness, source.clone());
            (
                raw.into_evidence(harness.clone(), source),
                canaries,
                retained_workspace,
            )
        }
    };
    let destination = evidence_destination(&root, &raw)?;
    let detail = format!(
        "destination={} runtime-identities={} upgrades={} backups={} checks={}",
        destination
            .strip_prefix(&root)
            .unwrap_or(&destination)
            .display(),
        raw.runtime.len(),
        raw.upgrades.len(),
        raw.backups.len(),
        raw.checks.len()
    );
    let publication = raw.validate().and_then(|()| {
        let workspace = publication_workspace(&root)?;
        let published =
            canaries.and_then(|canaries| publish(&workspace, &destination, &raw, &canaries));
        fs::remove_dir_all(&workspace).context("cleaning restricted evidence staging")?;
        published
    });
    let step = if raw.operation == "accept" {
        "production-baseline-accept"
    } else {
        "production-baseline-discovery"
    };
    if matches!(raw.outcome, Outcome::Passed) && publication.is_ok() {
        result.push(
            StepResult::ok(step)
                .detail(detail)
                .with_duration(start.elapsed()),
        );
    } else {
        let unpublished_detail = retained_workspace
            .as_deref()
            .and_then(|workspace| workspace.strip_prefix(&root).ok())
            .map(|workspace| {
                format!(
                    "evidence was not published; restricted-workspace={}",
                    workspace.display()
                )
            })
            .unwrap_or_else(|| "evidence was not published".into());
        result.push(
            StepResult::fail(step)
                .detail(if publication.is_ok() {
                    detail
                } else {
                    unpublished_detail
                })
                .with_duration(start.elapsed()),
        );
    }
    Ok(result)
}

/// Opt-in Task 3 smoke against the caller's checkout. This intentionally has no
/// clean-harness or upstream qualification boundary and publishes no evidence.
pub fn run_current_checkout_smoke() -> Result<CommandResult> {
    let root = PathBuf::from(git::toplevel(Path::new("."))?);
    let start = Instant::now();
    let _lease = RunLease::acquire(&root)?;
    let commit = String::from_utf8(git::at(&root).args(["rev-parse", "HEAD"]).output()?.stdout)?
        .trim()
        .to_owned();
    let revision = ResolvedRevision {
        commit,
        flake_ref: format!("path:{}", root.display()),
    };
    let runtime = crate::production_baseline_lifecycle::BaselineLifecycle::smoke(&root, revision)?;
    let mut result = CommandResult::new("nix-production-baseline-smoke");
    result.push(
        StepResult::ok("production-baseline-lifecycle-smoke")
            .detail(format!("runtime-identities={}", runtime.len()))
            .with_duration(start.elapsed()),
    );
    crate::lifecycle::finalize(&mut result, start);
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
            upgrades: vec![],
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

    fn package(executable_sha256: char) -> PackageIdentity {
        PackageIdentity {
            installable: "github:jaunder-org/jaunder/test#jaunder".into(),
            derivation: "/nix/store/test.drv".into(),
            output_path: "/nix/store/test".into(),
            nar_hash: "nar".into(),
            executable_sha256: executable_sha256.to_string().repeat(64),
        }
    }

    #[test]
    fn upgrade_classification_preserves_identical_binary_and_schema() {
        let upgrade =
            UpgradeIdentity::observe(StorageBackend::Sqlite, package('a'), package('a'), 4, 4)
                .unwrap();

        assert!(!upgrade.binary_changed);
        assert!(!upgrade.schema_migrated);
    }

    #[test]
    fn upgrade_classification_requires_observed_binary_and_schema_changes() {
        let upgrade =
            UpgradeIdentity::observe(StorageBackend::Postgres, package('a'), package('b'), 4, 5)
                .unwrap();

        assert!(upgrade.binary_changed);
        assert!(upgrade.schema_migrated);
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
    fn passing_acceptance_evidence_requires_target_activations_and_all_recovery_directions() {
        let mut evidence = evidence();
        let target = ResolvedRevision {
            commit: "e".repeat(40),
            flake_ref: format!("github:jaunder-org/jaunder/{}", "e".repeat(40)),
        };
        evidence.operation = "accept".into();
        evidence.outcome = Outcome::Passed;
        evidence.target = Some(target.clone());
        evidence.failure_classes.clear();
        evidence.checks = CHECK_IDS
            .map(|id| TimedOutcome {
                id: id.into(),
                outcome: TimedOutcomeStatus::Passed,
                duration_ms: 1,
            })
            .to_vec();
        let source_package = package('a');
        let target_package = package('b');
        evidence.runtime = [
            (
                evidence.source.clone(),
                StorageBackend::Sqlite,
                "source-sqlite",
            ),
            (
                evidence.source.clone(),
                StorageBackend::Postgres,
                "source-postgres",
            ),
            (target.clone(), StorageBackend::Sqlite, "target-sqlite"),
            (target.clone(), StorageBackend::Postgres, "target-postgres"),
        ]
        .into_iter()
        .map(|(revision, backend, deployment_id)| RuntimeIdentity {
            deployment_id: deployment_id.into(),
            backend,
            revision: revision.clone(),
            executable: format!("{}/bin/jaunder", package('a').output_path),
            exec_start: format!("{}/bin/jaunder serve", package('a').output_path),
            package: if revision == target {
                target_package.clone()
            } else {
                source_package.clone()
            },
        })
        .collect();
        evidence.backups = [
            (StorageBackend::Sqlite, StorageBackend::Sqlite),
            (StorageBackend::Sqlite, StorageBackend::Postgres),
            (StorageBackend::Postgres, StorageBackend::Sqlite),
            (StorageBackend::Postgres, StorageBackend::Postgres),
        ]
        .into_iter()
        .map(|(source_backend, target_backend)| BackupIdentity {
            source_backend,
            target_backend,
            format_version: 1,
            source_schema_version: 4,
            target_schema_version: 4,
            sha256: "f".repeat(64),
        })
        .collect();
        evidence.upgrades = [StorageBackend::Sqlite, StorageBackend::Postgres]
            .into_iter()
            .map(|backend| {
                UpgradeIdentity::observe(
                    backend,
                    source_package.clone(),
                    target_package.clone(),
                    4,
                    4,
                )
                .unwrap()
            })
            .collect();

        evidence.validate().unwrap();
    }

    struct SparsePassingDiscovery;

    impl DiscoveryGraph for SparsePassingDiscovery {
        fn run(&mut self, recorder: &mut DiscoveryRecorder) -> Result<()> {
            recorder.check("feeds", || Ok(()))
        }
    }

    struct FailingDiscovery;

    impl DiscoveryGraph for FailingDiscovery {
        fn run(&mut self, recorder: &mut DiscoveryRecorder) -> Result<()> {
            recorder.lifecycle("deploy", || bail!("fixture lifecycle failure"))
        }
    }

    #[test]
    fn sparse_discovery_cannot_pass_without_every_fixed_check() {
        let result = run_discovery_graph(&mut SparsePassingDiscovery);

        assert_eq!(result.outcome, Outcome::Failed);
        assert_eq!(result.checks.len(), 1);
        assert_eq!(result.failure_classes, vec![FailureClass::Harness]);
    }

    #[test]
    fn raw_discovery_failure_is_timed_classified_and_materializes_evidence() {
        let result = run_discovery_graph(&mut FailingDiscovery);
        let template = evidence();
        let evidence = result.into_evidence(template.harness, template.source);

        assert_eq!(evidence.outcome, Outcome::Failed);
        assert_eq!(evidence.failure_classes, vec![FailureClass::Infrastructure]);
        assert_eq!(evidence.lifecycle.len(), 1);
        assert_eq!(evidence.lifecycle[0].id, "deploy");
        assert_eq!(evidence.lifecycle[0].outcome, TimedOutcomeStatus::Failed);
        evidence.validate().unwrap();
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
        let workspace = publication_workspace(temp.path()).unwrap();
        let destination = temp.path().join("durable");
        publish(&workspace, &destination, &evidence(), &safe_canaries).unwrap();
        validate_allowlist(&destination).unwrap();

        let leaked_operation = EvidenceCanaries::new(
            vec!["discover".into()],
            vec!["unpublished-session".into()],
            vec!["unpublished-private-material".into()],
        )
        .unwrap();
        assert!(
            publish(
                &workspace,
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
                &workspace,
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
                &workspace,
                &temp.path().join("redacted"),
                &redacted,
                &safe_canaries,
            )
            .is_err()
        );
        assert!(!temp.path().join("redacted").exists());
    }
}
