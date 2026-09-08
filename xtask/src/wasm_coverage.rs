//! Host-side reconciliation for the independent Playwright/WASM coverage producers.
//!
//! The Nix producers are evidence producers, not gates: this consumer always realizes
//! both, retains each unpacked root, then decides whether the two profiles are eligible
//! to be combined.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use coverage::wasm::{Artifact, BrowserStatus, Outcome, Stage};
use csr_bundle::{Manifest, Role};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tar::Archive;

const BROWSERS: [&str; 2] = ["chromium", "firefox"];
const ROOT: &str = ".xtask/wasm-coverage";
const AGGREGATE: &str = ".xtask/wasm-coverage/status.json";

trait CommandRunner {
    fn run(&self, command: &mut Command) -> Result<Output>;
}

struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&self, command: &mut Command) -> Result<Output> {
        command.output().context("starting coverage subprocess")
    }
}

fn browser_stages(status: &BrowserStatus) -> [(&'static str, &Stage); 3] {
    [
        ("csr_structural", &status.csr_structural),
        ("diagnostic_export", &status.diagnostic_export),
        ("source_mapping", &status.source_mapping),
    ]
}

fn all_browser_stages_pass(status: &BrowserStatus) -> bool {
    browser_stages(status)
        .into_iter()
        .all(|(_, stage)| stage.outcome == Outcome::Passed)
}

#[derive(Clone, Debug, Serialize)]
pub struct Aggregate {
    pub version: &'static str,
    pub browsers: BTreeMap<String, AggregateBrowser>,
    pub verdict: Verdict,
    pub blockers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merged: Option<MergedEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AggregateBrowser {
    pub artifact_root: String,
    pub status: Option<BrowserStatus>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    Passed,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct MergedEvidence {
    pub profile_data: String,
    pub report: String,
}

/// Validate a producer's complete evidence root. Paths are checked before opening so
/// a manifest cannot direct the host outside the retained producer root.
pub fn validate_browser(root: &Path, expected_browser: &str) -> Result<BrowserStatus> {
    let status: BrowserStatus = serde_json::from_slice(
        &fs::read(root.join("status.json")).context("reading producer status.json")?,
    )
    .context("parsing producer status.json")?;
    if status.version != "v1" {
        bail!("unsupported status version {:?}", status.version);
    }
    if status.requested_browser != expected_browser {
        bail!("requested browser identity does not match {expected_browser}");
    }
    let early_failure = matches!(status.actual_browser.as_str(), "unknown" | "not-started")
        && status.csr_structural.outcome != Outcome::NotRun
        && status.diagnostic_export.outcome == Outcome::Failed
        && status.source_mapping.outcome == Outcome::NotRun
        && status.module_signature.is_none()
        && status.toolchain_identity.is_none();
    if status.actual_browser != expected_browser && !early_failure {
        bail!("actual browser identity does not match {expected_browser}");
    }
    for (name, stage) in browser_stages(&status) {
        validate_stage(name, stage)?;
    }
    if status.diagnostic_export.outcome == Outcome::Passed
        && status.csr_structural.outcome != Outcome::Passed
    {
        bail!("diagnostic export passed after structural failure");
    }
    if status.source_mapping.outcome == Outcome::Passed
        && status.diagnostic_export.outcome != Outcome::Passed
    {
        bail!("source mapping passed without a successful diagnostic export");
    }
    if status.diagnostic_export.outcome == Outcome::Passed
        && (status.module_signature.as_deref().is_none_or(str::is_empty)
            || status.toolchain_identity.is_none())
    {
        bail!("successful diagnostic export lacks module signature or toolchain identity");
    }
    validate_conditional_artifacts(&status)?;
    validate_served_module(&status)?;
    reconcile_manifest(root, &status.artifacts)?;
    Ok(status)
}

fn validate_stage(name: &str, stage: &Stage) -> Result<()> {
    let blocker = stage
        .blocker
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    match stage.outcome {
        Outcome::Passed if stage.blocker.is_some() => bail!("{name} passed with a blocker"),
        Outcome::Failed | Outcome::NotRun if blocker.is_none() => {
            bail!("{name} did not pass and has no blocker")
        }
        _ => Ok(()),
    }
}

fn validate_conditional_artifacts(status: &BrowserStatus) -> Result<()> {
    require(status, "module")?;
    require(status, "diagnostics")?;
    conditional(
        status,
        "profile",
        status.diagnostic_export.outcome == Outcome::Passed,
    )?;
    let mapped = status.source_mapping.outcome == Outcome::Passed;
    conditional(status, "profile_data", mapped)?;
    conditional(status, "mapped_report", mapped)?;
    Ok(())
}

fn require(status: &BrowserStatus, name: &str) -> Result<()> {
    if status.artifacts.contains_key(name) {
        Ok(())
    } else {
        bail!("missing required artifact {name}")
    }
}
fn conditional(status: &BrowserStatus, name: &str, required: bool) -> Result<()> {
    match (required, status.artifacts.contains_key(name)) {
        (true, false) => bail!("missing required artifact {name}"),
        (false, true) => bail!("forbidden artifact {name} is present"),
        _ => Ok(()),
    }
}

fn validate_served_module(status: &BrowserStatus) -> Result<()> {
    match (&status.csr_structural.outcome, &status.served_module) {
        (Outcome::Passed, Some(module)) => {
            let path = relative_path(&module.path).context("served module")?;
            if module.sha256.len() != 64
                || !module.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                bail!("served module has an invalid SHA-256 digest");
            }
            let artifact = &status.artifacts["module"];
            if artifact.path != format!("module/{}", path.display()) {
                bail!("retained module path does not identify the served module");
            }
            if !artifact.sha256.eq_ignore_ascii_case(&module.sha256) {
                bail!("retained module digest does not match the served module");
            }
        }
        (Outcome::Passed, None) => {
            bail!("successful CSR structural validation lacks served module")
        }
        (_, Some(_)) | (_, None) => {}
    }
    Ok(())
}

fn relative_path(path: &str) -> Result<&Path> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!(
            "artifact path is not contained and relative: {}",
            path.display()
        );
    }
    Ok(path)
}

fn reconcile_manifest(root: &Path, artifacts: &BTreeMap<String, Artifact>) -> Result<()> {
    let mut manifest_paths = BTreeSet::new();
    for (name, artifact) in artifacts {
        let relative = relative_path(&artifact.path).with_context(|| format!("artifact {name}"))?;
        if !manifest_paths.insert(relative.to_owned()) {
            bail!("duplicate manifest path {}", relative.display());
        }
        if artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("artifact {name} has an invalid SHA-256 digest");
        }
        let path = root.join(relative);
        let metadata =
            fs::symlink_metadata(&path).with_context(|| format!("missing artifact {name}"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!("artifact {name} is not a regular file");
        }
        let actual = sha256(&path)?;
        if !actual.eq_ignore_ascii_case(&artifact.sha256) {
            bail!("artifact {name} digest mismatch");
        }
    }
    let disk_paths = regular_files(root)?;
    let status = PathBuf::from("status.json");
    let actual: BTreeSet<_> = disk_paths
        .into_iter()
        .filter(|path| path != &status)
        .collect();
    if actual != manifest_paths {
        bail!("artifact manifest does not exactly reconcile with retained files");
    }
    Ok(())
}

