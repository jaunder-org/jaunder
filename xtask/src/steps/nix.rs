use std::fs::File;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::result::{CommandResult, StepResult};

/// The flake checks are Linux-only (`optionalAttrs isLinux` in flake.nix);
/// the project's CI host is x86_64-linux.
const SYSTEM: &str = "x86_64-linux";

#[derive(Clone, Copy)]
enum TestCheck {
    Wasm,
    Coverage,
    Doctests,
}

const TEST_CHECKS: [TestCheck; 3] = [TestCheck::Wasm, TestCheck::Coverage, TestCheck::Doctests];
const CHECK_SUPPORTING_TEST_CHECKS: [TestCheck; 2] = [TestCheck::Wasm, TestCheck::Doctests];
const STATIC_CHECK: &str = "static-checks";

/// The browser/backend combinations selected by the flake's `e2eCombos` catalog.
///
/// `validate` must address these outputs one by one: `e2e`'s aggregate is a
/// symlink join, which cannot retain two per-backend files with the same name.
const E2E_COMBOS: [(&str, &str); 4] = [
    ("sqlite", "chromium"),
    ("sqlite", "firefox"),
    ("postgres", "chromium"),
    ("postgres", "firefox"),
];

impl TestCheck {
    const fn name(self) -> &'static str {
        match self {
            Self::Wasm => "wasm-tests",
            Self::Coverage => "coverage",
            Self::Doctests => "doctests",
        }
    }

    fn run(self, result: &mut CommandResult) {
        match self {
            Self::Wasm => {
                let name = self.name();
                result.push(build_check(name, name));
            }
            Self::Coverage => coverage(result),
            Self::Doctests => doctests(result),
        }
    }
}

fn selected_test_checks(no_test: bool) -> &'static [TestCheck] {
    if no_test { &[] } else { &TEST_CHECKS }
}

#[cfg(test)]
fn test_check_names(no_test: bool) -> impl ExactSizeIterator<Item = &'static str> {
    selected_test_checks(no_test)
        .iter()
        .copied()
        .map(TestCheck::name)
}

#[cfg(test)]
pub(crate) fn check_supporting_test_check_names() -> impl ExactSizeIterator<Item = &'static str> {
    CHECK_SUPPORTING_TEST_CHECKS
        .iter()
        .copied()
        .map(TestCheck::name)
}

#[cfg(test)]
fn validate_check_names() -> impl Iterator<Item = &'static str> {
    std::iter::once(STATIC_CHECK).chain(TEST_CHECKS.iter().copied().map(TestCheck::name))
}

/// Run the hermetic static-check derivation. This is validate-only: `check`
/// already runs the host-local static lane, while CI's required validate job
/// needs the same definitions proven inside Nix.
pub fn static_checks(result: &mut CommandResult) {
    result.push(build_check("nix-static-checks", STATIC_CHECK));
}

/// Run the Nix-backed test checks unless `--no-test` disables the group.
pub fn test_checks(result: &mut CommandResult, no_test: bool) {
    for check in selected_test_checks(no_test) {
        check.run(result);
    }
}

/// Run the Nix-backed test checks that still have no host-native replacement for
/// the day-to-day `check` command.
pub fn check_supporting_test_checks(result: &mut CommandResult, no_test: bool) {
    if no_test {
        return;
    }
    for check in CHECK_SUPPORTING_TEST_CHECKS {
        check.run(result);
    }
}

/// The Nix coverage check: the instrumented test suite (SQLite- and
/// PostgreSQL-backed tests together in one pass under an ephemeral PostgreSQL)
/// emits the reports; the regression gate + auto-heal then runs host-side over
/// the check's `$out`.
pub fn coverage(result: &mut CommandResult) {
    // The producer always succeeds and always emits `$out` (reports + status +
    // diagnostics). The consumer (`coverage-gate`) fails iff the in-sandbox
    // sentinel reports a test/infra failure.
    result.push(build_check("nix-coverage", "coverage"));
    let gate = build_check("nix-coverage-gate", "coverage-gate");
    if !gate.ok {
        // A failed gate is an in-sandbox failure (test or infrastructure) — the
        // authoritative category lives in the producer's status.json. Report it
        // precisely (not as an opaque build failure) and skip host
        // post-processing (there is no coverage verdict to compute).
        let status_path = ".xtask/gcroots/coverage/status.json";
        result.push(failed_status_step(
            "coverage",
            "xtask.nix.coverage_status",
            "coverage gate failed (no status.json)",
            || std::fs::read_to_string(status_path),
            coverage::status::CoverageStatus::from_json,
            sentinel_detail,
            &mut std::io::stderr(),
        ));
        return;
    }
    result.push(gate);
    // `crate::coverage` is xtask's host-side gate module; `coverage` (no
    // `crate::`) is the shared crate holding the sentinel schema.
    let (step, report) = crate::coverage::run(".xtask/gcroots/coverage");
    result.push(step);
    result.coverage = report;
}

