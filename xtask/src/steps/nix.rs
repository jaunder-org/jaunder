use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::pin::Pin;
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;

use anyhow::{Context, Result, bail};
use processkit::{Outcome, StdioMode};
use tokio::io::AsyncWrite;

use super::process::Process;
use crate::nix_build;
use crate::result::{CommandResult, NixReport, PhaseName, PhaseRecord, StepResult};

/// The flake checks are Linux-only (`optionalAttrs isLinux` in `nix/checks.nix`);
/// the project's CI host is x86_64-linux.
const SYSTEM: &str = "x86_64-linux";

/// Lift and consume the existing producer's fixed artifact set, without building it.
pub(crate) fn consume_elisp_coverage(result: &mut CommandResult) {
    let artifacts = Path::new(".xtask/gcroots/elisp-coverage-producer/elisp-coverage");
    result.push(lift_elisp_coverage_artifacts(
        artifacts,
        Path::new(".xtask/diagnostics/elisp-coverage"),
    ));

    let step = match crate::elisp_coverage::consume(Path::new("."), artifacts) {
        Ok(report) => StepResult::ok("elisp-coverage").detail(format!(
            "covered {} point(s); ignored {} point(s)",
            report.covered_points, report.ignored_points
        )),
        Err(error) => StepResult::fail("elisp-coverage").detail(elisp_coverage_detail(error)),
    };
    result.push(step);
}

fn lift_elisp_coverage_artifacts(source: &Path, destination: &Path) -> StepResult {
    let names = ["lcov.info", "summary.txt", "status.json"];
    if let Err(error) = fs::create_dir_all(destination) {
        return StepResult::fail("elisp-coverage-artifacts")
            .detail(format!("creating {}: {error}", destination.display()));
    }
    let mut failures = Vec::new();
    for name in names {
        let target = destination.join(name);
        if let Err(error) = fs::remove_file(&target)
            && error.kind() != io::ErrorKind::NotFound
        {
            failures.push(format!("{name}: removing prior artifact: {error}"));
            continue;
        }
        if let Err(error) = fs::copy(source.join(name), target) {
            failures.push(format!("{name}: {error}"));
        }
    }
    if failures.is_empty() {
        StepResult::ok("elisp-coverage-artifacts")
            .detail("lifted lcov.info, summary.txt, status.json")
    } else {
        StepResult::fail("elisp-coverage-artifacts").detail(failures.join("; "))
    }
}

fn elisp_coverage_detail(error: crate::elisp_coverage::CoverageError) -> String {
    use crate::elisp_coverage::CoverageError;

    match error {
        CoverageError::Artifact { path, message } | CoverageError::Source { path, message } => {
            format!("{}: {message}", path.display())
        }
        CoverageError::Status { message }
        | CoverageError::Census { message }
        | CoverageError::Lcov { message } => message,
        CoverageError::Verdict { failures } => failures
            .into_iter()
            .map(|failure| format!("{}:{} {}", failure.path, failure.line, failure.message))
            .collect::<Vec<_>>()
            .join("; "),
    }
}

/// Shared emitted-diagnostic transport retained for non-E2E build failures.
struct DiagnosticsCopy {
    /// How many artifacts were lifted.
    copied: usize,
    /// One message per artifact that could NOT be lifted, naming the file and the OS
    /// error. Reported rather than counted: which file went missing is the whole
    /// diagnostic value, and a bare count would say only that something did.
    failures: Vec<String>,
}

/// Copy matching diagnostic artifacts and collect every best-effort failure.
fn copy_e2e_diagnostics_between(src_dir: &Path, dest_dir: &Path) -> DiagnosticsCopy {
    copy_e2e_diagnostics_with_ops(
        src_dir,
        dest_dir,
        |path| fs::remove_file(path),
        |from, to| fs::copy(from, to),
        |path, permissions| fs::set_permissions(path, permissions),
    )
}

/// The post-build validators read only these lifted files. Remove a previous
/// attempt's inputs before copying so a successful VM cannot validate stale
/// evidence when its current attempt fails to provide an input.
fn is_authoritative_e2e_input(name: &str) -> bool {
    (name.starts_with("playwright-report-") && name.ends_with(".json"))
        || (name.starts_with("duration-budget-manifest-") && name.ends_with(".json"))
        || (name.starts_with("e2e-phase-") && name.ends_with(".json"))
        || (name.starts_with("e2e-census-") && name.ends_with(".json"))
        || (name.starts_with("e2e-lane-manifest-") && name.ends_with(".json"))
        || (name.starts_with("producer-input-") && name.ends_with(".json"))
        || (name.starts_with("e2e-census-") && name.ends_with(".json"))
        || (name.starts_with("capture-") && name.ends_with(".tar.gz"))
}

fn is_e2e_diagnostic_artifact(name: &str) -> bool {
    (name.starts_with("jaunder-journal-") && name.ends_with(".log"))
        || (name.starts_with("system-journal-") && name.ends_with(".log"))
        || is_authoritative_e2e_input(name)
        || (name.starts_with("playwright-artifacts-") && name.ends_with(".tar.gz"))
}

/// Marker freshness matters only to attribution, never to lane reconciliation.
fn e2e_diagnostic_lane(name: &str) -> String {
    const PREFIXES: &[(&str, &str)] = &[
        ("duration-budget-manifest-", ".json"),
        ("playwright-report-", ".json"),
        ("e2e-lane-manifest-", ".json"),
        ("producer-input-", ".json"),
        ("e2e-census-", ".json"),
        ("e2e-phase-", ".json"),
        ("playwright-artifacts-", ".tar.gz"),
        ("capture-", ".tar.gz"),
        ("jaunder-journal-", ".log"),
        ("system-journal-", ".log"),
    ];
    PREFIXES
        .iter()
        .find_map(|(prefix, suffix)| {
            name.strip_prefix(prefix)?
                .strip_suffix(suffix)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn e2e_artifact_failure_detail(name: &str, operation: &str, detail: &str) -> String {
    format!(
        "lane {} file {} operation {} failed: {}",
        e2e_diagnostic_lane(name),
        name,
        operation,
        detail
    )
}

fn record_e2e_artifact_failure(
    name: &str,
    operation: &str,
    detail: String,
    failures: &mut Vec<String>,
) {
    let detail = e2e_artifact_failure_detail(name, operation, &detail);
    if name.starts_with("producer-input-") && name.ends_with(".json") {
        let _ = writeln!(
            io::stderr(),
            "xtask: warning: xtask.nix.e2e_provenance: {detail}; lifted marker unavailable, use selected-output attribution only"
        );
    } else {
        failures.push(detail);
    }
}

fn clear_authoritative_e2e_inputs(
    dest_dir: &Path,
    remove: &mut impl FnMut(&Path) -> io::Result<()>,
    failures: &mut Vec<String>,
) {
    let entries = match fs::read_dir(dest_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            failures.push(format!(
                "reading {}: {:?}",
                dest_dir.display(),
                error.kind()
            ));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(format!(
                    "reading entry under {}: {:?}",
                    dest_dir.display(),
                    error.kind()
                ));
                continue;
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_authoritative_e2e_input(name) {
            continue;
        }
        let path = entry.path();
        match remove(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => record_e2e_artifact_failure(
                name,
                "clear-stale-destination",
                format!("removing {}: {:?}", path.display(), error.kind()),
                failures,
            ),
        }
    }
}

fn copy_e2e_diagnostics_with_ops(
    src_dir: &Path,
    dest_dir: &Path,
    mut remove: impl FnMut(&Path) -> io::Result<()>,
    mut copy: impl FnMut(&Path, &Path) -> io::Result<u64>,
    mut set_permissions: impl FnMut(&Path, fs::Permissions) -> io::Result<()>,
) -> DiagnosticsCopy {
    let mut copied = 0;
    let mut failures = Vec::new();
    clear_authoritative_e2e_inputs(dest_dir, &mut remove, &mut failures);
    let entries = match fs::read_dir(src_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return DiagnosticsCopy { copied, failures };
        }
        Err(error) => {
            failures.push(format!("reading {}: {:?}", src_dir.display(), error.kind()));
            return DiagnosticsCopy { copied, failures };
        }
    };
    if let Err(error) = fs::create_dir_all(dest_dir) {
        failures.push(format!(
            "creating {}: {:?}",
            dest_dir.display(),
            error.kind()
        ));
    }
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(format!(
                    "reading entry under {}: {:?}",
                    src_dir.display(),
                    error.kind()
                ));
                continue;
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_e2e_diagnostic_artifact(name) {
            continue;
        }
        let from = entry.path();
        let to = dest_dir.join(name);
        // An authoritative path still present here could not be cleared above.
        // Keep its single lift failure rather than reporting the same bad path
        // again while attempting the copy.
        if is_authoritative_e2e_input(name) && to.is_dir() {
            continue;
        }
        match remove(&to) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                record_e2e_artifact_failure(
                    name,
                    "remove-destination",
                    format!("removing {}: {:?}", to.display(), error.kind()),
                    &mut failures,
                );
                continue;
            }
        }
        match copy(&from, &to) {
            Ok(_) => {
                if let Err(error) = set_permissions(&to, fs::Permissions::from_mode(0o644)) {
                    record_e2e_artifact_failure(
                        name,
                        "set-permissions",
                        format!(
                            "setting permissions on {}: {:?}",
                            to.display(),
                            error.kind()
                        ),
                        &mut failures,
                    );
                }
                copied += 1;
            }
            Err(error) => record_e2e_artifact_failure(
                name,
                "copy",
                format!(
                    "copying {} to {}: {:?}",
                    from.display(),
                    to.display(),
                    error.kind()
                ),
                &mut failures,
            ),
        }
    }
    DiagnosticsCopy { copied, failures }
}

const DIAGNOSTIC_FAILED: u8 = 1;
const PRIMARY_FAILED: u8 = 2;

/// Shared result of the raw stderr tee. The sink is moved into processkit, so
/// the synchronous owner keeps this small atomic handle for post-wait policy.
#[derive(Clone, Default)]
struct BuildCaptureState(Arc<AtomicU8>);

impl BuildCaptureState {
    fn record(&self, failure: u8) {
        self.0.fetch_or(failure, Ordering::Relaxed);
    }

    fn outcome(&self) -> BuildCaptureOutcome {
        let failures = self.0.load(Ordering::Relaxed);
        BuildCaptureOutcome {
            diagnostic_failed: failures & DIAGNOSTIC_FAILED != 0,
            primary_failed: failures & PRIMARY_FAILED != 0,
        }
    }
}

/// Correlate only the selected check request's current local builder. Nix 2.34.9
/// uses actBuild=105 with [drv, machine, 1, 1], and resBuildLogLine=101 on
/// the same activity ID (BuildLog and worker-protocol-connection.cc). This is
/// deliberately not a general Nix event model or a parser for retained logs.
struct CheckBuildTiming {
    target: Option<String>,
    check: &'static str,
    line: Vec<u8>,
    invalid: bool,
    id: Option<u64>,
    active: bool,
    stopped: bool,
    event: Option<PhaseRecord>,
}