fn regular_files(root: &Path) -> Result<BTreeSet<PathBuf>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeSet<PathBuf>) -> Result<()> {
        for entry in
            fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
        {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_symlink() {
                bail!("retained evidence contains a symlink: {}", path.display());
            }
            if kind.is_dir() {
                visit(root, &path, files)?;
            } else if kind.is_file() {
                files.insert(path.strip_prefix(root)?.to_owned());
            } else {
                bail!(
                    "retained evidence contains a non-regular file: {}",
                    path.display()
                );
            }
        }
        Ok(())
    }
    let mut files = BTreeSet::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// The verdict is intentionally independent of runner/build mechanics: only the three
/// producer outcomes decide it. Evidence validation failures remain blockers.
pub fn derive_verdict(statuses: &BTreeMap<String, BrowserStatus>) -> (Verdict, Vec<String>) {
    let mut blockers = Vec::new();
    for browser in BROWSERS {
        match statuses.get(browser) {
            Some(status) => {
                for (name, stage) in browser_stages(status) {
                    if stage.outcome != Outcome::Passed {
                        blockers.push(format!(
                            "{browser}.{name}: {}",
                            stage.blocker.as_deref().unwrap_or("not passed")
                        ));
                    }
                }
            }
            None => blockers.push(format!("{browser}: missing status")),
        }
    }
    (
        if blockers.is_empty() {
            Verdict::Passed
        } else {
            Verdict::Failed
        },
        blockers,
    )
}

fn merge_ineligibility(statuses: &BTreeMap<String, BrowserStatus>) -> Option<&'static str> {
    let Some(chromium) = statuses.get("chromium") else {
        return Some("Chromium status is missing");
    };
    let Some(firefox) = statuses.get("firefox") else {
        return Some("Firefox status is missing");
    };
    if !all_browser_stages_pass(chromium) {
        return Some("Chromium has a non-passing stage");
    }
    if !all_browser_stages_pass(firefox) {
        return Some("Firefox has a non-passing stage");
    }
    if chromium.module_signature != firefox.module_signature {
        return Some("Chromium and Firefox module signatures differ");
    }
    if chromium.toolchain_identity != firefox.toolchain_identity {
        return Some("Chromium and Firefox toolchain identities differ");
    }
    if chromium.served_module.is_none() || firefox.served_module.is_none() {
        return Some("a browser lacks served-module identity");
    }
    if chromium.served_module != firefox.served_module {
        return Some("Chromium and Firefox served-module identities differ");
    }
    let Some(chromium_module) = chromium.artifacts.get("module") else {
        return Some("Chromium lacks its retained module artifact");
    };
    let Some(firefox_module) = firefox.artifacts.get("module") else {
        return Some("Firefox lacks its retained module artifact");
    };
    if chromium_module.sha256 != firefox_module.sha256 {
        return Some("Chromium and Firefox retained module digests differ");
    }
    None
}

pub fn probe() -> Result<Aggregate> {
    probe_with(&ProcessRunner)
}

fn probe_with(runner: &dyn CommandRunner) -> Result<Aggregate> {
    clear_destination(Path::new(ROOT), "clearing stale WASM coverage evidence")?;
    fs::create_dir_all(ROOT)?;
    let mut roots = BTreeMap::new();
    let mut validation_blockers = Vec::new();
    for browser in BROWSERS {
        let root = Path::new(ROOT).join(browser);
        if let Err(error) = realize_and_unpack(runner, browser, &root) {
            validation_blockers.push(format!("{browser}: {error:#}"));
        }
        roots.insert(browser.to_owned(), root);
    }
    let mut statuses = BTreeMap::new();
    for browser in BROWSERS {
        let root = &roots[browser];
        match validate_browser(root, browser) {
            Ok(status) => {
                statuses.insert(browser.to_owned(), status);
            }
            Err(error) => validation_blockers.push(format!("{browser}: {error:#}")),
        }
    }
    let (verdict, mut blockers) = derive_verdict(&statuses);
    blockers.extend(validation_blockers);
    if blockers.is_empty()
        && let Some(blocker) = merge_ineligibility(&statuses)
    {
        blockers.push(format!("merged: {blocker}"));
    }
    let merged = if blockers.is_empty() {
        match merge_profiles(runner, &roots, &statuses) {
            Ok(merged) => Some(merged),
            Err(error) => {
                blockers.push(format!("merged: {error:#}"));
                None
            }
        }
    } else {
        None
    };
    let browsers = BROWSERS
        .into_iter()
        .map(|browser| {
            (
                browser.to_owned(),
                AggregateBrowser {
                    artifact_root: roots[browser].display().to_string(),
                    status: statuses.remove(browser),
                },
            )
        })
        .collect();
    let aggregate = Aggregate {
        version: "v1",
        browsers,
        verdict: if blockers.is_empty() {
            verdict
        } else {
            Verdict::Failed
        },
        blockers,
        merged,
    };
    write_aggregate(&aggregate)?;
    Ok(aggregate)
}

struct NixRealization<'a> {
    package: &'a str,
    output: &'a Path,
    impure_environment: Option<(&'a str, &'a str)>,
}