/// The Nix doctest check (#763): the producer runs the workspace doctests and
/// reconciles them against the fence population read from the source; the consumer
/// fails on a non-ok sentinel.
///
/// Doctests are the one suite `nextest` structurally cannot run, so the `coverage`
/// check above never sees them. Reconciling — rather than merely running — is what
/// stops a green report standing for a population that was never looked at.
pub fn doctests(result: &mut CommandResult) {
    result.push(build_check("nix-doctests", "doctests"));
    let gate = build_check("nix-doctests-gate", "doctests-gate");
    if !gate.ok {
        // As with coverage: the authoritative detail is the producer's
        // status.json, so report the violations rather than an opaque build
        // failure.
        let status_path = ".xtask/gcroots/doctests/status.json";
        result.push(failed_status_step(
            "doctests",
            "xtask.nix.doctest_status",
            "doctest gate failed (no status.json)",
            || std::fs::read_to_string(status_path),
            doctests::status::DoctestStatus::from_json,
            doctest_sentinel_detail,
            &mut std::io::stderr(),
        ));
        return;
    }
    result.push(gate);
}

fn failed_status_step<T>(
    step: &str,
    warning_key: &str,
    fallback: &str,
    read_status: impl FnOnce() -> io::Result<String>,
    parse_status: impl FnOnce(&str) -> Result<T>,
    render: impl FnOnce(&T) -> String,
    stderr: &mut impl Write,
) -> StepResult {
    StepResult::fail(step).detail(failed_status_detail(
        warning_key,
        fallback,
        read_status,
        parse_status,
        render,
        stderr,
    ))
}
fn failed_status_detail<T>(
    warning_key: &str,
    fallback: &str,
    read_status: impl FnOnce() -> io::Result<String>,
    parse_status: impl FnOnce(&str) -> Result<T>,
    render: impl FnOnce(&T) -> String,
    stderr: &mut impl Write,
) -> String {
    let parsed = match read_status() {
        Ok(raw) => parse_status(&raw),
        Err(error) => Err(error.into()),
    };
    match parsed {
        Ok(status) => render(&status),
        Err(_) => {
            let _ = writeln!(
                stderr,
                "xtask: warning: {warning_key}: ignored failure while reading failed-gate status"
            );
            fallback.to_owned()
        }
    }
}

/// Render the in-sandbox doctest sentinel into a human detail. Pure + tested.
///
/// Each violation renders as `file:line [kind] detail`, with `kind` serde-rendered
/// (kebab-case) rather than `Debug`-printed, so this message and the gate
/// derivation's `jq` output read identically.
fn doctest_sentinel_detail(status: &doctests::status::DoctestStatus) -> String {
    use doctests::status::StatusCategory::{Infra, Ok, Violations};
    match status.category {
        Ok => "in-sandbox: doctests ok".to_string(),
        Infra => format!(
            "doctest emit could not run: {}",
            status.infra_detail.as_deref().unwrap_or("unknown")
        ),
        Violations => {
            let lines: Vec<String> = status
                .violations
                .iter()
                .map(|v| {
                    let kind = serde_json::to_string(&v.kind)
                        .unwrap_or_default()
                        .trim_matches('"')
                        .to_string();
                    format!("{}:{} [{}] {}", v.file, v.line, kind, v.detail)
                })
                .collect();
            format!("{} violation(s):\n{}", lines.len(), lines.join("\n"))
        }
    }
}

/// Render the in-sandbox sentinel into a human `StepResult` detail. Pure +
/// tested; the I/O (reading status.json, running nix build) stays in
/// `coverage()`.
fn sentinel_detail(status: &coverage::status::CoverageStatus) -> String {
    use coverage::status::StatusCategory::{Infra, TestFailure, TestsOk};
    match status.category {
        TestsOk => "in-sandbox: tests ok".to_string(),
        Infra => format!(
            "infrastructure failure (not a coverage regression): {}",
            status.infra_detail.as_deref().unwrap_or("unknown")
        ),
        TestFailure => format!(
            "test failure(s) (not a coverage regression): {}",
            status.failed_tests.join(", ")
        ),
    }
}