impl CheckBuildTiming {
    fn new(target: Option<String>, check: &'static str) -> Self {
        Self {
            target,
            check,
            line: Vec::new(),
            invalid: false,
            id: None,
            active: false,
            stopped: false,
            event: None,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                if !self.invalid {
                    let line = std::mem::take(&mut self.line);
                    self.record(&line);
                }
                self.line.clear();
            } else if self.line.len() < 65_536 {
                self.line.push(byte);
            } else {
                // Bound memory, but never interrupt draining the process pipe.
                self.invalid = true;
            }
        }
    }

    fn record(&mut self, line: &[u8]) {
        let Some(json) = line.strip_prefix(b"@nix ") else {
            return;
        };
        let Ok(record) = serde_json::from_slice::<serde_json::Value>(json) else {
            self.invalid = true;
            return;
        };
        let action = record["action"].as_str();
        if action.is_none() {
            self.invalid = true;
            return;
        }
        if !matches!(action, Some("start" | "result" | "stop")) {
            return;
        }
        let Some(id) = record["id"].as_u64() else {
            self.invalid = true;
            return;
        };
        match action {
            Some("start") => {
                if self.id == Some(id) {
                    self.invalid = true;
                }
                if record["type"].as_u64() != Some(105) {
                    return;
                }
                let fields = &record["fields"];
                if fields[0].as_str() != self.target.as_deref() || self.target.is_none() {
                    return;
                }
                if self.id.is_some()
                    || fields.as_array().map(Vec::len) != Some(4)
                    || fields[1].as_str() != Some("")
                    || fields[2].as_u64() != Some(1)
                    || fields[3].as_u64() != Some(1)
                {
                    self.invalid = true;
                    return;
                }
                self.id = Some(id);
                self.active = true;
            }
            Some("result") if self.id == Some(id) && record["type"].as_u64().is_none() => {
                self.invalid = true;
            }
            Some("result") if record["type"].as_u64() == Some(101) && self.id == Some(id) => {
                let Some(text) = record["fields"][0].as_str() else {
                    self.invalid = true;
                    return;
                };
                if !self.active || record["fields"].as_array().map(Vec::len) != Some(1) {
                    self.invalid = true;
                    return;
                }
                match crate::sh::child_timing_evidence(text.as_bytes(), self.check) {
                    crate::sh::ChildTimingEvidence::NoCandidate => {}
                    crate::sh::ChildTimingEvidence::Invalid => self.invalid = true,
                    crate::sh::ChildTimingEvidence::Valid(phase) => {
                        if self.event.is_some() {
                            self.invalid = true;
                        } else {
                            self.event = Some(phase);
                        }
                    }
                }
            }
            Some("result")
                if record["type"].as_u64() == Some(101)
                    && record["fields"][0]
                        .as_str()
                        .is_some_and(|text| text.contains("devtool-check-timing")) =>
            {
                // An unmatched completion cannot be repaired by a later valid
                // sequence: current-child attribution is then ambiguous.
                self.invalid = true;
            }
            Some("stop") if self.id == Some(id) => {
                if !self.active {
                    self.invalid = true;
                }
                self.active = false;
                self.stopped = true;
            }
            _ => {}
        }
    }

    fn phase(&self) -> PhaseRecord {
        if !self.invalid
            && self.line.is_empty()
            && self.stopped
            && !self.active
            && let Some(event) = &self.event
        {
            let mut phase = event.clone();
            phase.detail = format!(
                "{}: current local Nix builder activity (monotonic child interval)",
                self.check
            );
            return phase;
        }
        PhaseRecord::unavailable(
            PhaseName::GateExecution,
            format!(
                "{}: no single timing event correlated to an unambiguous current local builder",
                self.check
            ),
        )
    }
}

/// Fans every processkit-provided byte chunk to the diagnostic sink and then
/// the primary stderr sink. Sink failures are recorded rather than returned so
/// processkit continues draining the child pipe and driving the other sink.
struct BuildStderrTee<A, B> {
    diagnostic: A,
    primary: B,
    state: BuildCaptureState,
    check_timing: Option<Arc<Mutex<CheckBuildTiming>>>,
}

impl<A, B> BuildStderrTee<A, B> {
    fn new(diagnostic: A, primary: B) -> (Self, BuildCaptureState) {
        let state = BuildCaptureState::default();
        (
            Self {
                diagnostic,
                primary,
                state: state.clone(),
                check_timing: None,
            },
            state,
        )
    }

    fn observe_check(mut self, timing: Arc<Mutex<CheckBuildTiming>>) -> Self {
        self.check_timing = Some(timing);
        self
    }
}

impl<A: Write + Unpin, B: Write + Unpin> AsyncWrite for BuildStderrTee<A, B> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Some(timing) = &self.check_timing
            && let Ok(mut timing) = timing.lock()
        {
            timing.push(buf);
        }
        if self.diagnostic.write_all(buf).is_err() {
            self.state.record(DIAGNOSTIC_FAILED);
        }
        if self.primary.write_all(buf).is_err() {
            self.state.record(PRIMARY_FAILED);
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<io::Result<()>> {
        if self.diagnostic.flush().is_err() {
            self.state.record(DIAGNOSTIC_FAILED);
        }
        if self.primary.flush().is_err() {
            self.state.record(PRIMARY_FAILED);
        }
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

#[derive(Default)]
struct BuildCaptureOutcome {
    diagnostic_failed: bool,
    primary_failed: bool,
}

fn report_build_diagnostic_failure(failed: bool, stderr: &mut impl Write) {
    if failed {
        let _ = writeln!(
            stderr,
            "xtask: warning: xtask.nix.build_diagnostics: ignored failure while capturing nix build diagnostics"
        );
    }
}

/// Lines of `build.log` kept in the fallback excerpt when the log carries no nix
/// `error:` block (an unusual failure). Distinct from [`NIX_ERROR_TAIL_LINES`], which
/// sizes nix's in-block builder tail on the normal path — they happen to share a value.
const EXCERPT_FALLBACK_LINES: usize = 50;

/// `--log-lines` value passed to `nix build`: how many lines of the failing builder's
/// (de-interleaved) tail nix includes in its `error:` summary block, which
/// [`write_failure_excerpt`] then carves out (#145). Independent of
/// [`EXCERPT_FALLBACK_LINES`].
const NIX_ERROR_TAIL_LINES: &str = "50";

/// Carve a scoped failure excerpt from a captured `nix build -L` log. Nix's own error
/// summary is a self-contained block at column 0 (`error: …` through EOF) that names the
/// failing derivation and includes its *de-interleaved* `Last N log lines` tail — exactly
/// the scoped content we want, no drv/prefix parsing needed (builder output streams
/// prefixed `<name>> …`, so it never matches at column 0). If there is no such block,
/// fall back to the log's last [`EXCERPT_FALLBACK_LINES`] lines behind a marker so the
/// excerpt is never empty.
fn failure_excerpt(build_log: &str) -> String {
    let lines: Vec<&str> = build_log.lines().collect();
    if let Some(i) = lines.iter().position(|l| l.starts_with("error:")) {
        return lines[i..].join("\n");
    }
    let start = lines.len().saturating_sub(EXCERPT_FALLBACK_LINES);
    // Interpolate the const so the marker text can't silently desync from the slice.
    let mut out =
        format!("=== no `error:` block in build log; last {EXCERPT_FALLBACK_LINES} lines: ===\n");
    out.push_str(&lines[start..].join("\n"));
    out
}

/// On a failed check, write the scoped [`failure_excerpt`] beside a complete
/// captured build log. The caller aggregates any failure with its other
/// diagnostic losses.
fn write_failure_excerpt(log_path: &str) -> io::Result<String> {
    write_failure_excerpt_with(
        log_path,
        |path| fs::read_to_string(path),
        |path, body| fs::write(path, body),
    )
}

fn write_failure_excerpt_with(
    log_path: &str,
    read_log: impl FnOnce(&str) -> io::Result<String>,
    write_excerpt: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
) -> io::Result<String> {
    let log = read_log(log_path)?;
    let dir = Path::new(log_path).parent().unwrap_or(Path::new("."));
    let excerpt_path = dir.join("failure-excerpt.log");
    let body = format!("{}\n", failure_excerpt(&log));
    write_excerpt(&excerpt_path, body.as_bytes())?;
    Ok(excerpt_path.to_string_lossy().into_owned())
}

fn prepare_build_dirs_with(
    create_gcroots: impl FnOnce() -> io::Result<()>,
    create_diagnostics: impl FnOnce() -> io::Result<()>,
) -> io::Result<bool> {
    create_gcroots()?;
    Ok(create_diagnostics().is_err())
}

/// The failure `detail` for a Nix check. Diagnostic paths are included only
/// when the build log was captured completely.
fn failure_detail(
    installable: &str,
    status: &(impl Display + ?Sized),
    excerpt_path: Option<&str>,
    log_path: Option<&str>,
) -> String {
    match (excerpt_path, log_path) {
        (Some(excerpt), Some(log)) => format!(
            "nix build {installable} exited with {status}; scoped excerpt (read first): {excerpt}; full build log: {log}"
        ),
        (None, Some(log)) => {
            format!("nix build {installable} exited with {status}; full build log: {log}")
        }
        _ => format!("nix build {installable} exited with {status}"),
    }
}

struct FailedBuildDiagnostics<'a> {
    step_name: &'a str,
    installable: &'a str,
    status: &'a dyn Display,
    log_path: &'a str,
    capture_failed: bool,
}

fn failed_build_after_diagnostics_with(
    build: FailedBuildDiagnostics<'_>,
    write_excerpt: impl FnOnce() -> io::Result<String>,
    rescue: impl FnOnce() -> bool,
    stderr: &mut impl Write,
) -> StepResult {
    let FailedBuildDiagnostics {
        step_name,
        installable,
        status,
        log_path,
        capture_failed,
    } = build;
    let reliable_log_path = (!capture_failed).then_some(log_path);
    let mut diagnostic_failed = capture_failed;
    let excerpt = if reliable_log_path.is_some() {
        match write_excerpt() {
            Ok(path) => Some(path),
            Err(_) => {
                diagnostic_failed = true;
                None
            }
        }
    } else {
        None
    };
    // Preserve the primary build failure while still recovering any VM-owned
    // diagnostics and phase sidecars from retained outputs.
    diagnostic_failed |= rescue();
    report_build_diagnostic_failure(diagnostic_failed, stderr);
    StepResult::fail(step_name).detail(failure_detail(
        installable,
        status,
        excerpt.as_deref(),
        reliable_log_path,
    ))
}
struct BuildCompletion<'a> {
    step_name: &'a str,
    installable: &'a str,
    log_path: &'a str,
    diagnostic_failed: bool,
    capture: BuildCaptureOutcome,
    outcome: anyhow::Result<Outcome>,
}

fn finish_build_with(
    build: BuildCompletion<'_>,
    write_excerpt: impl FnOnce() -> io::Result<String>,
    rescue: impl FnOnce() -> bool,
    nix_report: impl FnOnce() -> NixReport,
    stderr: &mut impl Write,
) -> StepResult {
    let BuildCompletion {
        step_name,
        installable,
        log_path,
        mut diagnostic_failed,
        capture,
        outcome,
    } = build;
    diagnostic_failed |= capture.diagnostic_failed;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            report_build_diagnostic_failure(diagnostic_failed, stderr);
            return StepResult::fail(step_name).detail(error.to_string());
        }
    };
    if capture.primary_failed {
        report_build_diagnostic_failure(diagnostic_failed, stderr);
        return StepResult::fail(step_name).detail("failed to stream nix build stderr");
    }
    if outcome.code() == Some(0) {
        report_build_diagnostic_failure(diagnostic_failed, stderr);
        let nix_report = nix_report();
        return StepResult::ok(step_name)
            .phases(nix_report.phase_records())
            .nix(nix_report);
    }
    let Some(status) = build_status(outcome) else {
        report_build_diagnostic_failure(diagnostic_failed, stderr);
        return StepResult::fail(step_name)
            .detail(format!("nix build {installable} ended with {outcome:?}"));
    };
    failed_build_after_diagnostics_with(
        FailedBuildDiagnostics {
            step_name,
            installable,
            status: &status,
            log_path,
            capture_failed: diagnostic_failed,
        },
        write_excerpt,
        rescue,
        stderr,
    )
}

/// `nix build -L --keep-failed --accept-flake-config --out-link .xtask/gcroots/<check> .#checks.<system>.<check>`,
/// fanning the `-L` build log to both the live terminal and
/// `.xtask/diagnostics/<check>/build.log` (gitignored; uploaded by the owning
/// core-validation, coverage-validation, or e2e diagnostics artifact in ci.yml).
/// On failure a completely captured log is named in the `StepResult`;
/// partial or unavailable diagnostic paths are omitted.
/// --accept-flake-config honors the jaunder-org cachix substituter for the
/// untrusted local user; --out-link makes the closure a GC root.
fn build_check_with_flags(step_name: &str, check: &str, flags: &[&str]) -> StepResult {
    let step = build_installable_with_flags(
        step_name,
        check,
        &format!(".#checks.{SYSTEM}.{check}"),
        flags,
    );
    with_check_execution_evidence(check, step)
}

fn timing_check(check: &str) -> Option<&'static str> {
    match check {
        "static-ert" => Some("ert"),
        "static-typescript" => Some("tsc"),
        _ => None,
    }
}

fn with_check_execution_evidence(check: &str, mut step: StepResult) -> StepResult {
    if let Some(child) = timing_check(check)
        && !step
            .phases
            .iter()
            .any(|p| p.name == PhaseName::GateExecution)
    {
        step.phases.push(CheckBuildTiming::new(None, child).phase());
    }
    step
}