fn realize_nix(runner: &dyn CommandRunner, realization: NixRealization<'_>) -> Result<()> {
    if let Some(parent) = realization.output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut command = Command::new("nix");
    command.args(["build", "-L"]);
    if realization.impure_environment.is_some() {
        command.arg("--impure");
    }
    command
        .args(["--accept-flake-config", "--out-link"])
        .arg(realization.output)
        .arg(format!(".#{}", realization.package));
    if let Some((name, value)) = realization.impure_environment {
        command.env(name, value);
    }
    let result = runner.run(&mut command)?;
    if !result.status.success() {
        bail!(
            "nix build for {} exited with {}: {}",
            realization.package,
            result.status,
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    Ok(())
}

fn realize_and_unpack(runner: &dyn CommandRunner, browser: &str, root: &Path) -> Result<()> {
    let package = format!("wasm-coverage-{browser}");
    let output = Path::new(".xtask/gcroots").join(&package);
    clear_destination(root, &format!("clearing stale {}", root.display()))?;
    realize_nix(
        runner,
        NixRealization {
            package: &package,
            output: &output,
            impure_environment: None,
        },
    )?;
    unpack_archive(
        &output.join(format!("{package}.tar.gz")),
        root,
        ArchiveLayout::Rooted("wasm-coverage"),
    )
}

fn clear_destination(destination: &Path, context: &str) -> Result<()> {
    match fs::remove_dir_all(destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("{context}: {}", destination.display())),
    }
}

#[derive(Clone, Copy)]
enum ArchiveLayout<'a> {
    Rooted(&'a str),
    SingleFile(&'a str),
}

/// Extract regular archive content only after normalizing each entry to a contained path.
fn unpack_archive(archive: &Path, destination: &Path, layout: ArchiveLayout<'_>) -> Result<()> {
    clear_destination(destination, "clearing archive destination")?;
    fs::create_dir_all(destination)?;
    let mut archive = Archive::new(GzDecoder::new(
        fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?,
    ));
    let mut extracted = false;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let relative = match layout {
            ArchiveLayout::Rooted(root) => {
                let Some(relative) = path.strip_prefix(root).ok() else {
                    bail!("archive entry is outside {root} root: {}", path.display());
                };
                if relative.as_os_str().is_empty() {
                    continue;
                }
                relative_path(&relative.to_string_lossy())?.to_owned()
            }
            ArchiveLayout::SingleFile(expected) => {
                let path_text = path.to_string_lossy();
                let relative = relative_path(&path_text)?;
                if relative != Path::new(expected) {
                    bail!("archive contains an unexpected entry: {}", path.display());
                }
                relative.to_owned()
            }
        };
        let target = destination.join(relative);
        match (layout, entry.header().entry_type()) {
            (ArchiveLayout::Rooted(_), entry_type) if entry_type.is_dir() => {
                fs::create_dir_all(target)?;
            }
            (_, entry_type) if entry_type.is_file() => {
                if extracted && matches!(layout, ArchiveLayout::SingleFile(_)) {
                    bail!("archive contains duplicate measurement evidence");
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                entry.unpack(target)?;
                extracted = true;
            }
            _ => bail!("archive contains a non-regular entry"),
        }
    }
    if matches!(layout, ArchiveLayout::SingleFile(_)) && !extracted {
        bail!("archive lacks measurement evidence");
    }
    Ok(())
}
fn merge_profiles(
    runner: &dyn CommandRunner,
    roots: &BTreeMap<String, PathBuf>,
    statuses: &BTreeMap<String, BrowserStatus>,
) -> Result<MergedEvidence> {
    let csr = realize_csr(runner)?;
    let bundle_root = csr.join("pkg");
    let manifest = Manifest::from_json(
        &fs::read(bundle_root.join("manifest.json")).context("reading CSR manifest")?,
    )
    .context("parsing CSR manifest")?;
    manifest
        .verify_bundle(&bundle_root)
        .context("verifying CSR manifest bundle")?;
    let served = manifest
        .role(Role::Wasm)
        .context("finding CSR wasm module")?;
    let served_path = served.path.as_str();
    let served_sha256 = served.sha256.as_str();
    if BROWSERS.iter().any(|browser| {
        let status = &statuses[*browser];
        status
            .served_module
            .as_ref()
            .is_none_or(|module| module.path != served_path || module.sha256 != served_sha256)
    }) {
        bail!("browser served module does not match the CSR manifest");
    }
    let source_identity: SourceIdentity = serde_json::from_slice(
        &fs::read(csr.join("source-identity.json")).context("reading CSR source identity")?,
    )
    .context("parsing CSR source identity")?;
    let csr_toolchain: Value = serde_json::from_slice(
        &fs::read(csr.join("toolchain-identity.json")).context("reading CSR toolchain identity")?,
    )
    .context("parsing CSR toolchain identity")?;
    if statuses["chromium"].toolchain_identity.as_ref() != Some(&csr_toolchain) {
        bail!("browser toolchain identity does not match the pinned CSR toolchain");
    }
    let output = Path::new(ROOT).join("merged");
    fs::create_dir_all(&output)?;
    let profdata = output.join("browser.profdata");
    let report = output.join("llvm-cov.txt");
    let profiles: Vec<_> = BROWSERS
        .iter()
        .map(|browser| roots[*browser].join(&statuses[*browser].artifacts["profile"].path))
        .collect();
    let compiled = source_identity.compilation_directory.as_str();
    let source = retained_nix_source(&source_identity)?;
    let mut merge = Command::new(csr.join("tools/llvm-profdata"));
    merge
        .arg("merge")
        .arg("-sparse")
        .args(&profiles)
        .arg("-o")
        .arg(&profdata);
    let result = runner.run(&mut merge)?;
    if !result.status.success() {
        bail!(
            "llvm-profdata merge exited with {}: {}",
            result.status,
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    let mut show = Command::new(csr.join("tools/llvm-cov"));
    show.arg("show")
        .arg(csr.join("instrumented/csr.wasm"))
        .arg(format!("-instr-profile={}", profdata.display()))
        .arg(format!("-path-equivalence={compiled},{}", source.display()));
    let result = runner.run(&mut show)?;
    if !result.status.success() {
        bail!(
            "llvm-cov show exited with {}: {}",
            result.status,
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    fs::write(&report, &result.stdout)?;
    let chromium = export_region_counts(
        runner,
        &csr.join("tools/llvm-cov"),
        &csr.join("instrumented/csr.wasm"),
        &roots["chromium"].join(&statuses["chromium"].artifacts["profile_data"].path),
        compiled,
        &source,
        "Chromium",
    )?;
    let firefox = export_region_counts(
        runner,
        &csr.join("tools/llvm-cov"),
        &csr.join("instrumented/csr.wasm"),
        &roots["firefox"].join(&statuses["firefox"].artifacts["profile_data"].path),
        compiled,
        &source,
        "Firefox",
    )?;
    let merged = export_region_counts(
        runner,
        &csr.join("tools/llvm-cov"),
        &csr.join("instrumented/csr.wasm"),
        &profdata,
        compiled,
        &source,
        "merged",
    )?;
    prove_count_union(&chromium, &firefox, &merged)?;
    Ok(MergedEvidence {
        profile_data: profdata.display().to_string(),
        report: report.display().to_string(),
    })
}

#[derive(Deserialize)]
struct SourceIdentity {
    source_identity: NixStoreSource,
    compilation_directory: String,
}

#[derive(Deserialize)]
struct NixStoreSource {
    kind: String,
    value: String,
}

fn retained_nix_source(identity: &SourceIdentity) -> Result<PathBuf> {
    if identity.source_identity.kind != "nix-store-source" {
        bail!("CSR source identity is not a Nix store source");
    }
    if identity.compilation_directory.trim().is_empty()
        || !Path::new(&identity.compilation_directory).is_absolute()
    {
        bail!("CSR compilation directory is not an absolute prefix");
    }
    let source = PathBuf::from(&identity.source_identity.value);
    if !source.starts_with("/nix/store") {
        bail!("CSR source identity is outside the Nix store");
    }
    let metadata = fs::symlink_metadata(&source).context("reading retained Nix source")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("CSR source identity is not a retained source directory");
    }
    let canonical = fs::canonicalize(&source).context("resolving retained Nix source")?;
    if canonical != source || !canonical.starts_with("/nix/store") {
        bail!("CSR source identity does not name an immutable Nix store source");
    }
    Ok(source)
}
type Region = (String, u64, u64, u64, u64, u64, u64);

fn export_region_counts(
    runner: &dyn CommandRunner,
    llvm_cov: &Path,
    wasm: &Path,
    profile: &Path,
    compiled: &str,
    source: &Path,
    label: &str,
) -> Result<BTreeMap<Region, u64>> {
    let mut export = Command::new(llvm_cov);
    export
        .arg("export")
        .arg(wasm)
        .arg(format!("-instr-profile={}", profile.display()))
        .arg(format!("-path-equivalence={compiled},{}", source.display()));
    let output = runner.run(&mut export)?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        bail!(
            "llvm-cov export for {label} exited with {}: {stderr}",
            output.status
        );
    }
    region_counts(&output.stdout)
        .with_context(|| format!("parsing llvm-cov export for {label}; stderr: {stderr}"))
}

fn region_counts(export: &[u8]) -> Result<BTreeMap<Region, u64>> {
    let export: Value = serde_json::from_slice(export)?;
    let mut counts = BTreeMap::new();
    let data = export
        .get("data")
        .and_then(Value::as_array)
        .context("llvm-cov export lacks data")?;
    for unit in data {
        let functions = unit
            .get("functions")
            .and_then(Value::as_array)
            .context("llvm-cov export data lacks functions")?;
        for function in functions {
            let filenames = function
                .get("filenames")
                .and_then(Value::as_array)
                .context("llvm-cov export function lacks filenames")?;
            let regions = function
                .get("regions")
                .and_then(Value::as_array)
                .context("llvm-cov export function lacks regions")?;
            for region in regions {
                let region = region
                    .as_array()
                    .filter(|region| region.len() >= 8)
                    .context("llvm-cov export region is malformed")?;
                let integer = |index: usize, field: &str| {
                    region[index].as_u64().with_context(|| {
                        format!("llvm-cov export {field} is not an unsigned integer")
                    })
                };
                let file_id = integer(5, "region file ID")?;
                let filename = filenames
                    .get(usize::try_from(file_id).context("llvm-cov export file ID is too large")?)
                    .and_then(Value::as_str)
                    .context("llvm-cov export region file ID is out of bounds")?;
                let key = (
                    filename.to_owned(),
                    integer(0, "region start line")?,
                    integer(1, "region start column")?,
                    integer(2, "region end line")?,
                    integer(3, "region end column")?,
                    integer(6, "region expanded-file ID")?,
                    integer(7, "region kind")?,
                );
                let count = integer(4, "region count")?;
                if counts.insert(key.clone(), count).is_some() {
                    bail!(
                        "llvm-cov export repeats region {}:{}:{}-{}:{} (expanded file {}, kind {})",
                        key.0,
                        key.1,
                        key.2,
                        key.3,
                        key.4,
                        key.5,
                        key.6,
                    );
                }
            }
        }
    }
    if counts.is_empty() {
        bail!("llvm-cov export has no regions");
    }
    Ok(counts)
}

fn prove_count_union(
    chromium: &BTreeMap<Region, u64>,
    firefox: &BTreeMap<Region, u64>,
    merged: &BTreeMap<Region, u64>,
) -> Result<()> {
    let regions: BTreeSet<_> = chromium
        .keys()
        .chain(firefox.keys())
        .chain(merged.keys())
        .cloned()
        .collect();
    let mut overlapping_execution = false;
    for region in regions {
        let expected = chromium.get(&region).copied().unwrap_or_default()
            + firefox.get(&region).copied().unwrap_or_default();
        if expected != merged.get(&region).copied().unwrap_or_default() {
            bail!(
                "merged region count for original Rust source {}:{}:{}-{}:{} (expanded file {}, kind {}) is not the browser-count sum",
                region.0,
                region.1,
                region.2,
                region.3,
                region.4,
                region.5,
                region.6,
            );
        }
        overlapping_execution |= chromium.get(&region).copied().unwrap_or_default() > 0
            && firefox.get(&region).copied().unwrap_or_default() > 0;
    }
    if !overlapping_execution {
        bail!("merged export does not prove a region executed by both browsers");
    }
    Ok(())
}
fn realize_csr(runner: &dyn CommandRunner) -> Result<PathBuf> {
    let output = Path::new(".xtask/gcroots/wasm-coverage-csr");
    realize_nix(
        runner,
        NixRealization {
            package: "wasm-coverage-csr",
            output,
            impure_environment: None,
        },
    )?;
    Ok(output.to_owned())
}

fn write_aggregate(aggregate: &Aggregate) -> Result<()> {
    let path = Path::new(AGGREGATE);
    fs::create_dir_all(path.parent().expect("aggregate has a parent"))?;
    let mut file = fs::File::create(path)?;
    file.write_all(serde_json::to_string_pretty(aggregate)?.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

const MEASUREMENT_ROOT: &str = ".xtask/wasm-coverage/measurement";
const MEASUREMENT_VERSION: &str = "wasm-coverage-measurement-v1";
const MODES: [&str; 2] = ["baseline", "instrumented"];

/// One retained VM result. The producer, rather than the host clock, records the
/// focused browser-flow duration and the served module's uncompressed byte count.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MeasurementRun {
    pub version: String,
    pub browser: String,
    pub mode: String,
    pub cache_buster: String,
    #[serde(default)]
    pub nix_realization: String,
    #[serde(default)]
    pub artifact_root: String,
    pub focused_flow_milliseconds: u64,
    pub served_wasm_path: String,
    pub served_wasm_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MeasurementSummary {
    pub browser: String,
    pub mode: String,
    pub median_milliseconds: u64,
    pub range_milliseconds: [u64; 2],
    pub median_wasm_bytes: u64,
    pub range_wasm_bytes: [u64; 2],
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MeasurementManifest {
    pub version: String,
    pub quiescent_window: String,
    pub runs: Vec<MeasurementRun>,
    pub summaries: Vec<MeasurementSummary>,
}

/// Execute the fixed experiment. Each retained realization warms its focused flow
/// before timing it; retained runs alternate baseline then instrumented, five times
/// for each browser.
pub fn measure(quiescent_window: &str) -> Result<MeasurementManifest> {
    measure_with(&ProcessRunner, quiescent_window)
}

fn measure_with(runner: &dyn CommandRunner, quiescent_window: &str) -> Result<MeasurementManifest> {
    if quiescent_window.trim().is_empty() {
        bail!("measurement requires a nonempty --quiescent-window acknowledgement");
    }
    let functional = probe_with(runner).context("establishing current functional evidence")?;
    require_functional_evidence(&functional)?;
    let root = Path::new(MEASUREMENT_ROOT);
    if root.exists() {
        fs::remove_dir_all(root).context("clearing stale measurement evidence")?;
    }
    fs::create_dir_all(root)?;
    let invocation_nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("reading measurement invocation clock")?
        .as_nanos();
    let mut runs = Vec::new();
    for browser in BROWSERS {
        for pair in 0..5 {
            for mode in MODES {
                let cache_buster = cache_buster(invocation_nonce, browser, mode, pair);
                let evidence = root
                    .join("runs")
                    .join(browser)
                    .join(format!("{pair}-{mode}"));
                let run = realize_measurement(runner, browser, mode, &cache_buster, &evidence)?;
                runs.push(run);
            }
        }
    }
    let manifest = MeasurementManifest {
        version: MEASUREMENT_VERSION.to_owned(),
        quiescent_window: quiescent_window.to_owned(),
        summaries: measurement_summaries(&runs)?,
        runs,
    };
    validate_measurement(&manifest)?;
    validate_retained_measurement(&manifest, root)?;
    write_measurement_manifest(&manifest)?;
    Ok(manifest)
}

fn cache_buster(invocation_nonce: u128, browser: &str, mode: &str, ordinal: usize) -> String {
    // Invocation-owned entropy prevents a binary-cache hit and is retained verbatim.
    format!(
        "{invocation_nonce}-{}-{browser}-{mode}-measured-{ordinal}",
        std::process::id()
    )
}

fn realize_measurement(
    runner: &dyn CommandRunner,
    browser: &str,
    mode: &str,
    cache_buster: &str,
    evidence: &Path,
) -> Result<MeasurementRun> {
    if cache_buster.trim().is_empty() {
        bail!("measurement cache-buster must not be empty");
    }
    let package = format!("wasm-coverage-measure-{browser}-{mode}");
    let output = Path::new(".xtask/gcroots").join(format!("{package}-{cache_buster}"));
    realize_nix(
        runner,
        NixRealization {
            package: &package,
            output: &output,
            impure_environment: Some(("JAUNDER_WASM_COVERAGE_CACHE_BUSTER", cache_buster)),
        },
    )?;
    unpack_archive(
        &output.join(format!("{package}.tar.gz")),
        evidence,
        ArchiveLayout::SingleFile("measurement.json"),
    )?;
    let realization = fs::canonicalize(&output)
        .context("resolving fresh Nix measurement realization")?
        .display()
        .to_string();
    fs::write(
        evidence.join("nix-realization.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::json!({
                "cache_buster": cache_buster,
                "nix_realization": realization,
            }))?
        ),
    )?;
    let mut run: MeasurementRun = serde_json::from_slice(
        &fs::read(evidence.join("measurement.json")).context("reading measurement result")?,
    )
    .context("parsing measurement result")?;
    run.nix_realization = realization;
    if run.browser != browser || run.mode != mode || run.cache_buster != cache_buster {
        bail!("measurement producer identity or cache-buster does not match invocation");
    }
    run.artifact_root = evidence
        .strip_prefix(MEASUREMENT_ROOT)
        .context("measurement evidence is outside the measurement root")?
        .to_string_lossy()
        .to_string();
    validate_measurement_run(&run)?;
    Ok(run)
}

fn require_functional_evidence(aggregate: &Aggregate) -> Result<()> {
    if aggregate.verdict != Verdict::Passed {
        bail!("measurement requires current passing functional evidence");
    }
    if !aggregate.blockers.is_empty() || aggregate.merged.is_none() {
        bail!("passing functional evidence is incomplete");
    }
    Ok(())
}

pub fn validate_measurement(manifest: &MeasurementManifest) -> Result<()> {
    if manifest.version != MEASUREMENT_VERSION || manifest.quiescent_window.trim().is_empty() {
        bail!("unknown measurement manifest or missing quiescent-window acknowledgement");
    }
    let mut cache_busters = BTreeSet::new();
    let mut realizations = BTreeSet::new();
    for browser in BROWSERS {
        let browser_runs: Vec<_> = manifest
            .runs
            .iter()
            .filter(|run| run.browser == browser)
            .collect();
        if browser_runs.len() != 10 {
            bail!(
                "{browser}: expected exactly 10 measured runs, got {}",
                browser_runs.len()
            );
        }
        for (index, run) in browser_runs.iter().enumerate() {
            validate_measurement_run(run)?;
            let expected_mode = MODES[index % 2];
            if run.mode != expected_mode {
                bail!(
                    "{browser}: run {index} is {}, expected {expected_mode}",
                    run.mode
                );
            }
            if !cache_busters.insert(run.cache_buster.as_str()) {
                bail!("duplicate measurement cache-buster {:?}", run.cache_buster);
            }
            if !realizations.insert(run.nix_realization.as_str()) {
                bail!("duplicate Nix realization {:?}", run.nix_realization);
            }
        }
    }
    if manifest.runs.len() != 20 {
        bail!("manifest contains runs outside the two exact browser populations");
    }
    if manifest.summaries != measurement_summaries(&manifest.runs)? {
        bail!("measurement summaries do not reconcile with retained runs");
    }
    Ok(())
}

fn validate_measurement_run(run: &MeasurementRun) -> Result<()> {
    if run.version != MEASUREMENT_VERSION
        || !BROWSERS.contains(&run.browser.as_str())
        || !MODES.contains(&run.mode.as_str())
        || run.artifact_root.trim().is_empty()
        || run.nix_realization.trim().is_empty()
        || relative_path(&run.served_wasm_path).is_err()
        || !run.served_wasm_path.ends_with(".wasm")
        || run.focused_flow_milliseconds == 0
        || run.served_wasm_bytes == 0
    {
        bail!("malformed, stale, or incomplete measurement run");
    }
    Ok(())
}

fn validate_retained_measurement(manifest: &MeasurementManifest, root: &Path) -> Result<()> {
    let runs_root = root.join("runs");
    let mut expected = BTreeSet::new();
    for run in &manifest.runs {
        let relative = relative_path(&run.artifact_root)?;
        if !relative.starts_with("runs") {
            bail!("measurement artifact root is outside retained runs");
        }
        let evidence = root.join(relative);
        let producer: MeasurementRun = serde_json::from_slice(
            &fs::read(evidence.join("measurement.json"))
                .context("reading retained measurement payload")?,
        )
        .context("parsing retained measurement payload")?;
        if producer.version != run.version
            || producer.browser != run.browser
            || producer.mode != run.mode
            || producer.cache_buster != run.cache_buster
            || producer.focused_flow_milliseconds != run.focused_flow_milliseconds
            || producer.served_wasm_path != run.served_wasm_path
            || producer.served_wasm_bytes != run.served_wasm_bytes
        {
            bail!("retained measurement payload does not match manifest");
        }
        let realization: Value = serde_json::from_slice(
            &fs::read(evidence.join("nix-realization.json"))
                .context("reading retained Nix realization")?,
        )
        .context("parsing retained Nix realization")?;
        if realization.pointer("/cache_buster").and_then(Value::as_str) != Some(&run.cache_buster)
            || realization
                .pointer("/nix_realization")
                .and_then(Value::as_str)
                != Some(&run.nix_realization)
        {
            bail!("retained Nix realization does not match manifest");
        }
        expected.insert(
            relative
                .join("measurement.json")
                .strip_prefix("runs")?
                .to_owned(),
        );
        expected.insert(
            relative
                .join("nix-realization.json")
                .strip_prefix("runs")?
                .to_owned(),
        );
    }
    if regular_files(&runs_root)? != expected {
        bail!("retained measurement evidence has missing, tampered, or unreferenced files");
    }
    Ok(())
}

fn measurement_summaries(runs: &[MeasurementRun]) -> Result<Vec<MeasurementSummary>> {
    let mut summaries = Vec::new();
    for browser in BROWSERS {
        for mode in MODES {
            let selected: Vec<_> = runs
                .iter()
                .filter(|run| run.browser == browser && run.mode == mode)
                .collect();
            if selected.len() != 5 {
                bail!(
                    "{browser}/{mode}: expected five runs, got {}",
                    selected.len()
                );
            }
            let timings: Vec<_> = selected
                .iter()
                .map(|run| run.focused_flow_milliseconds)
                .collect();
            let bytes: Vec<_> = selected.iter().map(|run| run.served_wasm_bytes).collect();
            summaries.push(MeasurementSummary {
                browser: browser.to_owned(),
                mode: mode.to_owned(),
                median_milliseconds: median(timings.clone())?,
                range_milliseconds: range(timings)?,
                median_wasm_bytes: median(bytes.clone())?,
                range_wasm_bytes: range(bytes)?,
            });
        }
    }
    Ok(summaries)
}

fn median(mut values: Vec<u64>) -> Result<u64> {
    values.sort_unstable();
    values
        .get(values.len() / 2)
        .copied()
        .context("median of empty population")
}
fn range(mut values: Vec<u64>) -> Result<[u64; 2]> {
    values.sort_unstable();
    Ok([
        *values.first().context("range of empty population")?,
        *values.last().context("range of empty population")?,
    ])
}

fn write_measurement_manifest(manifest: &MeasurementManifest) -> Result<()> {
    let path = Path::new(MEASUREMENT_ROOT).join("manifest-v1.json");
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(manifest)?),
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use coverage::wasm::ServedModule;
    use tempfile::tempdir;

    fn status(browser: &str) -> BrowserStatus {
        BrowserStatus {
            version: "v1".into(),
            requested_browser: browser.into(),
            actual_browser: browser.into(),
            csr_structural: Stage {
                outcome: Outcome::Passed,
                blocker: None,
            },
            diagnostic_export: Stage {
                outcome: Outcome::Passed,
                blocker: None,
            },
            source_mapping: Stage {
                outcome: Outcome::Passed,
                blocker: None,
            },
            module_signature: Some("module".into()),
            toolchain_identity: Some(serde_json::json!({"llvm":"pinned"})),
            served_module: None,
            artifacts: BTreeMap::new(),
        }
    }
    fn valid_root(browser: &str) -> tempfile::TempDir {
        let root = tempdir().unwrap();
        let mut value = status(browser);
        for (name, path, bytes) in [
            (
                "module",
                "module/pkg/served-module.wasm",
                b"module".as_slice(),
            ),
            (
                "diagnostics",
                "diagnostics/capture.log",
                b"capture".as_slice(),
            ),
            ("profile", "profiles/browser.profraw", b"profile".as_slice()),
            (
                "profile_data",
                "mapped/browser.profdata",
                b"profdata".as_slice(),
            ),
            (
                "mapped_report",
                "mapped/llvm-cov.txt",
                b"1|1| source".as_slice(),
            ),
        ] {
            let file = root.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, bytes).unwrap();
            value.artifacts.insert(
                name.into(),
                Artifact {
                    path: path.into(),
                    sha256: sha256(&file).unwrap(),
                },
            );
        }
        let module = value.artifacts["module"].clone();
        value.served_module = Some(ServedModule {
            path: "pkg/served-module.wasm".into(),
            sha256: module.sha256,
        });
        fs::write(
            root.path().join("status.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
        root
    }

    #[test]
    fn parses_complete_exact_browser_status_and_rejects_stale_identity() {
        let root = valid_root("chromium");
        assert_eq!(
            validate_browser(root.path(), "chromium")
                .unwrap()
                .requested_browser,
            "chromium"
        );
        assert!(validate_browser(root.path(), "firefox").is_err());
    }

    #[test]
    fn early_playwright_failure_is_validated_but_cannot_merge_or_pass() {
        let root = valid_root("chromium");
        let mut value: BrowserStatus =
            serde_json::from_slice(&fs::read(root.path().join("status.json")).unwrap()).unwrap();
        value.actual_browser = "not-started".into();
        value.csr_structural = Stage {
            outcome: Outcome::Passed,
            blocker: None,
        };
        value.diagnostic_export = Stage {
            outcome: Outcome::Failed,
            blocker: Some("Playwright exited with status 1 before coverage capture".into()),
        };
        value.source_mapping = Stage {
            outcome: Outcome::NotRun,
            blocker: Some("diagnostic export did not run".into()),
        };
        value.module_signature = None;
        value.toolchain_identity = None;
        for name in ["profile", "profile_data", "mapped_report"] {
            let artifact = value.artifacts.remove(name).unwrap();
            fs::remove_file(root.path().join(artifact.path)).unwrap();
        }
        fs::write(
            root.path().join("status.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();

        let status = validate_browser(root.path(), "chromium").unwrap();
        let statuses = BTreeMap::from([("chromium".into(), status)]);
        assert_eq!(derive_verdict(&statuses).0, Verdict::Failed);
        assert!(merge_ineligibility(&statuses).is_some());
    }

    #[test]
    fn unknown_browser_requires_the_wholly_failed_early_path() {
        let root = valid_root("chromium");
        let mut value: BrowserStatus =
            serde_json::from_slice(&fs::read(root.path().join("status.json")).unwrap()).unwrap();
        value.actual_browser = "unknown".into();
        fs::write(
            root.path().join("status.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
        assert!(validate_browser(root.path(), "chromium").is_err());
    }

    #[test]
    fn served_module_identity_must_match_the_retained_module() {
        let root = valid_root("chromium");
        let mut value: BrowserStatus =
            serde_json::from_slice(&fs::read(root.path().join("status.json")).unwrap()).unwrap();
        value.served_module.as_mut().unwrap().path = "pkg/other-module.wasm".into();
        fs::write(
            root.path().join("status.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
        assert!(validate_browser(root.path(), "chromium").is_err());
    }

    #[test]
    fn reconciliation_rejects_tampered_manifested_artifact() {
        let root = valid_root("firefox");
        fs::write(root.path().join("profiles/browser.profraw"), "tampered").unwrap();
        assert!(validate_browser(root.path(), "firefox").is_err());
    }

    #[test]
    fn contradictory_stage_status_is_rejected() {
        let root = valid_root("chromium");
        let mut value: BrowserStatus =
            serde_json::from_slice(&fs::read(root.path().join("status.json")).unwrap()).unwrap();
        value.source_mapping.outcome = Outcome::Passed;
        value.source_mapping.blocker = Some("contradiction".into());
        fs::write(
            root.path().join("status.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
        assert!(validate_browser(root.path(), "chromium").is_err());
    }

    #[test]
    fn nonpassing_stage_requires_an_exact_blocker() {
        let error = validate_stage(
            "source_mapping",
            &Stage {
                outcome: Outcome::NotRun,
                blocker: None,
            },
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "source_mapping did not pass and has no blocker"
        );
    }

    #[test]
    fn verdict_accumulates_both_browser_blockers() {
        let mut chromium = status("chromium");
        chromium.diagnostic_export = Stage {
            outcome: Outcome::Failed,
            blocker: Some("export failed".into()),
        };
        let mut firefox = status("firefox");
        firefox.source_mapping = Stage {
            outcome: Outcome::Failed,
            blocker: Some("mapping failed".into()),
        };
        let statuses = BTreeMap::from([("chromium".into(), chromium), ("firefox".into(), firefox)]);
        let (verdict, blockers) = derive_verdict(&statuses);
        assert_eq!(verdict, Verdict::Failed);
        assert_eq!(blockers.len(), 2);
    }

    #[test]
    fn merge_requires_two_matching_passes() {
        let mut chromium = status("chromium");
        chromium.served_module = Some(ServedModule {
            path: "pkg/served-module.wasm".into(),
            sha256: "a".repeat(64),
        });
        chromium.artifacts.insert(
            "module".into(),
            Artifact {
                path: "module/pkg/served-module.wasm".into(),
                sha256: "a".repeat(64),
            },
        );
        let mut firefox = status("firefox");
        firefox.served_module = chromium.served_module.clone();
        firefox.artifacts.insert(
            "module".into(),
            Artifact {
                path: "module/pkg/served-module.wasm".into(),
                sha256: "a".repeat(64),
            },
        );
        let mut statuses = BTreeMap::from([
            ("chromium".into(), chromium),
            ("firefox".into(), firefox.clone()),
        ]);
        assert!(merge_ineligibility(&statuses).is_none());
        firefox.artifacts.get_mut("module").unwrap().sha256 = "b".repeat(64);
        statuses.insert("firefox".into(), firefox);
        assert_eq!(
            merge_ineligibility(&statuses),
            Some("Chromium and Firefox retained module digests differ")
        );
    }

    #[test]
    fn browser_stage_policy_requires_every_stage_to_pass() {
        let mut browser = status("chromium");
        assert!(all_browser_stages_pass(&browser));
        browser.source_mapping.outcome = Outcome::Failed;
        assert!(!all_browser_stages_pass(&browser));
    }
    #[test]
    fn contained_relative_paths_reject_escape() {
        assert!(relative_path("profiles/browser.profraw").is_ok());
        assert!(relative_path("../profile").is_err());
        assert!(relative_path("/profile").is_err());
    }

    #[test]
    fn reconciliation_rejects_unmanifested_file() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("status.json"), "{}").unwrap();
        fs::write(root.path().join("extra"), "x").unwrap();
        assert!(reconcile_manifest(root.path(), &BTreeMap::new()).is_err());
    }

    #[test]
    fn conditional_references_are_enforced() {
        let mut value = status("chromium");
        for (name, path) in [("module", "module"), ("diagnostics", "diagnostics")] {
            value.artifacts.insert(
                name.into(),
                Artifact {
                    path: path.into(),
                    sha256: "0".repeat(64),
                },
            );
        }
        assert!(validate_conditional_artifacts(&value).is_err());
        for (name, path) in [
            ("profile", "profile"),
            ("profile_data", "profdata"),
            ("mapped_report", "report"),
        ] {
            value.artifacts.insert(
                name.into(),
                Artifact {
                    path: path.into(),
                    sha256: "0".repeat(64),
                },
            );
        }
        assert!(validate_conditional_artifacts(&value).is_ok());
        value.source_mapping.outcome = Outcome::Failed;
        assert!(validate_conditional_artifacts(&value).is_err());
    }

    fn export_fixture(regions: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "data": [{
                "functions": [{
                    "filenames": ["src/lib.rs"],
                    "regions": regions,
                }],
            }],
        }))
        .unwrap()
    }

    #[test]
    fn union_proof_uses_exact_export_regions_without_counter_abbreviation() {
        let chromium = region_counts(&export_fixture(serde_json::json!([
            [10, 2, 10, 12, 1360, 0, 0, 0],
            [12, 1, 12, 5, 0, 0, 0, 0],
        ])))
        .unwrap();
        let firefox = region_counts(&export_fixture(serde_json::json!([
            [10, 2, 10, 12, 1401, 0, 0, 0],
            [12, 1, 12, 5, 7, 0, 0, 0],
        ])))
        .unwrap();
        let merged = region_counts(&export_fixture(serde_json::json!([
            [10, 2, 10, 12, 2761, 0, 0, 0],
            [12, 1, 12, 5, 7, 0, 0, 0],
        ])))
        .unwrap();
        assert!(prove_count_union(&chromium, &firefox, &merged).is_ok());
        assert_eq!(chromium[&("src/lib.rs".into(), 10, 2, 10, 12, 0, 0)], 1360);

        let file_id = region_counts(
            &serde_json::to_vec(&serde_json::json!({
                "data": [{
                    "functions": [{
                        "filenames": ["generated.rs", "src/lib.rs"],
                        "regions": [[20, 1, 20, 8, 9, 1, 0, 0]],
                    }],
                }],
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(file_id[&("src/lib.rs".into(), 20, 1, 20, 8, 0, 0)], 9);
    }

    #[test]
    fn union_proof_rejects_missing_or_mismatched_regions() {
        let chromium = region_counts(&export_fixture(serde_json::json!([[
            10, 2, 10, 12, 2, 0, 0, 0
        ]])))
        .unwrap();
        let firefox = region_counts(&export_fixture(serde_json::json!([[
            10, 2, 10, 12, 3, 0, 0, 0
        ]])))
        .unwrap();
        let merged = region_counts(&export_fixture(serde_json::json!([[
            10, 2, 10, 12, 4, 0, 0, 0
        ]])))
        .unwrap();
        assert!(prove_count_union(&chromium, &firefox, &merged).is_err());
    }

    #[test]
    fn source_reconciliation_requires_a_tagged_absolute_nix_store_source() {
        let mut identity = SourceIdentity {
            source_identity: NixStoreSource {
                kind: "workspace-source".into(),
                value: "/tmp/source".into(),
            },
            compilation_directory: "/build/source".into(),
        };
        assert!(retained_nix_source(&identity).is_err());
        identity.source_identity.kind = "nix-store-source".into();
        assert!(retained_nix_source(&identity).is_err());
        identity.compilation_directory.clear();
        assert!(retained_nix_source(&identity).is_err());
    }

    #[test]
    fn destination_cleanup_ignores_absence_but_retains_other_errors() {
        let root = tempdir().unwrap();
        let missing = root.path().join("missing");
        assert!(clear_destination(&missing, "clearing test destination").is_ok());
        let file = root.path().join("file");
        fs::write(&file, "not a directory").unwrap();
        let error = clear_destination(&file, "clearing test destination").unwrap_err();
        assert!(error.to_string().contains("clearing test destination"));
        assert!(error.to_string().contains(&file.display().to_string()));
    }
    fn measurement_manifest() -> MeasurementManifest {
        let mut runs = Vec::new();
        for browser in BROWSERS {
            for pair in 0..5 {
                for mode in MODES {
                    runs.push(MeasurementRun {
                        version: MEASUREMENT_VERSION.into(),
                        browser: browser.into(),
                        mode: mode.into(),
                        cache_buster: format!("{browser}-{mode}-{pair}"),
                        nix_realization: format!("/nix/store/{browser}-{mode}-{pair}"),
                        artifact_root: format!("runs/{browser}/{pair}-{mode}"),
                        focused_flow_milliseconds: 10 + pair as u64,
                        served_wasm_path: "pkg/served-module.wasm".into(),
                        served_wasm_bytes: 100 + pair as u64,
                    });
                }
            }
        }
        MeasurementManifest {
            version: MEASUREMENT_VERSION.into(),
            quiescent_window: "coordinated".into(),
            summaries: measurement_summaries(&runs).unwrap(),
            runs,
        }
    }

    #[test]
    fn measurement_requires_exact_fresh_alternating_population_and_reconciliation() {
        let manifest = measurement_manifest();
        assert!(validate_measurement(&manifest).is_ok());
        let mut duplicate = manifest.clone();
        duplicate.runs[1].cache_buster = duplicate.runs[0].cache_buster.clone();
        assert!(validate_measurement(&duplicate).is_err());
        let mut stale = manifest.clone();
        stale.runs[0].nix_realization.clear();
        assert!(validate_measurement(&stale).is_err());
        let mut missing = manifest.clone();
        missing.runs.pop();
        let mut duplicate_realization = manifest.clone();
        duplicate_realization.runs[1].nix_realization =
            duplicate_realization.runs[0].nix_realization.clone();
        assert!(validate_measurement(&duplicate_realization).is_err());
        assert!(validate_measurement(&missing).is_err());
        let mut reordered = manifest.clone();
        reordered.runs.swap(0, 1);
        assert!(validate_measurement(&reordered).is_err());
        let mut unreconciled = manifest;
        unreconciled.summaries[0].median_milliseconds = 99;
        assert!(validate_measurement(&unreconciled).is_err());
    }

    #[test]
    fn measurement_statistics_are_deterministic() {
        assert_eq!(median(vec![5, 1, 3, 2, 4]).unwrap(), 3);
        assert_eq!(range(vec![5, 1, 3, 2, 4]).unwrap(), [1, 5]);
    }

    #[test]
    fn functional_evidence_requires_a_clean_merged_pass() {
        let passing = Aggregate {
            version: "v1",
            browsers: BTreeMap::new(),
            verdict: Verdict::Passed,
            blockers: Vec::new(),
            merged: Some(MergedEvidence {
                profile_data: "profile".into(),
                report: "report".into(),
            }),
        };
        assert!(require_functional_evidence(&passing).is_ok());
        let incomplete = Aggregate {
            merged: None,
            ..passing
        };
        assert!(require_functional_evidence(&incomplete).is_err());
    }

    fn retain_measurement(manifest: &MeasurementManifest, root: &Path) {
        for run in &manifest.runs {
            let evidence = root.join(&run.artifact_root);
            fs::create_dir_all(&evidence).unwrap();
            fs::write(
                evidence.join("measurement.json"),
                serde_json::to_vec(run).unwrap(),
            )
            .unwrap();
            fs::write(
                evidence.join("nix-realization.json"),
                serde_json::to_vec(&serde_json::json!({
                    "cache_buster": run.cache_buster,
                    "nix_realization": run.nix_realization,
                }))
                .unwrap(),
            )
            .unwrap();
        }
    }

    #[test]
    fn retained_measurement_rejects_missing_tampered_and_unreferenced_evidence() {
        let manifest = measurement_manifest();
        let root = tempdir().unwrap();
        retain_measurement(&manifest, root.path());
        assert!(validate_retained_measurement(&manifest, root.path()).is_ok());
        fs::remove_file(
            root.path()
                .join(&manifest.runs[0].artifact_root)
                .join("measurement.json"),
        )
        .unwrap();
        assert!(validate_retained_measurement(&manifest, root.path()).is_err());
        retain_measurement(&manifest, root.path());
        fs::write(
            root.path()
                .join(&manifest.runs[0].artifact_root)
                .join("measurement.json"),
            "{}",
        )
        .unwrap();
        assert!(validate_retained_measurement(&manifest, root.path()).is_err());
        retain_measurement(&manifest, root.path());
        fs::write(root.path().join("runs/extra"), "unexpected").unwrap();
        assert!(validate_retained_measurement(&manifest, root.path()).is_err());
    }

    #[test]
    fn measurement_refuses_missing_quiescent_acknowledgement() {
        assert!(measure_with(&ProcessRunner, " \t").is_err());
    }
}