/// Run every browser/backend E2E combo independently, rather than building the
/// aggregate symlink join. Each combo has same-named per-backend report inputs,
/// so only its own GC root can preserve a trustworthy report/manifest pair.
///
/// `postgres-integration` is deliberately not dispatched — its tests already run
/// under the coverage check. The separate Elisp integration check remains part
/// of the full `validate` route.
pub fn e2e(result: &mut CommandResult) {
    route_e2e_combos(E2E_COMBOS, |backend, browser| {
        e2e_combo(result, backend, browser);
    });
    elisp_integration(result);
}

fn route_e2e_combos(
    combos: impl IntoIterator<Item = (&'static str, &'static str)>,
    mut route_combo: impl FnMut(&str, &str),
) {
    for (backend, browser) in combos {
        route_combo(backend, browser);
    }
}

/// Build a single E2E {backend}×{browser} combo, lift its diagnostics, then
/// validate the report and manifest only when the VM itself succeeded. Used by
/// both CI's `cargo xtask e2e` matrix path and `cargo xtask validate`.
pub fn e2e_combo(result: &mut CommandResult, backend: &str, browser: &str) {
    let check = format!("e2e-{backend}-{browser}");
    let step_name = format!("nix-{check}");
    let build = build_check(&step_name, &check);
    finish_e2e_combo(
        result,
        build,
        || {
            lift_e2e_diagnostics(
                Path::new(&format!(".xtask/gcroots/{check}")),
                Path::new(&format!(".xtask/diagnostics/{check}")),
            );
        },
        || crate::steps::duration_budget::validate_lifted_combo(backend, browser),
    );
}

/// Preserve ADR-0037's diagnostic-before-failure order. A failed VM has already
/// explained its own failure, so only a successful VM is fail-closed on missing
/// or inconsistent duration artifacts.
fn finish_e2e_combo(
    result: &mut CommandResult,
    build: StepResult,
    lift: impl FnOnce(),
    validate: impl FnOnce() -> StepResult,
) {
    let succeeded = build.ok;
    result.push(build);
    lift();
    if succeeded {
        result.push(validate());
    }
}

/// Runs the hermetic elisp live-integration `nixosTest` check (ADR-0035): a NixOS
/// VM with Emacs + the jaunder binary, where the harness self-boots the server.
/// `validate` calls this separately after the browser combos; this dedicated
/// builder is the per-job CI path.
pub fn elisp_integration(result: &mut CommandResult) {
    let check = "e2e-elisp-integration";
    result.push(build_check("nix-elisp-integration", check));
    lift_e2e_diagnostics(
        std::path::Path::new(&format!(".xtask/gcroots/{check}")),
        std::path::Path::new(&format!(".xtask/diagnostics/{check}")),
    );
}

/// What one diagnostics-copy pass did.
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
        |path| std::fs::remove_file(path),
        |from, to| std::fs::copy(from, to),
        |path, permissions| std::fs::set_permissions(path, permissions),
    )
}

fn copy_e2e_diagnostics_with_ops(
    src_dir: &Path,
    dest_dir: &Path,
    mut remove: impl FnMut(&Path) -> io::Result<()>,
    mut copy: impl FnMut(&Path, &Path) -> io::Result<u64>,
    mut set_permissions: impl FnMut(&Path, std::fs::Permissions) -> io::Result<()>,
) -> DiagnosticsCopy {
    let wanted = |name: &str| {
        (name.starts_with("jaunder-journal-") && name.ends_with(".log"))
            || (name.starts_with("system-journal-") && name.ends_with(".log"))
            || (name.starts_with("playwright-report-") && name.ends_with(".json"))
            || (name.starts_with("duration-budget-manifest-") && name.ends_with(".json"))
            || (name.starts_with("playwright-artifacts-") && name.ends_with(".tar.gz"))
            || (name.starts_with("capture-") && name.ends_with(".tar.gz"))
    };
    let entries = match std::fs::read_dir(src_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return DiagnosticsCopy {
                copied: 0,
                failures: Vec::new(),
            };
        }
        Err(error) => {
            return DiagnosticsCopy {
                copied: 0,
                failures: vec![format!("reading {}: {error}", src_dir.display())],
            };
        }
    };
    let mut copied = 0;
    let mut failures = Vec::new();
    if let Err(error) = std::fs::create_dir_all(dest_dir) {
        failures.push(format!("creating {}: {error}", dest_dir.display()));
    }
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(format!(
                    "reading entry under {}: {error}",
                    src_dir.display()
                ));
                continue;
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !wanted(name) {
            continue;
        }
        let from = entry.path();
        let to = dest_dir.join(name);
        match remove(&to) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                failures.push(format!("removing {}: {error}", to.display()));
                continue;
            }
        }
        match copy(&from, &to) {
            Ok(_) => {
                if let Err(error) = set_permissions(&to, std::fs::Permissions::from_mode(0o644)) {
                    failures.push(format!("setting permissions on {}: {error}", to.display()));
                }
                copied += 1;
            }
            Err(error) => failures.push(format!(
                "copying {} to {}: {error}",
                from.display(),
                to.display()
            )),
        }
    }
    DiagnosticsCopy { copied, failures }
}