/// The hook's ordinary Git-flake request must not update or write the lock file.
pub(crate) fn shared_static_docs_check() -> StepResult {
    build_check_with_flags(
        "nix-static-docs",
        "static-docs",
        &["--no-update-lock-file", "--no-write-lock-file"],
    )
}

fn build_installable_with_flags(
    step_name: &str,
    check: &str,
    installable: &str,
    extra_flags: &[&str],
) -> StepResult {
    let start = std::time::Instant::now();
    let mut diagnostic_failed = match prepare_build_dirs_with(
        || fs::create_dir_all(".xtask/gcroots"),
        || fs::create_dir_all(format!(".xtask/diagnostics/{check}")),
    ) {
        Ok(failed) => failed,
        Err(error) => {
            return StepResult::fail(step_name)
                .detail(format!("creating .xtask/gcroots: {error}"))
                .with_duration(start.elapsed());
        }
    };
    let out_link = format!(".xtask/gcroots/{check}");
    let log_dir = format!(".xtask/diagnostics/{check}");
    let before = if extra_flags.is_empty() {
        nix_build::observe(installable)
    } else {
        nix_build::observe_readonly(installable)
    };
    let log_path = format!("{log_dir}/build.log");
    let diagnostic: Box<dyn Write + Send> = match File::create(&log_path) {
        Ok(file) => Box::new(file),
        Err(_) => {
            diagnostic_failed = true;
            Box::new(io::sink())
        }
    };
    let (stderr_tee, capture_state) = BuildStderrTee::new(diagnostic, io::stderr());
    let check_timing = timing_check(check).map(|child| {
        Arc::new(Mutex::new(CheckBuildTiming::new(
            before.derivation.clone(),
            child,
        )))
    });
    let stderr_tee = match &check_timing {
        Some(timing) => stderr_tee.observe_check(timing.clone()),
        None => stderr_tee,
    };
    let log_flags: &[&str] = if check_timing.is_some() {
        &["--log-format", "internal-json"]
    } else {
        &[]
    };
    let process = match Process::start(
        processkit::Command::new("nix")
            .args([
                "build",
                "-L",
                "--keep-failed",
                "--log-lines",
                NIX_ERROR_TAIL_LINES,
                "--accept-flake-config",
                "--out-link",
                &out_link,
            ])
            .args(log_flags)
            .args(extra_flags)
            .arg(installable)
            .inherit_stdin()
            .stdout(StdioMode::Inherit)
            .stderr(StdioMode::Piped)
            .stderr_raw_tee(stderr_tee),
    ) {
        Ok(process) => process,
        Err(error) => {
            diagnostic_failed |= capture_state.outcome().diagnostic_failed;
            report_build_diagnostic_failure(diagnostic_failed, &mut io::stderr());
            return StepResult::fail(step_name)
                .detail(error.to_string())
                .with_duration(start.elapsed());
        }
    };
    let outcome = process.wait();
    let timing_reliable = outcome.is_ok();
    let mut step = finish_build_with(
        BuildCompletion {
            step_name,
            installable,
            log_path: &log_path,
            diagnostic_failed,
            capture: capture_state.outcome(),
            outcome,
        },
        || write_failure_excerpt(&log_path),
        || rescue_emitted_diagnostics(check, &Path::new(".xtask/diagnostics").join(check)),
        || {
            if extra_flags.is_empty() {
                before.finish(installable)
            } else {
                before.finish_readonly(installable)
            }
        },
        &mut io::stderr(),
    )
    .with_duration(start.elapsed());
    if let Some(timing) = check_timing {
        step.phases.push(
            timing
                .lock()
                .ok()
                .filter(|_| timing_reliable)
                .map(|t| t.phase())
                .unwrap_or_else(|| {
                    CheckBuildTiming::new(None, timing_check(check).unwrap()).phase()
                }),
        );
    }
    step
}

enum BuildStatus {
    Status(ExitStatus),
    UnknownSignal,
}

impl Display for BuildStatus {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(status) => status.fmt(formatter),
            Self::UnknownSignal => formatter.write_str("signal: unknown"),
        }
    }
}

fn build_status(outcome: Outcome) -> Option<BuildStatus> {
    if let Some(code) = outcome.code() {
        return Some(BuildStatus::Status(ExitStatus::from_raw(code << 8)));
    }
    if let Some(signal) = outcome.signal() {
        return Some(BuildStatus::Status(ExitStatus::from_raw(signal)));
    }
    matches!(outcome, Outcome::Signalled(None)).then_some(BuildStatus::UnknownSignal)
}
/// Run `nix eval --raw --accept-flake-config <installable>`, optionally in a
/// supplied flake directory.
fn nix_eval_raw(dir: Option<&Path>, installable: &str) -> Result<String> {
    let mut cmd = Command::new("nix");
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let out = cmd
        .args(["eval", "--raw", "--accept-flake-config", installable])
        .output()
        .with_context(|| format!("spawning `nix eval {installable}`"))?;
    if !out.status.success() {
        bail!(
            "`nix eval {installable}` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let path = String::from_utf8(out.stdout)
        .with_context(|| format!("`nix eval {installable}` output was not UTF-8"))?
        .trim()
        .to_owned();
    if path.is_empty() {
        bail!("`nix eval {installable}` returned an empty path");
    }
    Ok(path)
}

/// Evaluate the probe-only identity of the filtered coverage source.
pub(crate) fn eval_coverage_source_probe_drvpath(flake_dir: &Path) -> Result<String> {
    nix_eval_raw(
        Some(flake_dir),
        &format!(".#checks.{SYSTEM}.coverage-source-probe.drvPath"),
    )
}

/// Realize the coverage producer and return its output directory.
pub(crate) fn build_coverage_out_path(flake_dir: &Path) -> Result<String> {
    let installable = format!(".#checks.{SYSTEM}.coverage");
    let out = Command::new("nix")
        .current_dir(flake_dir)
        .args([
            "build",
            "--no-link",
            "--print-out-paths",
            "--accept-flake-config",
            &installable,
        ])
        .output()
        .with_context(|| format!("spawning `nix build {installable}`"))?;
    if !out.status.success() {
        bail!(
            "`nix build {installable}` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let path = String::from_utf8(out.stdout)
        .with_context(|| format!("`nix build {installable}` output was not UTF-8"))?
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    if path.is_empty() {
        bail!("`nix build {installable}` returned an empty output path");
    }
    Ok(path)
}

/// Named derivation identities guarded by `nix probe-source`.
pub(crate) struct SourceProbeDrvPaths {
    pub(crate) static_docs: String,
    pub(crate) static_code: String,
    pub(crate) static_elisp: String,
    pub(crate) static_e2e_census: String,
    pub(crate) site: String,
    pub(crate) wasm_tests: String,
    pub(crate) xtask: String,
    pub(crate) devtool: String,
    pub(crate) tooling_dependencies: String,
    pub(crate) csr_debug_bundle: String,
}

/// Evaluate the derivation identities guarded by `nix probe-source`.
/// This deliberately performs only serial `nix eval` calls: the probe mutates
/// one isolated worktree between arms, so concurrent evaluation would obscure
/// which tracked perturbation produced a failed identity.
pub(crate) fn eval_source_probe_drvpaths(flake_dir: &Path) -> Result<SourceProbeDrvPaths> {
    Ok(SourceProbeDrvPaths {
        static_docs: nix_eval_raw(Some(flake_dir), ".#checks.x86_64-linux.static-docs.drvPath")?,
        static_code: nix_eval_raw(Some(flake_dir), ".#checks.x86_64-linux.static-code.drvPath")?,
        static_elisp: nix_eval_raw(
            Some(flake_dir),
            ".#checks.x86_64-linux.static-elisp.drvPath",
        )?,
        static_e2e_census: nix_eval_raw(
            Some(flake_dir),
            ".#checks.x86_64-linux.static-e2e-census.drvPath",
        )?,
        site: nix_eval_raw(Some(flake_dir), ".#site.drvPath")?,
        wasm_tests: nix_eval_raw(Some(flake_dir), ".#checks.x86_64-linux.wasm-tests.drvPath")?,
        xtask: nix_eval_raw(Some(flake_dir), ".#xtask.drvPath")?,
        devtool: nix_eval_raw(Some(flake_dir), ".#devtool.drvPath")?,
        tooling_dependencies: nix_eval_raw(
            Some(flake_dir),
            ".#devtool.toolingDependencies.drvPath",
        )?,
        csr_debug_bundle: nix_eval_raw(Some(flake_dir), ".#csrDebugBundle.drvPath")?,
    })
}

/// On a failed build, best-effort copy diagnostics from retained outputs.
/// Returns whether any secondary recovery step failed so the build owner can
/// aggregate one warning without changing the primary failure.
fn rescue_emitted_diagnostics(check: &str, destination: &Path) -> bool {
    // Preserve the non-E2E emitted-diagnostic recovery route. E2E kept-output
    // recovery now belongs exclusively to the Just producer helper.
    let mut failed = fs::create_dir_all(destination).is_err();
    let prefix = format!("nix-build-jaunder-{check}");
    let Ok(entries) = fs::read_dir("/tmp") else {
        return true;
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                failed = true;
                continue;
            }
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let source = entry.path().join("emit-out/diagnostics");
        if source.is_dir() {
            let outcome = copy_e2e_diagnostics_between(&source, destination);
            if outcome.copied > 0 {
                eprintln!(
                    "xtask: recovered {} emitted diagnostics for {check}",
                    outcome.copied
                );
            }
            failed |= !outcome.failures.is_empty();
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::io::{self, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::rc::Rc;
    use std::sync::Arc;

    use processkit::{Command, Outcome, StdioMode};
    use tokio::io::AsyncWriteExt;
    use tokio::runtime::Builder;

    use super::{
        BuildCaptureOutcome, BuildCompletion, BuildStderrTee, FailedBuildDiagnostics, Process,
        StepResult, failed_build_after_diagnostics_with, finish_build_with,
        lift_elisp_coverage_artifacts, prepare_build_dirs_with, report_build_diagnostic_failure,
    };
    use crate::result::{NixRealization, NixReport, PhaseOutcome};

    fn injected_nix_report(realization: NixRealization) -> NixReport {
        NixReport {
            installable: ".#checks.x86_64-linux.check".to_owned(),
            derivation: Some("/nix/store/check.drv".to_owned()),
            realization,
        }
    }

    #[test]
    fn elisp_coverage_artifact_lift_replaces_prior_read_only_outputs() {
        // Nix outputs are read-only; repeated validate runs must replace the
        // prior lifted diagnostics rather than failing on their mode bits.
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        for name in ["lcov.info", "summary.txt", "status.json"] {
            std::fs::write(source.path().join(name), format!("fresh-{name}")).unwrap();
            let existing = destination.path().join(name);
            std::fs::write(&existing, "stale").unwrap();
            let mut permissions = std::fs::metadata(&existing).unwrap().permissions();
            permissions.set_mode(0o444);
            std::fs::set_permissions(&existing, permissions).unwrap();
        }

        let step = lift_elisp_coverage_artifacts(source.path(), destination.path());

        assert!(step.ok, "{:?}", step.detail);
        for name in ["lcov.info", "summary.txt", "status.json"] {
            assert_eq!(
                std::fs::read_to_string(destination.path().join(name)).unwrap(),
                format!("fresh-{name}")
            );
        }
    }

    const ERT_DRV: &str = "/nix/store/fixture-static-ert.drv";
    const ERT_EVENT: &str = r#"{"event":"devtool-check-timing","schema_version":1,"check":"ert","duration_ms":31,"outcome":"success"}"#;

    fn ert_live_stream(marker: &str) -> String {
        format!(
            "@nix {}\n@nix {}\n@nix {}\n",
            serde_json::json!({"action":"start","id":41,"type":105,"fields":[ERT_DRV,"",1,1]}),
            serde_json::json!({"action":"result","id":41,"type":101,"fields":[format!("Emacs stdout without newline{marker}")]}),
            serde_json::json!({"action":"stop","id":41})
        )
    }

    #[test]
    fn ert_live_builder_success_and_failure_require_complete_activity() {
        for (marker, outcome) in [
            (ERT_EVENT.to_string(), PhaseOutcome::Success),
            (
                ERT_EVENT.replace("success", "failure"),
                PhaseOutcome::Failed,
            ),
        ] {
            let mut timing = super::CheckBuildTiming::new(Some(ERT_DRV.into()), "ert");
            // Arbitrary processkit chunks must not change line framing.
            for bytes in ert_live_stream(&marker).as_bytes().chunks(3) {
                timing.push(bytes);
            }
            let phase = timing.phase();
            assert_eq!(phase.duration_ms, Some(31));
            assert_eq!(phase.outcome, outcome);
        }
    }

    #[test]
    fn ert_live_builder_rejects_historical_missing_malformed_and_ambiguous_evidence() {
        let valid = ert_live_stream(ERT_EVENT);
        let lines: Vec<_> = valid.split_inclusive('\n').collect();
        for stream in [
            String::new(), // Ordinary valid local-store reuse: no builder events.
            format!("historical build.log: {ERT_EVENT}\n"),
            lines[1].into(),
            format!("@nix broken-json\n{valid}"),
            ert_live_stream(&ERT_EVENT.replace("schema_version\":1", "schema_version\":2")),
            ert_live_stream(&format!(
                "{ERT_EVENT}\n{}",
                ERT_EVENT.replace("schema_version\":1", "schema_version\":2")
            )),
            ert_live_stream(&format!(
                "{ERT_EVENT}\n{}",
                ERT_EVENT.replace("duration_ms\":31", "duration_ms\":\"31\"")
            )),
            ert_live_stream(&format!("{ERT_EVENT}\n{}", ERT_EVENT.trim_end_matches('}'))),
            ert_live_stream(&format!("{ERT_EVENT}\n{ERT_EVENT}{}", " ".repeat(256))),
            ert_live_stream(&ERT_EVENT.replace("ert", "tsc")),
            ert_live_stream("no timing marker"),
            format!(
                "@nix {}\n",
                serde_json::json!({"action":"msg", "msg":valid})
            ),
            format!("@nix []\n{valid}"),
            format!("{}{}{}", lines[1], lines[0], lines[2]),
            format!("{}{}{}{}", lines[1], lines[0], lines[1], lines[2]),
            format!("{}{}{}{}", lines[0], lines[1], lines[1], lines[2]),
            format!("{}{}{}{}", lines[0], lines[0], lines[1], lines[2]),
            format!("{}{}{}", lines[0], lines[2], lines[1]),
            valid.replace("\"id\":41,\"type\":101", "\"id\":42,\"type\":101"),
            valid.replace(ERT_DRV, "/nix/store/unrelated.drv"),
            valid.replace("\"\",1,1", "\"remote\",1,1"),
            format!("{}{}", lines[0], lines[1]),
            valid.trim_end().into(),
            format!("{}\n{valid}", "x".repeat(65_537)),
            ert_live_stream(&format!("historical internal-json: {}", lines[1].trim())),
        ] {
            let mut timing = super::CheckBuildTiming::new(Some(ERT_DRV.into()), "ert");
            timing.push(stream.as_bytes());
            assert_eq!(
                timing.phase().outcome,
                PhaseOutcome::Unavailable,
                "{stream}"
            );
        }
        let mut timing = super::CheckBuildTiming::new(None, "ert");
        timing.push(valid.as_bytes());
        assert_eq!(timing.phase().outcome, PhaseOutcome::Unavailable);
    }

    #[test]
    fn ert_live_tee_observes_bytes_without_reading_diagnostic_history() {
        for (input, expected) in [
            (ert_live_stream(ERT_EVENT), Some(31)),
            (
                format!("{}\n{}", "x".repeat(65_537), ert_live_stream(ERT_EVENT)),
                None,
            ),
        ] {
            let timing = std::sync::Arc::new(std::sync::Mutex::new(super::CheckBuildTiming::new(
                Some(ERT_DRV.into()),
                "ert",
            )));
            let (diagnostic, diagnostic_bytes) = recording_sink();
            let (primary, primary_bytes) = recording_sink();
            let (tee, state) = BuildStderrTee::new(diagnostic, primary);
            drive_tee(tee.observe_check(timing.clone()), input.as_bytes());
            assert_eq!(timing.lock().unwrap().phase().duration_ms, expected);
            assert_eq!(*diagnostic_bytes.borrow(), input.as_bytes());
            assert_eq!(*primary_bytes.borrow(), input.as_bytes());
            assert!(!state.outcome().diagnostic_failed);
            assert!(!state.outcome().primary_failed);
        }
    }

    fn tsc_builder_line(action: &str, id: u64, kind: u64, fields: serde_json::Value) -> String {
        format!(
            "@nix {}\n",
            serde_json::json!({"action":action,"id":id,"type":kind,"fields":fields})
        )
    }

    const TSC_EVENT: &str = r#"{"event":"devtool-check-timing","schema_version":1,"check":"tsc","duration_ms":37,"outcome":"success"}"#;

    fn tsc_start() -> String {
        tsc_builder_line(
            "start",
            7,
            105,
            serde_json::json!(["/nix/store/ts.drv", "", 1, 1]),
        )
    }

    #[test]
    fn invalid_timing_candidate_is_sticky_across_current_builder_results() {
        let start = tsc_start();
        let stop = tsc_builder_line("stop", 7, 0, serde_json::json!([]));
        for candidate in [
            TSC_EVENT.replace("\"schema_version\":1", "\"schema_version\":2"),
            r#"{"event":"devtool-check-timing","schema_version":1,"check":"tsc","duration_ms":"#
                .into(),
            format!("{TSC_EVENT}{}", " ".repeat(1024)),
            format!("{TSC_EVENT}{}", " ".repeat(9000)),
            format!("{}{TSC_EVENT}", " ".repeat(9000)),
            format!("{}{TSC_EVENT}", " ".repeat(65536)),
        ] {
            for lines in [
                [TSC_EVENT, candidate.as_str()],
                [candidate.as_str(), TSC_EVENT],
            ] {
                for multiline in [true, false] {
                    let results = if multiline {
                        tsc_builder_line("result", 7, 101, serde_json::json!([lines.join("\n")]))
                    } else {
                        lines
                            .iter()
                            .map(|line| {
                                tsc_builder_line("result", 7, 101, serde_json::json!([line]))
                            })
                            .collect::<String>()
                    };
                    let input = format!("{start}{results}{stop}");
                    let observer = Arc::new(std::sync::Mutex::new(super::CheckBuildTiming::new(
                        Some("/nix/store/ts.drv".into()),
                        "tsc",
                    )));
                    let (diagnostic, diagnostic_bytes) = recording_sink();
                    let (primary, primary_bytes) = recording_sink();
                    let (tee, state) = BuildStderrTee::new(diagnostic, primary);
                    drive_tee(tee.observe_check(observer.clone()), input.as_bytes());
                    assert!(!state.outcome().diagnostic_failed);
                    assert!(!state.outcome().primary_failed);
                    assert_eq!(*diagnostic_bytes.borrow(), input.as_bytes());
                    assert_eq!(*primary_bytes.borrow(), input.as_bytes());
                    let phase = observer.lock().unwrap().phase();
                    assert_eq!(phase.outcome, PhaseOutcome::Unavailable, "{input}");
                    for ok in [true, false] {
                        let step = if ok {
                            StepResult::ok("nix-static-typescript")
                        } else {
                            StepResult::fail("nix-static-typescript")
                        };
                        let step = super::with_check_execution_evidence(
                            "static-typescript",
                            step.detail(&input).phases([phase.clone()]),
                        );
                        assert_eq!(step.ok, ok);
                        assert_eq!(step.detail.as_deref(), Some(input.as_str()));
                    }
                }
            }
        }
        let mut observer = super::CheckBuildTiming::new(Some("/nix/store/ts.drv".into()), "tsc");
        let results = tsc_builder_line(
            "result",
            7,
            101,
            serde_json::json!([format!(
                "ordinary diagnostics\n{TSC_EVENT}\nmore diagnostics"
            )]),
        );
        let diagnostic = tsc_builder_line(
            "result",
            7,
            101,
            serde_json::json!(["ordinary diagnostic ".repeat(900)]),
        );
        observer.push(format!("{start}{diagnostic}{results}{diagnostic}{stop}").as_bytes());
        assert_eq!(observer.phase().duration_ms, Some(37));
    }

    #[test]
    fn typescript_current_builder_activity_supplies_child_execution() {
        let mut observer = super::CheckBuildTiming::new(Some("/nix/store/ts.drv".into()), "tsc");
        observer.push(tsc_start().as_bytes());
        observer
            .push(tsc_builder_line("result", 7, 101, serde_json::json!([TSC_EVENT])).as_bytes());
        observer.push(tsc_builder_line("stop", 7, 0, serde_json::json!([])).as_bytes());
        let phase = observer.phase();
        assert_eq!(phase.duration_ms, Some(37));
        assert_eq!(phase.outcome, PhaseOutcome::Success);
    }

    #[test]
    fn typescript_live_tee_observes_timing_without_changing_diagnostic_bytes() {
        let observer = Arc::new(std::sync::Mutex::new(super::CheckBuildTiming::new(
            Some("/nix/store/ts.drv".into()),
            "tsc",
        )));
        let input = format!(
            "{}{}{}",
            tsc_start(),
            tsc_builder_line("result", 7, 101, serde_json::json!([TSC_EVENT])),
            tsc_builder_line("stop", 7, 0, serde_json::json!([]))
        );
        let (diagnostic, diagnostic_bytes) = recording_sink();
        let (primary, primary_bytes) = recording_sink();
        let (tee, state) = BuildStderrTee::new(diagnostic, primary);
        drive_tee(tee.observe_check(observer.clone()), input.as_bytes());
        assert!(!state.outcome().diagnostic_failed);
        assert!(!state.outcome().primary_failed);
        assert_eq!(*diagnostic_bytes.borrow(), input.as_bytes());
        assert_eq!(*primary_bytes.borrow(), input.as_bytes());
        assert_eq!(observer.lock().unwrap().phase().duration_ms, Some(37));
    }

    #[test]
    fn typescript_builder_timing_requires_live_target_activity_and_valid_child_event() {
        let start = tsc_start();
        let stop = tsc_builder_line("stop", 7, 0, serde_json::json!([]));
        let event = TSC_EVENT.replace("success", "failure");
        let result = tsc_builder_line("result", 7, 101, serde_json::json!([event]));
        let valid = format!("{start}{result}{stop}");
        for (input, derivation) in [
            (String::new(), Some("/nix/store/ts.drv")),
            (result.clone(), Some("/nix/store/ts.drv")),
            (format!("{start}{result}"), Some("/nix/store/ts.drv")),
            (
                valid.replace("\"\",1,1", "\"remote\",1,1"),
                Some("/nix/store/ts.drv"),
            ),
            (
                valid.replace("\"\",1,1", "\"\",0,1"),
                Some("/nix/store/ts.drv"),
            ),
            (format!("{start}{stop}{result}"), Some("/nix/store/ts.drv")),
            (valid.clone(), Some("/nix/store/other.drv")),
            (valid.clone(), None),
            (
                format!("{start}{}{stop}", result.replace("\"id\":7", "\"id\":8")),
                Some("/nix/store/ts.drv"),
            ),
            (
                valid.replace("\"type\":101", "\"type\":100"),
                Some("/nix/store/ts.drv"),
            ),
            (
                format!(
                    "{start}{}{stop}",
                    tsc_builder_line("msg", 7, 101, serde_json::json!([event]))
                ),
                Some("/nix/store/ts.drv"),
            ),
            (
                format!(
                    "{start}{}{stop}",
                    tsc_builder_line("result", 7, 101, serde_json::json!(["{malformed"]))
                ),
                Some("/nix/store/ts.drv"),
            ),
            (
                format!("{start}{result}{result}{stop}"),
                Some("/nix/store/ts.drv"),
            ),
            (
                format!("{start}{start}{result}{stop}"),
                Some("/nix/store/ts.drv"),
            ),
            (
                format!("{start}{}", result.trim_end()),
                Some("/nix/store/ts.drv"),
            ),
            (valid.replace("tsc", "ert"), Some("/nix/store/ts.drv")),
        ] {
            let mut observer = super::CheckBuildTiming::new(derivation.map(str::to_owned), "tsc");
            observer.push(input.as_bytes());
            assert_eq!(
                observer.phase().outcome,
                PhaseOutcome::Unavailable,
                "{input}"
            );
        }
        let mut observer = super::CheckBuildTiming::new(Some("/nix/store/ts.drv".into()), "tsc");
        let input = format!("{}\n{valid}", "x".repeat(9000));
        for chunk in input.as_bytes().chunks(3) {
            observer.push(chunk);
        }
        let phase = observer.phase();
        assert_eq!(phase.duration_ms, Some(37));
        assert_eq!(phase.outcome, PhaseOutcome::Failed);
        let step = super::with_check_execution_evidence(
            "static-typescript",
            StepResult::fail("nix-static-typescript").phases([phase.clone()]),
        );
        assert!(!step.ok);
        assert_eq!(step.phases, [phase]);
    }

    #[test]
    fn typescript_nix_transport_and_historical_output_never_imply_current_execution() {
        let historical_log = tempfile::NamedTempFile::new().unwrap();
        let retained = format!(
            "{}{}{}",
            tsc_start(),
            tsc_builder_line("result", 7, 101, serde_json::json!([TSC_EVENT])),
            tsc_builder_line("stop", 7, 0, serde_json::json!([]))
        );
        std::fs::write(historical_log.path(), retained).unwrap();
        let historical_detail = format!("retained log: {}", historical_log.path().display());
        for detail in ["", TSC_EVENT, &historical_detail] {
            for realization in [
                NixRealization::Reused,
                NixRealization::Realized,
                NixRealization::Unknown,
            ] {
                for ok in [true, false] {
                    let report = injected_nix_report(realization);
                    let phases = report.phase_records();
                    let step = if ok {
                        StepResult::ok("nix-static-typescript")
                    } else {
                        StepResult::fail("nix-static-typescript")
                    }
                    .detail(detail)
                    .with_duration(std::time::Duration::from_millis(91))
                    .phases(phases.clone())
                    .nix(report);
                    let step = super::with_check_execution_evidence("static-typescript", step);
                    assert_eq!(step.ok, ok);
                    assert_eq!(step.duration_ms, 91);
                    assert_eq!(step.detail.as_deref(), Some(detail));
                    assert_eq!(step.nix.as_ref().unwrap().realization, realization);
                    assert_eq!(&step.phases[..3], &phases);
                    assert_eq!(step.phases[3].outcome, PhaseOutcome::Unavailable);
                    assert_eq!(step.phases[3].duration_ms, None);
                }
            }
        }
        assert!(
            super::with_check_execution_evidence("static-code", StepResult::ok("nix-static-code"))
                .phases
                .is_empty()
        );
    }

    use super::{failure_detail, failure_excerpt, write_failure_excerpt};

    const SAMPLE_LOG: &str = "\
fail-probe> build-output-line-1
other-drv> interleaved noise from a parallel derivation
fail-probe> build-output-line-2
fail-probe> FATAL_ERROR_MARKER
error: Cannot build '/nix/store/xxx-fail-probe-0.1.0.drv'.
       Reason: builder failed with exit code 3.
       Last 3 log lines:
       > build-output-line-58
       > build-output-line-59
       > FATAL_ERROR_MARKER
       For full logs, run:
         nix log /nix/store/xxx-fail-probe-0.1.0.drv
";

    #[test]
    fn failure_excerpt_carves_error_block_dropping_interleaved_head() {
        let e = failure_excerpt(SAMPLE_LOG);
        assert!(
            e.starts_with("error: Cannot build"),
            "starts at the error block: {e:?}"
        );
        assert!(e.contains("Last 3 log lines"));
        assert!(e.contains("FATAL_ERROR_MARKER"));
        // The interleaved -L head (prefixed builder lines) is excluded.
        assert!(!e.contains("interleaved noise"));
        assert!(!e.contains("fail-probe> build-output-line-1"));
    }

    #[test]
    fn failure_excerpt_falls_back_to_tail_when_no_error_block() {
        // 60 numbered lines, no column-0 `error:` line.
        let log: String = (1..=60).map(|i| format!("plain-line-{i}\n")).collect();
        let e = failure_excerpt(&log);
        assert!(e.contains("no `error:` block"), "marker present: {e:?}");
        assert!(e.contains("plain-line-60")); // last line kept
        assert!(e.contains("plain-line-11")); // first kept line (last 50 of 60)
        assert!(!e.contains("plain-line-10")); // trimmed head
    }

    #[test]
    fn failure_excerpt_ignores_error_prefixed_by_a_builder() {
        // A builder printing its own `error:` streams as `<name>> error:` — NOT column 0,
        // so it must not be treated as nix's error block.
        let log = "drv> error: cargo test failed\ndrv> more\nerror: builder for 'x' failed\n";
        let e = failure_excerpt(log);
        assert!(e.starts_with("error: builder for 'x' failed"));
        assert!(!e.contains("drv> error"));
    }

    fn failing_sink() -> impl Write + Unpin {
        struct FailingSink;
        impl Write for FailingSink {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("sensitive sink failure"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::other("sensitive sink failure"))
            }
        }
        FailingSink
    }

    #[derive(Clone)]
    struct RecordingSink(Rc<RefCell<Vec<u8>>>);

    impl Write for RecordingSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn recording_sink() -> (RecordingSink, Rc<RefCell<Vec<u8>>>) {
        let bytes = Rc::new(RefCell::new(Vec::new()));
        (RecordingSink(Rc::clone(&bytes)), bytes)
    }

    fn drive_tee<A: Write + Unpin, B: Write + Unpin>(mut tee: BuildStderrTee<A, B>, input: &[u8]) {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tee.write_all(input).await.unwrap();
                tee.shutdown().await.unwrap();
            });
    }

    fn run_stderr_fixture(script: &str) -> (Outcome, Vec<u8>, Vec<u8>, BuildCaptureOutcome) {
        let directory = tempfile::tempdir().unwrap();
        let diagnostic_path = directory.path().join("diagnostic");
        let primary_path = directory.path().join("primary");
        let diagnostic = std::fs::File::create(&diagnostic_path).unwrap();
        let primary = std::fs::File::create(&primary_path).unwrap();
        let (tee, state) = BuildStderrTee::new(diagnostic, primary);
        let outcome = Process::start(
            Command::new("sh")
                .args(["-c", script])
                .inherit_stdin()
                .stdout(StdioMode::Inherit)
                .stderr(StdioMode::Piped)
                .stderr_raw_tee(tee),
        )
        .unwrap()
        .wait()
        .unwrap();
        (
            outcome,
            std::fs::read(diagnostic_path).unwrap(),
            std::fs::read(primary_path).unwrap(),
            state.outcome(),
        )
    }

    #[test]
    fn processkit_tee_drains_complete_stderr_before_successful_wait_returns() {
        let (outcome, diagnostic, primary, capture) =
            run_stderr_fixture("printf 'first\\nlast' >&2");

        assert_eq!(outcome, Outcome::Exited(0));
        assert_eq!(diagnostic, b"first\nlast");
        assert_eq!(primary, diagnostic);
        assert!(!capture.diagnostic_failed);
        assert!(!capture.primary_failed);
    }

    #[test]
    fn processkit_tee_drains_complete_stderr_before_failed_wait_returns() {
        let (outcome, diagnostic, primary, capture) =
            run_stderr_fixture("printf 'failed-without-newline' >&2; exit 7");

        assert_eq!(outcome, Outcome::Exited(7));
        assert_eq!(diagnostic, b"failed-without-newline");
        assert_eq!(primary, diagnostic);
        assert!(!capture.diagnostic_failed);
        assert!(!capture.primary_failed);
    }

    #[test]
    fn processkit_reports_a_signalled_build_fixture() {
        let (outcome, diagnostic, primary, capture) =
            run_stderr_fixture("printf 'before-signal' >&2; kill -TERM $$");

        assert_eq!(outcome, Outcome::Signalled(Some(15)));
        assert_eq!(diagnostic, b"before-signal");
        assert_eq!(primary, diagnostic);
        assert!(!capture.diagnostic_failed);
        assert!(!capture.primary_failed);
    }

    const INHERITED_STDIO_PROBE: &str = "JAUNDER_XTASK_INHERITED_STDIO_PROBE";

    #[test]
    fn inherited_stdio_child_fixture() {
        if std::env::var_os(INHERITED_STDIO_PROBE).is_none() {
            return;
        }
        let (tee, _) = BuildStderrTee::new(io::sink(), io::sink());
        let outcome = Process::start(
            Command::new("sh")
                .args([
                    "-c",
                    "IFS= read -r value; printf 'inherited-stdout:%s' \"$value\"",
                ])
                .inherit_stdin()
                .stdout(StdioMode::Inherit)
                .stderr(StdioMode::Piped)
                .stderr_raw_tee(tee),
        )
        .unwrap()
        .wait()
        .unwrap();

        assert_eq!(outcome, Outcome::Exited(0));
    }

    #[test]
    fn processkit_build_configuration_inherits_stdin_and_stdout() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "steps::nix::tests::inherited_stdio_child_fixture",
                "--nocapture",
            ])
            .env(INHERITED_STDIO_PROBE, "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"probe-value\n")
            .unwrap();
        let output = child.wait_with_output().unwrap();

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("inherited-stdout:probe-value"),
            "nested test stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[test]
    fn build_capture_fans_arbitrary_bytes_to_both_sinks() {
        let input = [vec![b'x'; 200_000], vec![0, 0xff, b'z']].concat();
        let (diagnostic, diagnostic_bytes) = recording_sink();
        let (primary, primary_bytes) = recording_sink();
        let (tee, state) = BuildStderrTee::new(diagnostic, primary);

        drive_tee(tee, &input);

        assert!(!state.outcome().diagnostic_failed);
        assert!(!state.outcome().primary_failed);
        assert_eq!(*diagnostic_bytes.borrow(), input);
        assert_eq!(*primary_bytes.borrow(), input);
    }

    #[test]
    fn build_capture_writes_diagnostic_before_primary() {
        struct OrderedSink {
            name: &'static str,
            writes: Rc<RefCell<Vec<&'static str>>>,
        }
        impl Write for OrderedSink {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.writes.borrow_mut().push(self.name);
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let writes = Rc::new(RefCell::new(Vec::new()));
        let (tee, _) = BuildStderrTee::new(
            OrderedSink {
                name: "diagnostic",
                writes: Rc::clone(&writes),
            },
            OrderedSink {
                name: "primary",
                writes: Rc::clone(&writes),
            },
        );

        drive_tee(tee, b"one chunk");

        assert_eq!(*writes.borrow(), ["diagnostic", "primary"]);
    }

    #[test]
    fn build_capture_diagnostic_failure_warns_once_without_changing_primary_output() {
        let input = vec![b'y'; 200_000];
        let (primary, primary_bytes) = recording_sink();
        let (tee, state) = BuildStderrTee::new(failing_sink(), primary);

        drive_tee(tee, &input);

        let outcome = state.outcome();
        let mut stderr = Vec::new();
        report_build_diagnostic_failure(outcome.diagnostic_failed, &mut stderr);
        assert!(!outcome.primary_failed);
        assert_eq!(*primary_bytes.borrow(), input);
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("xtask.nix.build_diagnostics").count(), 1);
        assert_eq!(warning.lines().count(), 1);
        assert!(!warning.contains("sensitive"));
    }

    #[test]
    fn build_capture_primary_failure_does_not_stop_diagnostic_output() {
        let input = b"complete build output";
        let (diagnostic, diagnostic_bytes) = recording_sink();
        let (tee, state) = BuildStderrTee::new(diagnostic, failing_sink());

        drive_tee(tee, input);

        let BuildCaptureOutcome {
            diagnostic_failed,
            primary_failed,
        } = state.outcome();
        assert!(!diagnostic_failed);
        assert!(primary_failed);
        assert_eq!(*diagnostic_bytes.borrow(), input);
    }

    #[test]
    fn build_completion_wait_error_wins_over_primary_failure() {
        let excerpt_called = Cell::new(false);
        let rescue_called = Cell::new(false);
        let mut stderr = Vec::new();
        let result = finish_build_with(
            BuildCompletion {
                step_name: "nix-check",
                installable: ".#checks.x86_64-linux.check",
                log_path: "build.log",
                diagnostic_failed: false,
                capture: BuildCaptureOutcome {
                    diagnostic_failed: false,
                    primary_failed: true,
                },
                outcome: Err(anyhow::anyhow!("wait failed")),
            },
            || {
                excerpt_called.set(true);
                Ok("excerpt.log".to_owned())
            },
            || {
                rescue_called.set(true);
                false
            },
            || panic!("failed build must not observe after completion"),
            &mut stderr,
        );

        assert!(result.nix.is_none());
        assert_eq!(result.detail.as_deref(), Some("wait failed"));
        assert!(!excerpt_called.get());
        assert!(!rescue_called.get());
    }

    #[test]
    fn build_completion_primary_failure_wins_over_child_failure() {
        let excerpt_called = Cell::new(false);
        let rescue_called = Cell::new(false);
        let mut stderr = Vec::new();
        let result = finish_build_with(
            BuildCompletion {
                step_name: "nix-check",
                installable: ".#checks.x86_64-linux.check",
                log_path: "build.log",
                diagnostic_failed: false,
                capture: BuildCaptureOutcome {
                    diagnostic_failed: false,
                    primary_failed: true,
                },
                outcome: Ok(Outcome::Exited(7)),
            },
            || {
                excerpt_called.set(true);
                Ok("excerpt.log".to_owned())
            },
            || {
                rescue_called.set(true);
                false
            },
            || panic!("failed build must not observe after completion"),
            &mut stderr,
        );

        assert!(result.nix.is_none());
        assert_eq!(
            result.detail.as_deref(),
            Some("failed to stream nix build stderr")
        );
        assert!(!excerpt_called.get());
        assert!(!rescue_called.get());
    }

    #[test]
    fn build_completion_signalled_child_runs_failure_diagnostics() {
        let excerpt_called = Cell::new(false);
        let rescue_called = Cell::new(false);
        let mut stderr = Vec::new();
        let result = finish_build_with(
            BuildCompletion {
                step_name: "nix-check",
                installable: ".#checks.x86_64-linux.check",
                log_path: "build.log",
                diagnostic_failed: false,
                capture: BuildCaptureOutcome::default(),
                outcome: Ok(Outcome::Signalled(Some(15))),
            },
            || {
                excerpt_called.set(true);
                Ok("excerpt.log".to_owned())
            },
            || {
                rescue_called.set(true);
                false
            },
            || panic!("failed build must not observe after completion"),
            &mut stderr,
        );

        assert!(result.nix.is_none());
        assert!(result.detail.unwrap().contains("signal: 15"));
        assert!(excerpt_called.get());
        assert!(rescue_called.get());
    }

    #[test]
    fn build_completion_unknown_signal_runs_failure_diagnostics() {
        let excerpt_called = Cell::new(false);
        let rescue_called = Cell::new(false);
        let mut stderr = Vec::new();
        let result = finish_build_with(
            BuildCompletion {
                step_name: "nix-check",
                installable: ".#checks.x86_64-linux.check",
                log_path: "build.log",
                diagnostic_failed: false,
                capture: BuildCaptureOutcome::default(),
                outcome: Ok(Outcome::Signalled(None)),
            },
            || {
                excerpt_called.set(true);
                Ok("excerpt.log".to_owned())
            },
            || {
                rescue_called.set(true);
                false
            },
            || panic!("failed build must not observe after completion"),
            &mut stderr,
        );

        assert!(result.detail.unwrap().contains("signal: unknown"));
        assert!(result.nix.is_none());
        assert!(excerpt_called.get());
        assert!(rescue_called.get());
    }

    #[test]
    fn build_completion_success_skips_failure_diagnostics() {
        let mut stderr = Vec::new();
        let result = finish_build_with(
            BuildCompletion {
                step_name: "nix-check",
                installable: ".#checks.x86_64-linux.check",
                log_path: "build.log",
                diagnostic_failed: false,
                capture: BuildCaptureOutcome::default(),
                outcome: Ok(Outcome::Exited(0)),
            },
            || panic!("successful build must not write an excerpt"),
            || panic!("successful build must not rescue diagnostics"),
            || injected_nix_report(NixRealization::Unknown),
            &mut stderr,
        );

        assert_eq!(
            result.nix.expect("successful build report").realization,
            NixRealization::Unknown
        );
        assert!(result.ok);
    }

    #[test]
    fn successful_flake_checks_attach_each_realization_from_the_injected_seam() {
        for realization in [
            NixRealization::Reused,
            NixRealization::Realized,
            NixRealization::Unknown,
        ] {
            let mut stderr = Vec::new();
            let step = finish_build_with(
                BuildCompletion {
                    step_name: "nix-check",
                    installable: ".#checks.x86_64-linux.check",
                    log_path: "build.log",
                    diagnostic_failed: false,
                    capture: BuildCaptureOutcome::default(),
                    outcome: Ok(Outcome::Exited(0)),
                },
                || panic!("successful build must not write an excerpt"),
                || panic!("successful build must not rescue diagnostics"),
                || injected_nix_report(realization),
                &mut stderr,
            );
            assert_eq!(
                step.nix.expect("successful build report").realization,
                realization
            );
        }
    }

    #[test]
    fn build_directory_population_fails_closed_but_diagnostics_are_best_effort() {
        let gcroot_error = prepare_build_dirs_with(
            || Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected")),
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(gcroot_error.kind(), io::ErrorKind::PermissionDenied);

        let diagnostic_failure = prepare_build_dirs_with(
            || Ok(()),
            || Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected")),
        )
        .unwrap();
        assert!(diagnostic_failure);
        let primary = StepResult::fail("nix-e2e").detail("original build failure");
        let before = serde_json::to_string(&primary).unwrap();
        let mut stderr = Vec::new();
        report_build_diagnostic_failure(diagnostic_failure, &mut stderr);
        assert_eq!(serde_json::to_string(&primary).unwrap(), before);
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("xtask.nix.build_diagnostics").count(), 1);
        assert_eq!(warning.lines().count(), 1);
        assert!(!warning.contains("injected"));
    }

    #[test]
    fn failure_detail_names_excerpt_first_then_full_log() {
        // `false` exits non-zero, giving a real failed ExitStatus to format.
        let status = std::process::Command::new("false").status().unwrap();
        let with = failure_detail(
            ".#checks.x86_64-linux.e2e",
            &status,
            Some(".xtask/diagnostics/e2e/failure-excerpt.log"),
            Some(".xtask/diagnostics/e2e/build.log"),
        );
        assert!(with.contains(".#checks.x86_64-linux.e2e"));
        assert!(with.contains("exited with"));
        assert!(with.contains("failure-excerpt.log"));
        assert!(with.contains("full build log: .xtask/diagnostics/e2e/build.log"));
        // Excerpt named before the full log.
        assert!(with.find("failure-excerpt.log").unwrap() < with.find("full build log").unwrap());

        let without = failure_detail(
            ".#checks.x86_64-linux.e2e",
            &status,
            None,
            Some(".xtask/diagnostics/e2e/build.log"),
        );
        assert!(without.contains("full build log: .xtask/diagnostics/e2e/build.log"));
        assert!(!without.contains("failure-excerpt.log"));

        let unavailable = failure_detail(".#checks.x86_64-linux.e2e", &status, None, None);
        assert!(!unavailable.contains("failure-excerpt.log"));
        assert!(!unavailable.contains("build.log"));
    }

    #[test]
    fn write_failure_excerpt_writes_sibling_carved_file() {
        let dir = std::env::temp_dir().join(format!("xtask-excerpt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("build.log");
        std::fs::write(&log, SAMPLE_LOG).unwrap();
        let path = write_failure_excerpt(log.to_str().unwrap()).unwrap();
        assert!(path.ends_with("failure-excerpt.log"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("error: Cannot build"));
        assert!(!body.contains("interleaved noise"));
        std::fs::remove_dir_all(&dir).ok();
    }
    #[test]
    fn e2e_capture_host_transport_behavior() {
        let output = std::process::Command::new("python3")
            .args([
                "-B",
                "-m",
                "unittest",
                "discover",
                "-s",
                "nix",
                "-p",
                "test_e2e_capture.py",
            ])
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
            .output()
            .expect("run host capture transport tests");
        assert!(
            output.status.success(),
            "capture transport tests failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn evaluate_e2e_expression(result: &str) -> std::process::Output {
        // Keep the real combo -> check -> execution path, replacing only NixOS
        // realization and store inputs so this remains a primitive evaluation.
        let checks_path =
            serde_json::to_string(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../nix/checks.nix"))
                .expect("checks path");
        let expression = r#"
          let
            lib = rec {
              inherit (builtins) listToAttrs genList concatLists concatStringsSep;
              concatStrings = concatStringsSep "";
              concatMap = f: xs: concatLists (map f xs);
              concatMapStringsSep = sep: f: xs: concatStringsSep sep (map f xs);
              unique = xs: builtins.foldl' (acc: x: if builtins.elem x acc then acc else acc ++ [ x ]) [] xs;
              optionalString = condition: value: if condition then value else "";
              optionalAttrs = condition: value: if condition then value else {};
              removeSuffix = suffix: value: builtins.substring 0 (builtins.stringLength value - builtins.stringLength suffix) value;
            };
            outputs = import (builtins.toPath CHECKS_PATH) {
              self = {};
              system = "x86_64-linux";
              nixpkgs = {};
              nixosInternals = {};
              packageInternals = {
                visualFontConfig = "/primitive/fonts";
                jaunderBin = "/primitive/jaunder";
                e2ePackage = "/primitive/e2e";
              };
              pkgs = {
                inherit lib;
                stdenv.hostPlatform.isLinux = true;
                testers.nixosTest = args: args // { overrideTestDerivation = _: args; };
                writeText = _: _: "/primitive/input";
                nodejs = "/primitive/node";
                playwright-driver.browsers = "/primitive/browsers";
              };
            };
          in RESULT
        "#
        .replace("CHECKS_PATH", &checks_path)
        .replace("RESULT", result);
        // Materialize interpolated source files (not derivation outputs) so the
        // executed Python imports the same capture module as the VM driver.
        std::process::Command::new("nix-instantiate")
            .args([
                "--eval",
                "--read-write-mode",
                "--strict",
                "--json",
                "--expr",
                &expression,
            ])
            .output()
            .expect("primitive Nix lane evaluation")
    }

    fn evaluated_e2e_lanes() -> serde_json::Value {
        let output = evaluate_e2e_expression(
            r#"builtins.listToAttrs (map (name: {
              inherit name;
              value = { inherit (outputs.checks.${name}) globalTimeout testScript; };
            }) (builtins.filter (name: builtins.match "e2e-.*" name != null)
              (builtins.attrNames outputs.checks)))"#,
        );
        assert!(
            output.status.success(),
            "lane evaluation: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("evaluated lanes")
    }

    #[test]
    fn e2e_vm_deadline_accounts_for_the_full_lifecycle() {
        let lanes = evaluated_e2e_lanes();
        let lanes = lanes.as_object().expect("lane map");
        assert_eq!(lanes.len(), 8);
        for (name, lane) in lanes {
            let (suite, deadline) = if name == "e2e-postgres-chromium-unsplit" {
                (1800, 2220)
            } else {
                (1500, 1920)
            };
            assert_eq!(lane["globalTimeout"], deadline, "{name}");
            let script = lane["testScript"].as_str().expect("VM test script");
            let syntax = std::process::Command::new("python3")
                .args([
                    "-B",
                    "-c",
                    r#"
import ast
import json
import sys

module = ast.parse(sys.argv[1])
timeouts = []
for call in ast.walk(module):
    if not isinstance(call, ast.Call) or not call.args:
        continue
    if not isinstance(call.func, ast.Attribute) or call.func.attr != "execute":
        continue
    if not any(isinstance(node, ast.Constant) and isinstance(node.value, str)
               and "node_modules/.bin/playwright test" in node.value
               for node in ast.walk(call.args[0])):
        continue
    timeout = next(keyword.value for keyword in call.keywords if keyword.arg == "timeout")
    timeouts.append(ast.literal_eval(timeout))
print(json.dumps(sorted(timeouts)))
"#,
                    script,
                ])
                .output()
                .expect("parse evaluated VM script");
            assert!(
                syntax.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&syntax.stderr)
            );
            let timeouts: Vec<u64> =
                serde_json::from_slice(&syntax.stdout).expect("browser execution timeouts");
            let expected = if name.contains("firefox") {
                vec![1500, suite]
            } else {
                vec![suite]
            };
            assert_eq!(timeouts, expected, "{name}");
            assert!(timeouts.iter().all(|timeout| *timeout < deadline), "{name}");
        }
        for (producer, suite, deadline, accepted) in [
            ("null", 1500, 1920, true),
            ("null", 1500, 1919, false),
            ("null", 1500, 1680, false),
            ("null", 1800, 2220, true),
            ("null", 1800, 2219, false),
            ("{}", 1500, 6000, true),
        ] {
            let expression = format!(
                r#"(outputs.internals.mkE2eCheck {{
                  backend = "sqlite";
                  checkName = "deadline-boundary";
                  browser = "chromium";
                  traceId = "";
                  traceParent = "";
                  producer = {producer};
                  lane = if {producer} == null then {{}} else null;
                  playwrightTimeout = {suite};
                  vmGlobalTimeout = {deadline};
                }}).globalTimeout"#
            );
            let output = evaluate_e2e_expression(&expression);
            assert_eq!(
                output.status.success(),
                accepted,
                "producer={producer} suite={suite} deadline={deadline}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            if accepted {
                let actual: u64 =
                    serde_json::from_slice(&output.stdout).expect("evaluated VM deadline");
                assert_eq!(actual, deadline);
            }
        }
    }

    #[test]
    fn e2e_panic_gate_verifies_the_lane_qualified_journal() {
        let checks = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("nix")
                .join("checks.nix"),
        )
        .expect("nix/checks.nix");
        assert!(checks.contains("--server-log /tmp/jaunder-journal-${identity}.log"));
        assert!(!checks.contains("--server-log /tmp/jaunder-journal-${backend}.log"));
    }

    #[test]
    fn e2e_result_capture_helper_exercises_source_transport_and_failure_precedence() {
        let lanes = evaluated_e2e_lanes();
        let helper = lanes["e2e-sqlite-chromium-unsplit"]["testScript"]
            .as_str()
            .expect("evaluated VM script");
        let node_binary = std::process::Command::new("which")
            .arg("node")
            .output()
            .expect("locate the pinned Node runtime");
        assert!(node_binary.status.success(), "Node runtime is required");
        let node_binary = String::from_utf8(node_binary.stdout).unwrap();
        let helper = helper.replace("/primitive/node/bin/node", node_binary.trim());
        let script = r#"
import ast
import json
import os
import shlex
import shutil
import subprocess
import tempfile

namespace = {"json": json, "shlex": shlex}
module = ast.parse(__HELPER__)
exec(compile(ast.Module(body=[node for node in module.body if isinstance(node, ast.FunctionDef)], type_ignores=[]), "<evaluated VM helpers>", "exec"), namespace)
capture = namespace["capture_result_file"]
assert_capture_outcome = namespace["assert_capture_outcome"]

class Machine:
  def __init__(self, root, fail_inspection=False, fail_stage=False, fail_output=False, empty_inspection_failure=False):
    self.root = root
    self.output = os.path.join(root, "retained")
    os.makedirs(self.output, exist_ok=True)
    self.fail_inspection = fail_inspection
    self.fail_stage = fail_stage
    self.fail_output = fail_output
    self.empty_inspection_failure = empty_inspection_failure
  def execute(self, command):
    if " -e " in command and self.fail_inspection:
      return (2, "injected inspect failure")
    if " -e " in command and self.empty_inspection_failure:
      return (127, "")
    if command.startswith("cp -- ") and self.fail_stage:
      return (1, "injected stage failure")
    result = subprocess.run(command, shell=True, capture_output=True, text=True)
    return (result.returncode, result.stdout if result.returncode == 0 else result.stderr)
  def copy_from_machine(self, path, target):
    if self.fail_output:
      raise OSError("injected output copy failure")
    shutil.copyfile(path, os.path.join(self.output, os.path.basename(path)))

with tempfile.TemporaryDirectory() as root:
  def paths(name, payload):
    source = os.path.join(root, name + ".source")
    guest = os.path.join(root, name + ".guest")
    with open(source, "wb") as stream:
      stream.write(payload)
    return source, guest

  source, guest = paths("empty-ok", b"")
  errors = []
  result = capture(Machine(root), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "empty"
  assert result["source_to_guest"]["outcome"] == "copied"
  assert result["guest_to_output"]["outcome"] == "copied", result
  assert os.path.getsize(os.path.join(root, "retained", os.path.basename(guest))) == 0
  assert not errors

  source, guest = paths("empty-stage-fail", b"")
  errors = []
  result = capture(Machine(root, fail_stage=True), "sqlite-chromium-unsplit", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "empty"
  assert result["source_to_guest"]["outcome"] == "copy-error"
  assert result["guest_to_output"]["outcome"] == "not-applicable"
  assert "lane sqlite-chromium-unsplit file results.json operation source-to-guest copy" in errors[0]

  malformed = b"{\"partial\":"
  source, guest = paths("malformed", malformed)
  errors = []
  result = capture(Machine(root), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "nonempty"
  assert open(os.path.join(root, "retained", os.path.basename(guest)), "rb").read() == malformed

  missing = os.path.join(root, "does-not-exist")
  result = capture(Machine(root), "sqlite-firefox-ordinary-1-of-2", "results.json", missing, guest, [])
  assert result["source"]["condition"] == "missing"
  assert result["source_to_guest"]["outcome"] == "not-applicable"
  required_errors = []
  capture(Machine(root), "sqlite-firefox-ordinary-1-of-2", "e2e-expected-census.json", missing, guest, required_errors, required=True)
  assert "required source is missing" in required_errors[0]

  source, guest = paths("empty-output-fail", b"")
  errors = []
  result = capture(Machine(root, fail_output=True), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "empty"
  assert result["guest_to_output"]["outcome"] == "copy-error"

  source, guest = paths("inspection-error", b"data")
  errors = []
  result = capture(Machine(root, fail_inspection=True), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "inspection-error"
  assert "operation source inspection" in errors[0]
  errors = []
  result = capture(Machine(root, empty_inspection_failure=True), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "inspection-error"
  assert "exited with status 127" in result["source"]["detail"], result

  source, guest = paths("output-fail", b"{\"ok\":true}")
  errors = []
  result = capture(Machine(root, fail_output=True), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source_to_guest"]["outcome"] == "copied"
  assert result["guest_to_output"]["outcome"] == "copy-error"
  assert "operation guest-to-output copy" in errors[0]

  try:
    assert_capture_outcome(9, ["ancillary lift failed"], "sqlite-chromium-unsplit")
  except AssertionError as error:
    assert "Playwright failed (exit 9)" in str(error)
    assert "capture retention failed" not in str(error)
  else:
    raise AssertionError("Playwright failure was lost")
  try:
    assert_capture_outcome(0, ["required lift failed"], "sqlite-chromium-unsplit")
  except AssertionError as error:
    assert "capture retention failed" in str(error)
  else:
    raise AssertionError("successful Playwright hid required lift failure")
print("actual result-capture helper cases passed")
"#
        .replace("__HELPER__", &format!("{helper:?}"));
        let output = std::process::Command::new("python3")
            .args(["-B", "-c", &script])
            .output()
            .expect("launch capture helper harness");
        assert!(
            output.status.success(),
            "capture helper harness failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("actual result-capture helper cases passed")
        );
    }

    #[test]
    fn e2e_vm_routes_result_files_through_capture_receipt_before_verdict() {
        let checks =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../nix/checks.nix"))
                .expect("nix/checks.nix");
        let result_helper = checks
            .find("# BEGIN result capture helper")
            .expect("actual capture helper");
        let census_capture = result_helper
            + checks[result_helper..]
                .find("/tmp/e2e/test-results/e2e-expected-census.json")
                .expect("configured census source");
        let gate_start = checks
            .find("gate_started_at = time.monotonic()")
            .expect("Playwright starts");
        let report_capture = gate_start
            + checks[gate_start..]
                .find("/tmp/e2e/test-results/results.json")
                .expect("Playwright report capture");
        let duration_capture = report_capture
            + checks[report_capture..]
                .find("/tmp/e2e/test-results/duration-budget-manifest.json")
                .expect("duration report capture");
        let assertion = gate_start
            + checks[gate_start..]
                .find("assert_capture_outcome(pw_status, retention_errors")
                .expect("Playwright/capture verdict policy");
        assert!(result_helper < census_capture && census_capture < gate_start);
        assert!(gate_start < report_capture && report_capture < duration_capture);
        assert!(duration_capture < assertion);
        assert!(checks.contains("\"result_captures\": e2e_result_captures"));
        assert!(checks.contains("\"condition\": \"not-applicable\""));
        assert!(checks.contains("/tmp/playwright-report-${identity}.json"));
        assert!(checks.contains("/tmp/duration-budget-manifest-${identity}.json"));
        assert!(checks.contains("/tmp/e2e-census-${identity}.json"));
        assert!(checks.contains("retention_errors.extend(retain_e2e_capture())"));
        assert!(checks.contains("assert panic_status == 0"));
    }

    fn run_seed_timeout_harness(timed_out_command: &str, identity: &str) -> serde_json::Value {
        let lanes = evaluated_e2e_lanes();
        let helper = lanes[format!("e2e-{identity}")]["testScript"]
            .as_str()
            .expect("evaluated VM script");
        let script = format!(
            r#"
import ast
import contextlib
import json
from pathlib import Path
import sys
import tarfile
import tempfile

temporary = tempfile.TemporaryDirectory()

class FakeMachine:
  def __init__(self):
    self.actions = []
    self.state_dir = Path(temporary.name) / "vm-state-machine"
    self.out_dir = Path(temporary.name) / "output"
    (self.state_dir / "xchg").mkdir(parents=True)
    self.out_dir.mkdir()
    (self.state_dir / "xchg" / "diag.log").write_bytes(b"abc")

  def execute(self, command, timeout=None):
    self.actions.append(["execute", command, timeout])
    if command == {timed_out_command:?}:
      return (124, "")
    if "sha256sum" in command:
      return (0, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  /var/lib/jaunder/capture/diag.log\\n")
    if "-printf" in command:
      return (0, "3 /var/lib/jaunder/capture/diag.log\\n")
    return (0, "")

  def copy_from_machine(self, path, target):
    self.actions.append(["copy_from_machine", path, target])

  def succeed(self, command):
    self.actions.append(["succeed", command])
    return ""

  def wait_for_unit(self, unit, timeout=None):
    self.actions.append(["wait_for_unit", unit, timeout])

  def wait_for_open_port(self, port, timeout=None):
    self.actions.append(["wait_for_open_port", port, timeout])

machine = FakeMachine()
module = ast.parse({helper:?})
exec(compile(ast.Module(body=[node for node in module.body if isinstance(node, ast.FunctionDef)], type_ignores=[]), "<evaluated VM helpers>", "exec"))
try:
  with contextlib.redirect_stdout(sys.stderr):
    assert_seed_storage_spans()
except AssertionError as error:
  with tarfile.open(machine.out_dir / "capture-{identity}.tar.gz") as archive:
    payload = archive.extractfile("capture/diag.log").read().decode()
  print(json.dumps({{"error": str(error), "actions": machine.actions, "capture_bytes": payload}}))
else:
  raise AssertionError("seed timeout harness unexpectedly succeeded")
finally:
  temporary.cleanup()
"#
        );
        let output = std::process::Command::new("python3")
            .arg("-B")
            .arg("-c")
            .arg(script)
            .output()
            .expect("launching Python seed timeout harness");
        assert!(
            output.status.success(),
            "seed timeout harness failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("capture-hash exit=0"),
            "capture-retention diagnostics must remain on stderr: {:?}",
            output.stderr
        );
        serde_json::from_slice(&output.stdout).expect("seed timeout harness JSON")
    }

    fn assert_seed_timeout_harness(
        timed_out_command: &str,
        expected_token: &str,
        forbidden_stage: &str,
    ) {
        for identity in ["sqlite-chromium-unsplit", "postgres-chromium-unsplit"] {
            assert_seed_timeout_result(
                run_seed_timeout_harness(timed_out_command, identity),
                timed_out_command,
                expected_token,
                forbidden_stage,
            );
        }
    }

    fn assert_seed_timeout_result(
        result: serde_json::Value,
        timed_out_command: &str,
        expected_token: &str,
        forbidden_stage: &str,
    ) {
        let expected_error = if expected_token == "seed-collector-stop-timeout" {
            "seed-collector-stop-timeout: collector stop exceeded 30s"
        } else {
            "seed-trace-verifier-timeout: verification exceeded 30s"
        };
        assert_eq!(result["error"], expected_error);
        let actions = result["actions"].as_array().expect("harness actions");
        let timeout = actions
            .iter()
            .find(|action| action[0] == "execute" && action[1] == timed_out_command)
            .expect("timed out command");
        assert_eq!(timeout[2], 30);
        assert_eq!(result["capture_bytes"], "abc");
        assert!(
            !actions.iter().any(|action| {
                action[0] == "copy_from_machine"
                    || action[1]
                        .as_str()
                        .is_some_and(|command| command.contains("tar czf"))
            }),
            "capture must be archived directly from the host exchange: {actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| action.to_string().contains("systemctl start ")),
            "timeout path must not restart a service: {actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| action.to_string().contains(forbidden_stage)),
            "timeout path must not reach {forbidden_stage}: {actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| action.to_string().contains("playwright")),
            "timeout path must not reach Playwright: {actions:?}"
        );
    }

    #[test]
    fn e2e_seed_collector_stop_timeout_retains_capture_before_failing() {
        assert_seed_timeout_harness(
            "systemctl stop otel-collector.service",
            "seed-collector-stop-timeout",
            "test-support verify-seed-trace",
        );
    }

    #[test]
    fn e2e_seed_verifier_timeout_retains_capture_before_failing() {
        assert_seed_timeout_harness(
            "test-support verify-seed-trace --capture-dir /var/lib/jaunder/capture 2>&1",
            "seed-trace-verifier-timeout",
            "systemctl start otel-collector.service",
        );
    }

    #[test]
    fn e2e_seed_verifier_merges_stderr_into_the_reported_failure() {
        let checks = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("nix")
                .join("checks.nix"),
        )
        .expect("nix/checks.nix");
        let command = "test-support verify-seed-trace --capture-dir /var/lib/jaunder/capture 2>&1";
        let command_at = checks.find(command).expect("verifier stderr is captured");
        let assertion_at = checks[command_at..]
            .find("raise AssertionError(verify_out)")
            .map(|offset| command_at + offset)
            .expect("captured verifier output is reported");
        assert!(command_at < assertion_at);
    }

    #[test]
    fn e2e_artifact_failure_detail_keeps_cause_for_provenance_and_regular_files() {
        for name in [
            "capture-sqlite-firefox-ordinary-1-of-2.tar.gz",
            "producer-input-sqlite-firefox-ordinary-1-of-2.json",
        ] {
            let detail = super::e2e_artifact_failure_detail(
                name,
                "copy",
                "copying retained file: PermissionDenied",
            );
            assert!(detail.contains("lane sqlite-firefox-ordinary-1-of-2"));
            assert!(detail.contains(name));
            assert!(detail.contains("operation copy failed"));
            assert!(detail.contains("PermissionDenied"));
        }
    }

    #[test]
    fn copy_e2e_diagnostics_between_copies_journal_capture_playwright_and_manifest() {
        let tmp = std::env::temp_dir().join(format!("xtask-j-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("jaunder-journal-sqlite.log"), b"j").unwrap();
        let malformed_report = b"{\"partial\":";
        std::fs::write(src.join("playwright-report-sqlite.json"), malformed_report).unwrap();
        std::fs::write(src.join("e2e-census-sqlite.json"), b"").unwrap();
        std::fs::write(src.join("duration-budget-manifest-sqlite.json"), b"m").unwrap();
        // #123/#49 failure-path artifacts: the trace/screenshot tarball and the
        // full system journal, copied out of the VM before the check is failed.
        std::fs::write(src.join("playwright-artifacts-sqlite.tar.gz"), b"a").unwrap();
        std::fs::write(src.join("system-journal-sqlite.log"), b"s").unwrap();
        // #227 capture-dir tarball (per-backend name is required).
        std::fs::write(src.join("capture-sqlite.tar.gz"), b"d").unwrap();
        std::fs::write(src.join("unrelated.txt"), b"x").unwrap();
        // A bare `capture.tar.gz` (no `-<backend>`) must NOT match — the filter requires
        // the `capture-<backend>` prefix the flake's tar step always produces.
        std::fs::write(src.join("capture.tar.gz"), b"n").unwrap();
        // The manifest only has meaning when carried under the per-backend basename
        // the VM capture step emits; the reporter's in-tree name is never lifted.
        std::fs::write(src.join("duration-budget-manifest.json"), b"n").unwrap();

        let outcome = super::copy_e2e_diagnostics_between(&src, &dest);

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert_eq!(
            outcome.copied, 7,
            "journal + report + census + manifest + artifacts tarball + system journal + capture tarball \
             are copied; unrelated and in-tree artifact names are not"
        );
        assert!(dest.join("jaunder-journal-sqlite.log").exists());
        assert_eq!(
            std::fs::read(dest.join("playwright-report-sqlite.json")).unwrap(),
            malformed_report
        );
        assert_eq!(
            std::fs::metadata(dest.join("e2e-census-sqlite.json"))
                .unwrap()
                .len(),
            0
        );
        assert!(dest.join("duration-budget-manifest-sqlite.json").exists());
        assert!(dest.join("playwright-artifacts-sqlite.tar.gz").exists());
        assert!(dest.join("system-journal-sqlite.log").exists());
        assert!(dest.join("capture-sqlite.tar.gz").exists());
        assert!(!dest.join("unrelated.txt").exists());
        assert!(
            !dest.join("capture.tar.gz").exists(),
            "un-suffixed capture.tar.gz must not be lifted"
        );
        assert!(
            !dest.join("duration-budget-manifest.json").exists(),
            "the in-tree manifest name must not be lifted"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn copy_e2e_diagnostics_removes_stale_post_build_inputs_before_lifting() {
        let tmp = std::env::temp_dir().join(format!("xtask-stale-e2e-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("playwright-report-sqlite.json"), b"stale report").unwrap();
        std::fs::write(
            dest.join("duration-budget-manifest-sqlite.json"),
            b"stale manifest",
        )
        .unwrap();
        std::fs::write(dest.join("capture-sqlite.tar.gz"), b"stale capture").unwrap();

        let outcome = super::copy_e2e_diagnostics_between(&src, &dest);

        assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
        assert_eq!(outcome.copied, 0);
        for name in [
            "playwright-report-sqlite.json",
            "duration-budget-manifest-sqlite.json",
            "capture-sqlite.tar.gz",
        ] {
            assert!(
                !dest.join(name).exists(),
                "a missing current {name} must not leave a prior attempt's input"
            );
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn e2e_provenance_lift_failure_does_not_fail_a_successful_lane() {
        for operation in ["remove", "copy", "permissions"] {
            let tmp = tempfile::tempdir().unwrap();
            let src = tmp.path().join("src");
            let dest = tmp.path().join("dest");
            std::fs::create_dir_all(&src).unwrap();
            std::fs::create_dir_all(&dest).unwrap();
            let name = "producer-input-postgres-firefox-ordinary-1-of-2.json";
            std::fs::write(src.join(name), b"current marker").unwrap();
            std::fs::write(dest.join(name), b"stale marker").unwrap();
            let denied = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
            let outcome = super::copy_e2e_diagnostics_with_ops(
                &src,
                &dest,
                |path| {
                    if operation == "remove" {
                        Err(denied())
                    } else {
                        std::fs::remove_file(path)
                    }
                },
                |from, to| {
                    if operation == "copy" {
                        Err(denied())
                    } else {
                        std::fs::copy(from, to)
                    }
                },
                |path, permissions| {
                    if operation == "permissions" {
                        Err(denied())
                    } else {
                        std::fs::set_permissions(path, permissions)
                    }
                },
            );
            assert!(
                outcome.failures.is_empty(),
                "{operation}: {:?}",
                outcome.failures
            );
            if operation == "copy" {
                assert!(
                    !dest.join(name).exists(),
                    "failed copying must not preserve a stale marker"
                );
            }
        }
    }

    #[test]
    fn e2e_provenance_marker_is_cleared_when_current_producer_emits_none() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        let name = "producer-input-postgres-chromium-unsplit.json";
        std::fs::write(dest.join(name), b"previous valid marker").unwrap();
        let outcome = super::copy_e2e_diagnostics_between(&src, &dest);
        assert!(outcome.failures.is_empty());
        assert!(
            !dest.join(name).exists(),
            "previous marker cannot attest a new attempt"
        );
        std::fs::write(src.join(name), b"current marker").unwrap();
        assert_eq!(super::copy_e2e_diagnostics_between(&src, &dest).copied, 1);
        assert_eq!(std::fs::read(dest.join(name)).unwrap(), b"current marker");
    }

    #[test]
    fn copy_e2e_diagnostics_overwrites_a_read_only_previous_copy() {
        // Artifacts come from the nix store, so the previous run's copy is on disk
        // read-only and `fs::copy` onto it fails EACCES. That was swallowed, leaving
        // the FIRST run's capture in place — and the #681 gate reads its capture from
        // this directory, so it would verify a new build against stale traces.
        let tmp = std::env::temp_dir().join(format!("xtask-ro-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();

        std::fs::write(src.join("capture-sqlite.tar.gz"), b"first").unwrap();
        assert_eq!(super::copy_e2e_diagnostics_between(&src, &dest).copied, 1);
        // Reproduce the store's 0444 on the destination.
        std::fs::set_permissions(
            dest.join("capture-sqlite.tar.gz"),
            std::fs::Permissions::from_mode(0o444),
        )
        .unwrap();

        std::fs::write(src.join("capture-sqlite.tar.gz"), b"second").unwrap();
        assert_eq!(super::copy_e2e_diagnostics_between(&src, &dest).copied, 1);

        assert_eq!(
            std::fs::read(dest.join("capture-sqlite.tar.gz")).unwrap(),
            b"second",
            "a re-run must replace the previous capture, not silently keep it"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn copy_e2e_diagnostics_reports_a_file_it_could_not_copy() {
        // Inject a transport error so reporting is exercised independently of
        // host filesystem permissions.
        let tmp = std::env::temp_dir().join(format!("xtask-copy-fail-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        let name = "capture-sqlite-firefox-ordinary-1-of-2.tar.gz";
        std::fs::write(src.join(name), b"new").unwrap();
        let outcome = super::copy_e2e_diagnostics_with_ops(
            &src,
            &dest,
            |_| Ok(()),
            |_, _| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            |_, _| Ok(()),
        );
        assert_eq!(outcome.copied, 0);
        assert_eq!(outcome.failures.len(), 1, "{:?}", outcome.failures);
        assert!(outcome.failures[0].contains(
            "lane sqlite-firefox-ordinary-1-of-2 file capture-sqlite-firefox-ordinary-1-of-2.tar.gz operation copy failed: copying"
        ));
        assert!(outcome.failures[0].contains("PermissionDenied"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ancillary_warning_build_diagnostic_losses_aggregate_once() {
        let status = std::process::Command::new("false").status().unwrap();
        for failed in ["capture", "excerpt", "rescue"] {
            let mut stderr = Vec::new();
            let result = failed_build_after_diagnostics_with(
                FailedBuildDiagnostics {
                    step_name: "nix-e2e",
                    installable: ".#checks.x86_64-linux.e2e",
                    status: &status,
                    log_path: ".xtask/diagnostics/e2e/build.log",
                    capture_failed: failed == "capture",
                },
                || {
                    if failed == "capture" {
                        panic!("an unreliable capture must not be read");
                    }
                    if failed == "excerpt" {
                        return Err(io::Error::other("sensitive excerpt failure"));
                    }
                    Ok(".xtask/diagnostics/e2e/failure-excerpt.log".to_owned())
                },
                || failed == "rescue",
                &mut stderr,
            );
            let expected_detail = failure_detail(
                ".#checks.x86_64-linux.e2e",
                &status,
                (failed == "rescue").then_some(".xtask/diagnostics/e2e/failure-excerpt.log"),
                (failed != "capture").then_some(".xtask/diagnostics/e2e/build.log"),
            );
            assert!(!result.ok);
            let detail = result.detail.unwrap();
            assert_eq!(detail, expected_detail);
            if failed == "capture" {
                assert!(!detail.contains("failure-excerpt.log"));
                assert!(!detail.contains("build.log"));
            }
            let warning = String::from_utf8(stderr).unwrap();
            assert_eq!(warning.matches("xtask.nix.build_diagnostics").count(), 1);
            assert_eq!(warning.lines().count(), 1);
            assert!(!warning.contains("sensitive"));
        }
    }
}