/// Best-effort diagnostics lift with one aggregate warning per attempt.
fn lift_e2e_diagnostics(src_dir: &Path, dest_dir: &Path) -> usize {
    lift_e2e_diagnostics_with(src_dir, dest_dir, &mut std::io::stderr())
}

fn lift_e2e_diagnostics_with(src_dir: &Path, dest_dir: &Path, stderr: &mut impl Write) -> usize {
    lift_e2e_diagnostics_with_ops(
        src_dir,
        dest_dir,
        |path| std::fs::remove_file(path),
        |from, to| std::fs::copy(from, to),
        |path, permissions| std::fs::set_permissions(path, permissions),
        stderr,
    )
}

fn lift_e2e_diagnostics_with_ops(
    src_dir: &Path,
    dest_dir: &Path,
    remove: impl FnMut(&Path) -> io::Result<()>,
    copy: impl FnMut(&Path, &Path) -> io::Result<u64>,
    set_permissions: impl FnMut(&Path, std::fs::Permissions) -> io::Result<()>,
    stderr: &mut impl Write,
) -> usize {
    report_diagnostics_copy(
        copy_e2e_diagnostics_with_ops(src_dir, dest_dir, remove, copy, set_permissions),
        stderr,
    )
}

fn report_diagnostics_copy(outcome: DiagnosticsCopy, stderr: &mut impl Write) -> usize {
    if !outcome.failures.is_empty() {
        let _ = writeln!(
            stderr,
            "xtask: warning: xtask.nix.e2e_diagnostics: ignored failure(s) while lifting e2e diagnostics"
        );
    }
    outcome.copied
}

/// Fans every chunk to a diagnostic sink and the primary stderr sink while
/// continuing to drain after either fails. The owner inspects the recorded
/// failures after EOF: diagnostic failure warns; primary failure fails the step.
struct MultiWriter<A: Write, B: Write> {
    diagnostic: A,
    primary: B,
    diagnostic_failed: bool,
    primary_failed: bool,
}

impl<A: Write, B: Write> MultiWriter<A, B> {
    fn new(diagnostic: A, primary: B) -> Self {
        Self {
            diagnostic,
            primary,
            diagnostic_failed: false,
            primary_failed: false,
        }
    }
}

impl<A: Write, B: Write> Write for MultiWriter<A, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.diagnostic_failed |= self.diagnostic.write_all(buf).is_err();
        self.primary_failed |= self.primary.write_all(buf).is_err();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.diagnostic_failed |= self.diagnostic.flush().is_err();
        self.primary_failed |= self.primary.flush().is_err();
        Ok(())
    }
}

#[derive(Default)]
struct BuildCaptureOutcome {
    diagnostic_failed: bool,
    primary_failed: bool,
}

fn drain_build_stderr(
    reader: &mut impl io::Read,
    diagnostic: &mut impl Write,
    primary: &mut impl Write,
) -> BuildCaptureOutcome {
    let mut sink = MultiWriter::new(diagnostic, primary);
    let read_failed = io::copy(reader, &mut sink).is_err();
    let _ = sink.flush();
    BuildCaptureOutcome {
        diagnostic_failed: sink.diagnostic_failed,
        primary_failed: read_failed || sink.primary_failed,
    }
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
        |path| std::fs::read_to_string(path),
        |path, body| std::fs::write(path, body),
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
    status: &std::process::ExitStatus,
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
    status: &'a std::process::ExitStatus,
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
    diagnostic_failed |= rescue();
    report_build_diagnostic_failure(diagnostic_failed, stderr);
    StepResult::fail(step_name).detail(failure_detail(
        installable,
        status,
        excerpt.as_deref(),
        reliable_log_path,
    ))
}

/// `nix build -L --keep-failed --accept-flake-config --out-link .xtask/gcroots/<check> .#checks.<system>.<check>`,
/// fanning the `-L` build log to both the live terminal and
/// `.xtask/diagnostics/<check>/build.log` (gitignored; uploaded by ci.yml's
/// `validate-diagnostics` artifact). On failure a completely captured log is
/// named in the `StepResult`; partial/unavailable diagnostic paths are omitted.
/// --accept-flake-config honors the jaunder-org cachix substituter for the
/// untrusted local user; --out-link makes the closure a GC root.
fn build_check(step_name: &str, check: &str) -> StepResult {
    let start = std::time::Instant::now();
    let mut diagnostic_failed = match prepare_build_dirs_with(
        || std::fs::create_dir_all(".xtask/gcroots"),
        || std::fs::create_dir_all(format!(".xtask/diagnostics/{check}")),
    ) {
        Ok(failed) => failed,
        Err(error) => {
            return StepResult::fail(step_name)
                .detail(format!("creating .xtask/gcroots: {error}"))
                .with_duration(start.elapsed());
        }
    };
    let out_link = format!(".xtask/gcroots/{check}");
    let installable = format!(".#checks.{SYSTEM}.{check}");
    let log_dir = format!(".xtask/diagnostics/{check}");
    let log_path = format!("{log_dir}/build.log");

    let mut child = match Command::new("nix")
        .args([
            "build",
            "-L",
            "--keep-failed",
            "--log-lines",
            NIX_ERROR_TAIL_LINES,
            "--accept-flake-config",
            "--out-link",
            &out_link,
            &installable,
        ])
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            report_build_diagnostic_failure(diagnostic_failed, &mut io::stderr());
            return StepResult::fail(step_name)
                .detail(error.to_string())
                .with_duration(start.elapsed());
        }
    };

    let mut primary_output_failed = false;
    if let Some(mut stderr_pipe) = child.stderr.take() {
        let outcome = match File::create(&log_path) {
            Ok(mut file) => drain_build_stderr(&mut stderr_pipe, &mut file, &mut io::stderr()),
            Err(_) => {
                diagnostic_failed = true;
                drain_build_stderr(&mut stderr_pipe, &mut io::sink(), &mut io::stderr())
            }
        };
        diagnostic_failed |= outcome.diagnostic_failed;
        primary_output_failed |= outcome.primary_failed;
    }
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            report_build_diagnostic_failure(diagnostic_failed, &mut io::stderr());
            return StepResult::fail(step_name)
                .detail(error.to_string())
                .with_duration(start.elapsed());
        }
    };
    if primary_output_failed {
        report_build_diagnostic_failure(diagnostic_failed, &mut io::stderr());
        return StepResult::fail(step_name)
            .detail("failed to stream nix build stderr")
            .with_duration(start.elapsed());
    }
    if status.success() {
        report_build_diagnostic_failure(diagnostic_failed, &mut io::stderr());
        StepResult::ok(step_name).with_duration(start.elapsed())
    } else {
        failed_build_after_diagnostics_with(
            FailedBuildDiagnostics {
                step_name,
                installable: &installable,
                status: &status,
                log_path: &log_path,
                capture_failed: diagnostic_failed,
            },
            || write_failure_excerpt(&log_path),
            || rescue_diagnostics(check),
            &mut io::stderr(),
        )
        .with_duration(start.elapsed())
    }
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

fn eval_out_path(check: &str) -> Result<String> {
    nix_eval_raw(None, &format!(".#checks.{SYSTEM}.{check}.outPath"))
}

pub fn eval_coverage_drvpath(flake_dir: &Path) -> Result<String> {
    nix_eval_raw(
        Some(flake_dir),
        &format!(".#checks.{SYSTEM}.coverage.drvPath"),
    )
}

/// On a failed build, best-effort copy diagnostics from retained outputs.
/// Returns whether any secondary recovery step failed so the build owner can
/// aggregate one warning without changing the primary failure.
fn rescue_diagnostics(check: &str) -> bool {
    let dest = format!(".xtask/diagnostics/{check}");
    let mut failed = std::fs::create_dir_all(&dest).is_err();
    if check.starts_with("e2e") {
        match eval_out_path(check) {
            Ok(out_path) => {
                let outcome = copy_e2e_diagnostics_between(Path::new(&out_path), Path::new(&dest));
                failed |= !outcome.failures.is_empty();
            }
            Err(_) => failed = true,
        }
    }
    let prefix = format!("nix-build-jaunder-{check}");
    let entries = match std::fs::read_dir("/tmp") {
        Ok(entries) => entries,
        Err(_) => return true,
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
        let src = entry.path().join("emit-out/diagnostics");
        if src.is_dir() {
            failed |= !matches!(
                Command::new("cp")
                    .arg("-r")
                    .arg(&src)
                    .arg(&dest)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status(),
                Ok(status) if status.success()
            );
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::{
        CommandResult, E2E_COMBOS, FailedBuildDiagnostics, StepResult,
        check_supporting_test_check_names, doctest_sentinel_detail, drain_build_stderr,
        failed_build_after_diagnostics_with, failed_status_step, finish_e2e_combo,
        prepare_build_dirs_with, report_build_diagnostic_failure, route_e2e_combos,
        sentinel_detail, test_check_names, validate_check_names,
    };
    use coverage::status::{CoverageStatus, StatusCategory};
    use doctests::check::{Kind, Violation};
    use doctests::status::DoctestStatus;

    #[test]
    fn nix_test_check_names_include_wasm_tests() {
        assert!(test_check_names(false).eq(["wasm-tests", "coverage", "doctests"]));
    }

    #[test]
    fn check_supporting_test_check_names_exclude_coverage() {
        assert!(check_supporting_test_check_names().eq(["wasm-tests", "doctests"]));
    }

    #[test]
    fn validate_check_names_include_static_checks_before_test_checks() {
        assert!(validate_check_names().eq(["static-checks", "wasm-tests", "coverage", "doctests"]));
    }

    #[test]
    fn nix_test_check_names_omit_all_for_no_test() {
        assert!(test_check_names(true).next().is_none());
    }

    #[test]
    fn doctest_sentinel_detail_names_each_violation() {
        let s = DoctestStatus::from_violations(vec![Violation {
            file: "common/src/token.rs".to_string(),
            line: 56,
            kind: Kind::NotRun,
            detail: "scanned but never evaluated".to_string(),
        }]);
        let d = doctest_sentinel_detail(&s);
        assert!(d.contains("common/src/token.rs:56"), "{d}");
        // The kebab-case spelling, so this reads the same as the gate's jq output.
        assert!(d.contains("[not-run]"), "{d}");
        assert!(d.contains("scanned but never evaluated"), "{d}");
    }

    #[test]
    fn doctest_sentinel_detail_is_terse_when_ok() {
        let s = DoctestStatus::from_violations(Vec::new());
        assert_eq!(doctest_sentinel_detail(&s), "in-sandbox: doctests ok");
    }

    #[test]
    fn doctest_sentinel_detail_reports_an_infra_failure_as_such() {
        let s = DoctestStatus::infra("could not spawn cargo");
        let d = doctest_sentinel_detail(&s);
        assert!(d.contains("could not spawn cargo"), "{d}");
        assert!(!d.contains("violation"), "{d}");
    }

    #[test]
    fn infra_detail_is_labeled_as_infrastructure() {
        let s = CoverageStatus {
            category: StatusCategory::Infra,
            failed_tests: vec![],
            infra_detail: Some("No space left on device".into()),
        };
        let d = sentinel_detail(&s);
        assert!(d.contains("infrastructure failure"));
        assert!(d.contains("No space left on device"));
    }

    #[test]
    fn test_failure_lists_tests_and_disclaims_coverage() {
        let s = CoverageStatus {
            category: StatusCategory::TestFailure,
            failed_tests: vec!["web_posts::case_3".into()],
            infra_detail: None,
        };
        let d = sentinel_detail(&s);
        assert!(d.contains("test failure"));
        assert!(d.contains("web_posts::case_3"));
    }

    fn assert_status_attempt_warns_once<T>(
        warning_key: &str,
        fallback: &str,
        parse: fn(&str) -> anyhow::Result<T>,
        render: fn(&T) -> String,
    ) {
        for raw in [None, Some("{malformed")] {
            let mut stderr = Vec::new();
            let mut result = CommandResult::new("nix-status");
            result.push(failed_status_step(
                "gate",
                warning_key,
                fallback,
                || match raw {
                    Some(raw) => Ok(raw.to_owned()),
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "injected",
                    )),
                },
                parse,
                render,
                &mut stderr,
            ));
            let mut expected = CommandResult::new("nix-status");
            expected.push(StepResult::fail("gate").detail(fallback));
            assert_eq!(
                serde_json::to_string(&result).unwrap(),
                serde_json::to_string(&expected).unwrap()
            );
            let warning = String::from_utf8(stderr).unwrap();
            assert_eq!(warning.matches(warning_key).count(), 1);
            assert_eq!(warning.lines().count(), 1);
            assert!(!warning.contains("injected"));
            assert!(!warning.contains("{malformed"));
        }
    }

    #[test]
    fn failed_coverage_status_attempts_preserve_fallback_and_warn_once() {
        assert_status_attempt_warns_once(
            "xtask.nix.coverage_status",
            "coverage gate failed (no status.json)",
            CoverageStatus::from_json,
            sentinel_detail,
        );
    }

    #[test]
    fn failed_doctest_status_attempts_preserve_fallback_and_warn_once() {
        assert_status_attempt_warns_once(
            "xtask.nix.doctest_status",
            "doctest gate failed (no status.json)",
            DoctestStatus::from_json,
            doctest_sentinel_detail,
        );
    }

    #[test]
    fn failed_e2e_out_path_lookup_aggregates_with_build_diagnostics() {
        let status = std::process::Command::new("false").status().unwrap();
        let mut stderr = Vec::new();
        let result = failed_build_after_diagnostics_with(
            FailedBuildDiagnostics {
                step_name: "nix-e2e",
                installable: ".#checks.x86_64-linux.e2e",
                status: &status,
                log_path: ".xtask/diagnostics/e2e/build.log",
                capture_failed: false,
            },
            || Ok(".xtask/diagnostics/e2e/failure-excerpt.log".to_owned()),
            || {
                let failed_eval: anyhow::Result<String> =
                    Err(anyhow::anyhow!("injected sensitive nix eval failure"));
                failed_eval.is_err()
            },
            &mut stderr,
        );
        assert!(!result.ok);
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("xtask.nix.build_diagnostics").count(), 1);
        assert_eq!(warning.lines().count(), 1);
        assert!(!warning.contains("sensitive"));
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

    fn failing_sink() -> impl Write {
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

    #[test]
    fn build_capture_fans_full_input_out_to_both_sinks() {
        let input = vec![b'x'; 200_000];
        let mut diagnostic = Vec::new();
        let mut primary = Vec::new();
        let mut reader: &[u8] = &input;
        let outcome = drain_build_stderr(&mut reader, &mut diagnostic, &mut primary);

        assert!(!outcome.diagnostic_failed);
        assert!(!outcome.primary_failed);
        assert_eq!(diagnostic, input);
        assert_eq!(primary, input);
    }

    #[test]
    fn build_capture_diagnostic_failure_warns_once_without_changing_primary_output() {
        let input = vec![b'y'; 200_000];
        let mut primary = Vec::new();
        let mut reader: &[u8] = &input;
        let outcome = drain_build_stderr(&mut reader, &mut failing_sink(), &mut primary);
        let mut stderr = Vec::new();
        report_build_diagnostic_failure(outcome.diagnostic_failed, &mut stderr);

        assert!(!outcome.primary_failed);
        assert_eq!(primary, input);
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("xtask.nix.build_diagnostics").count(), 1);
        assert_eq!(warning.lines().count(), 1);
        assert!(!warning.contains("sensitive"));
    }

    #[test]
    fn build_capture_primary_failure_is_not_silenced() {
        let input = b"complete build output";
        let mut diagnostic = Vec::new();
        let mut reader: &[u8] = input;
        let outcome = drain_build_stderr(&mut reader, &mut diagnostic, &mut failing_sink());

        assert!(!outcome.diagnostic_failed);
        assert!(outcome.primary_failed);
        assert_eq!(diagnostic, input);
    }
    #[test]
    fn build_capture_read_failure_fails_the_primary_output_contract() {
        struct FailingReader(bool);
        impl io::Read for FailingReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    Err(io::Error::other("sensitive read failure"))
                } else {
                    self.0 = true;
                    buf[..4].copy_from_slice(b"part");
                    Ok(4)
                }
            }
        }

        let mut diagnostic = Vec::new();
        let mut primary = Vec::new();
        let outcome = drain_build_stderr(&mut FailingReader(false), &mut diagnostic, &mut primary);
        assert!(outcome.primary_failed);
        assert_eq!(diagnostic, b"part");
        assert_eq!(primary, b"part");
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
    fn e2e_vm_captures_report_and_manifest_before_asserting_playwright_status() {
        let flake = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("flake.nix"),
        )
        .expect("flake.nix");
        let report_copy =
            "cp /tmp/e2e/test-results/results.json /tmp/playwright-report-${backend}.json";
        let report_grab = r#"_grab("/tmp/playwright-report-${backend}.json")"#;
        let manifest_copy = "cp /tmp/e2e/test-results/duration-budget-manifest.json \
                             /tmp/duration-budget-manifest-${backend}.json";
        let manifest_grab = r#"_grab("/tmp/duration-budget-manifest-${backend}.json")"#;
        let assertion = "assert pw_status == 0";

        let report_copy_at = flake.find(report_copy).expect("report is copied");
        let report_grab_at = flake.find(report_grab).expect("report is lifted");
        let manifest_copy_at = flake.find(manifest_copy).expect("manifest is copied");
        let manifest_grab_at = flake.find(manifest_grab).expect("manifest is lifted");
        let assertion_at = flake
            .find(assertion)
            .expect("Playwright status is asserted");

        assert!(report_copy_at < report_grab_at && report_grab_at < assertion_at);
        assert!(manifest_copy_at < manifest_grab_at && manifest_grab_at < assertion_at);
    }

    #[test]
    fn copy_e2e_diagnostics_between_copies_journal_capture_playwright_and_manifest() {
        let tmp = std::env::temp_dir().join(format!("xtask-j-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("jaunder-journal-sqlite.log"), b"j").unwrap();
        std::fs::write(src.join("playwright-report-sqlite.json"), b"p").unwrap();
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
            outcome.copied, 6,
            "journal + report + manifest + artifacts tarball + system journal + capture tarball \
             are copied; unrelated and in-tree artifact names are not"
        );
        assert!(dest.join("jaunder-journal-sqlite.log").exists());
        assert!(dest.join("playwright-report-sqlite.json").exists());
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
        // A directory squatting on the destination name deterministically makes
        // replacement fail without depending on host permissions.
        let tmp = std::env::temp_dir().join(format!("xtask-copy-fail-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(dest.join("capture-sqlite.tar.gz")).unwrap();
        std::fs::write(src.join("capture-sqlite.tar.gz"), b"new").unwrap();
        let outcome = super::copy_e2e_diagnostics_between(&src, &dest);
        assert_eq!(outcome.copied, 0);
        assert_eq!(outcome.failures.len(), 1, "{:?}", outcome.failures);
        assert!(outcome.failures[0].contains("capture-sqlite.tar.gz"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ancillary_warning_e2e_diagnostic_failures_warn_once_per_attempt() {
        let tmp = std::env::temp_dir().join(format!("xtask-nix-lift-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("capture-sqlite.tar.gz"), b"new").unwrap();
        let primary = StepResult::fail("nix-e2e").detail("original build failure");
        let before = serde_json::to_string(&primary).unwrap();
        for failed_operation in ["remove", "copy", "permissions"] {
            let mut stderr = Vec::new();
            let _ = super::lift_e2e_diagnostics_with_ops(
                &src,
                &dest,
                |_| {
                    if failed_operation == "remove" {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "sensitive remove failure",
                        ))
                    } else {
                        Ok(())
                    }
                },
                |_, _| {
                    if failed_operation == "copy" {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "sensitive copy failure",
                        ))
                    } else {
                        Ok(1)
                    }
                },
                |_, _| {
                    if failed_operation == "permissions" {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "sensitive permission failure",
                        ))
                    } else {
                        Ok(())
                    }
                },
                &mut stderr,
            );
            let warning = String::from_utf8(stderr).unwrap();
            assert_eq!(warning.matches("xtask.nix.e2e_diagnostics").count(), 1);
            assert_eq!(warning.lines().count(), 1);
            assert!(!warning.contains("sensitive"));
            assert_eq!(serde_json::to_string(&primary).unwrap(), before);
        }
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

    #[test]
    fn aggregate_route_addresses_every_named_browser_combo() {
        let mut routed = Vec::new();
        route_e2e_combos(E2E_COMBOS, |backend, browser| {
            routed.push((backend.to_owned(), browser.to_owned()))
        });
        assert_eq!(
            routed,
            E2E_COMBOS.map(|(backend, browser)| (backend.to_owned(), browser.to_owned()))
        );
    }

    #[test]
    fn successful_combo_lifts_before_dispatching_duration_validator() {
        let mut result = CommandResult::new("e2e-sqlite-chromium");
        let events = std::cell::RefCell::new(Vec::new());
        finish_e2e_combo(
            &mut result,
            StepResult::ok("nix-e2e-sqlite-chromium"),
            || events.borrow_mut().push("lift"),
            || {
                events.borrow_mut().push("validate");
                StepResult::ok("e2e-duration-budget")
            },
        );

        assert_eq!(events.into_inner(), ["lift", "validate"]);
        assert!(result.ok);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            ["nix-e2e-sqlite-chromium", "e2e-duration-budget"]
        );
    }

    #[test]
    fn failed_combo_keeps_diagnostics_but_does_not_mask_primary_failure() {
        let mut result = CommandResult::new("e2e-sqlite-chromium");
        let lifted = std::cell::Cell::new(false);
        finish_e2e_combo(
            &mut result,
            StepResult::fail("nix-e2e-sqlite-chromium"),
            || lifted.set(true),
            || panic!("a failed VM must not dispatch duration validation"),
        );

        assert!(lifted.get());
        assert!(!result.ok);
        assert_eq!(result.steps.len(), 1);
    }
}
