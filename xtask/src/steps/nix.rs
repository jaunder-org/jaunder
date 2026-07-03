use std::fs::File;
use std::io::{self, Write};
use std::process::{Command, Stdio};

use crate::result::{CommandResult, Mode, StepResult};

/// The flake checks are Linux-only (`optionalAttrs isLinux` in flake.nix);
/// the project's CI host is x86_64-linux.
const SYSTEM: &str = "x86_64-linux";

/// The Nix coverage check: the instrumented test suite (SQLite- and
/// PostgreSQL-backed tests together in one pass under an ephemeral PostgreSQL)
/// emits the reports; the regression gate + auto-heal then runs host-side over
/// the check's `$out`.
pub fn coverage(result: &mut CommandResult, mode: Mode) {
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
        let detail = std::fs::read_to_string(status_path)
            .ok()
            .and_then(|s| coverage::status::CoverageStatus::from_json(&s).ok())
            .map(|s| sentinel_detail(&s))
            .unwrap_or_else(|| "coverage gate failed (no status.json)".to_string());
        result.push(StepResult::fail("coverage").detail(detail));
        return;
    }
    result.push(gate);
    // `crate::coverage` is xtask's host-side gate module; `coverage` (no
    // `crate::`) is the shared crate holding the sentinel schema.
    let (step, report) = crate::coverage::run(".xtask/gcroots/coverage", mode);
    result.push(step);
    result.coverage = report;
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

/// The e2e gate: build the `e2e` aggregate check, which joins all four
/// {sqlite,postgres}×{chromium,firefox} combo VM checks. They are independent
/// derivations, so the host realizes them in parallel up to its `max-jobs` —
/// CI's install-nix-action sets `max-jobs = auto`; a plain dev box defaults to 1
/// and runs them serially. Since the workers=2 flip (#155) each combo VM is
/// sized small (cores=2, 3 GB) and runs only 2 Playwright workers, so several
/// realize concurrently without oversubscribing a typical multi-core host — the
/// fan-out is left to the host's `max-jobs` rather than pinned here. The intent
/// is declared in the flake (`e2e-checks` aggregate / `e2eCombos`). This
/// aggregate path is the full LOCAL `validate` equivalent; CI instead fans the
/// combos across runners via `cargo xtask e2e` (see `e2e_combo`).
/// `postgres-integration` is deliberately not dispatched — its tests already run
/// under the coverage check.
pub fn e2e(result: &mut CommandResult) {
    let step = build_check("nix-e2e", "e2e");
    // #93: surface the server journals in the one canonical, always-uploaded
    // diagnostics dir, regardless of cache-hit/pass/fail. The aggregate
    // symlinkJoin collapses same-named outputs, so this captures one journal per
    // BACKEND (the two browser combos emit the same `jaunder-journal-<backend>.log`),
    // not per combo; per-combo fidelity is on the `cargo xtask e2e` path. Best-
    // effort: a failed e2e derivation produces no out-link, but its panic is
    // already in build.log (the `-L` stream + the gate's assertion message).
    copy_e2e_journals();
    result.push(step);
}

/// Build a single e2e {backend}×{browser} combo check via `build_check` (so the
/// `nix build -L --keep-failed` log + `rescue_diagnostics` failure bundle land in
/// `.xtask/diagnostics/e2e-<backend>-<browser>/`), then copy that combo's journal
/// into the canonical diagnostics dir. Used by CI's e2e matrix.
pub fn e2e_combo(result: &mut CommandResult, backend: &str, browser: &str) {
    let check = format!("e2e-{backend}-{browser}");
    let step_name = format!("nix-{check}");
    result.push(build_check(&step_name, &check));
    copy_e2e_diagnostics_between(
        std::path::Path::new(&format!(".xtask/gcroots/{check}")),
        std::path::Path::new(&format!(".xtask/diagnostics/{check}")),
    );
}

/// Runs the hermetic elisp live-integration `nixosTest` check (ADR-0035): a NixOS
/// VM with Emacs + the jaunder binary, where the harness self-boots the server.
/// The `e2e-elisp-integration` check also joins the `e2e-checks` aggregate, so
/// local `validate` realizes it in parallel via `e2e`; this dedicated builder is
/// the per-job CI path (`cargo xtask elisp-integration`), mirroring `e2e_combo`.
pub fn elisp_integration(result: &mut CommandResult) {
    let check = "e2e-elisp-integration";
    result.push(build_check("nix-elisp-integration", check));
    copy_e2e_diagnostics_between(
        std::path::Path::new(&format!(".xtask/gcroots/{check}")),
        std::path::Path::new(&format!(".xtask/diagnostics/{check}")),
    );
}

/// Copy the realized e2e check's diagnostic files — server journals, OTEL traces,
/// and the Playwright report — into the canonical diagnostics dir. Best-effort;
/// silent on a missing out-link (e.g. a failed build).
fn copy_e2e_journals() {
    copy_e2e_diagnostics_between(
        std::path::Path::new(".xtask/gcroots/e2e"),
        std::path::Path::new(".xtask/diagnostics/e2e"),
    );
}

/// Copy e2e diagnostic files — the app journal (`jaunder-journal-*.log`), the full
/// system journal (`system-journal-*.log`), OTEL traces (`otel-traces-*.jsonl`),
/// the Playwright per-test JSON report (`playwright-report-*.json`), and the
/// Playwright trace/screenshot tarball (`playwright-artifacts-*.tar.gz`) — from
/// `src_dir` into `dest_dir` (created if needed). The system journal and tarball
/// are the #123/#49 failure-path additions. Serves both the success path (from the
/// out-link) and the failure path (from the kept outPath). Returns the count
/// copied. Pure path logic so it is unit-testable.
fn copy_e2e_diagnostics_between(src_dir: &std::path::Path, dest_dir: &std::path::Path) -> usize {
    let wanted = |name: &str| {
        (name.starts_with("jaunder-journal-") && name.ends_with(".log"))
            || (name.starts_with("system-journal-") && name.ends_with(".log"))
            || (name.starts_with("otel-traces-") && name.ends_with(".jsonl"))
            || (name.starts_with("playwright-report-") && name.ends_with(".json"))
            || (name.starts_with("playwright-artifacts-") && name.ends_with(".tar.gz"))
    };
    let Ok(entries) = std::fs::read_dir(src_dir) else {
        return 0;
    };
    let _ = std::fs::create_dir_all(dest_dir);
    let mut copied = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !wanted(name) {
            continue;
        }
        let from = entry.path();
        let to = dest_dir.join(name);
        // OTEL traces arrive as a directory (`otel-traces-<backend>.jsonl/otel-traces.jsonl`,
        // an intentional layout the trace-analysis tooling depends on); journals and the
        // Playwright report are flat files. Handle both.
        let ok = if from.is_dir() {
            copy_tree(&from, &to).is_ok()
        } else {
            std::fs::copy(&from, &to).is_ok()
        };
        if ok {
            copied += 1;
        }
    }
    copied
}

/// Recursively copy the directory tree at `from` to `to` (creating `to` and any
/// parents). Pure std I/O so it stays unit-testable; used for the OTEL trace
/// directory artifact.
fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dst)?;
        } else {
            std::fs::copy(entry.path(), dst)?;
        }
    }
    Ok(())
}

/// A `Write` that fans every write out to two inner writers, **best-effort**: a
/// write or flush error from either sink is swallowed and the whole chunk is
/// always reported consumed. This is deliberate. `build_check` drives it with
/// `io::copy` to drain a child's stderr pipe, and that drain MUST run to EOF even
/// if the log file (or our own stderr) goes unwritable mid-build — otherwise the
/// unread pipe fills and the child blocks forever in `wait()` (the exact
/// disk-pressure case this capture exists to diagnose). Log capture is a
/// diagnostic nicety, never a reason to hang the gate.
struct MultiWriter<A: Write, B: Write>(A, B);

impl<A: Write, B: Write> Write for MultiWriter<A, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = self.0.write_all(buf);
        let _ = self.1.write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.0.flush();
        let _ = self.1.flush();
        Ok(())
    }
}

/// The failure `detail` for a Nix check, naming the installable, the exit status,
/// and the captured build-log path. Pure so it can be unit-tested.
fn failure_detail(installable: &str, status: &std::process::ExitStatus, log_path: &str) -> String {
    format!("nix build {installable} exited with {status}; full build log: {log_path}")
}

/// `nix build -L --keep-failed --accept-flake-config --out-link .xtask/gcroots/<check> .#checks.<system>.<check>`,
/// fanning the `-L` build log to both the live terminal and
/// `.xtask/diagnostics/<check>/build.log` (gitignored; uploaded by ci.yml's
/// `validate-diagnostics` artifact). On failure the saved log path is named in the
/// `StepResult` detail so the failure is diagnosable without a rebuild.
/// --accept-flake-config honors the jaunder-org cachix substituter for the
/// untrusted local user; --out-link makes the closure a GC root.
fn build_check(step_name: &str, check: &str) -> StepResult {
    let _ = std::fs::create_dir_all(".xtask/gcroots");
    let out_link = format!(".xtask/gcroots/{check}");
    let installable = format!(".#checks.{SYSTEM}.{check}");

    let log_dir = format!(".xtask/diagnostics/{check}");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = format!("{log_dir}/build.log");

    let mut child = match Command::new("nix")
        .args([
            "build",
            // -L streams every (transitive) derivation's build log to stderr, so
            // the failing dependency's output is in the stream we capture below.
            "-L",
            // Retain the failed build dir so a catastrophic in-sandbox failure
            // (e.g. ENOSPC that prevented writing `$out`) still leaves first-hand
            // data; `rescue_diagnostics` then copies it out.
            "--keep-failed",
            "--accept-flake-config",
            "--out-link",
            &out_link,
            &installable,
        ])
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return StepResult::fail(step_name).detail(e.to_string()),
    };

    // Drain the piped stderr to both the live terminal and the log file. We must
    // drain it regardless (an undrained full pipe would block the child); if the
    // log file can't be opened we still copy to stderr alone.
    if let Some(mut stderr_pipe) = child.stderr.take() {
        match File::create(&log_path) {
            Ok(file) => {
                let mut sink = MultiWriter(file, io::stderr());
                let _ = io::copy(&mut stderr_pipe, &mut sink);
            }
            Err(_) => {
                let _ = io::copy(&mut stderr_pipe, &mut io::stderr());
            }
        }
    }

    match child.wait() {
        Ok(s) if s.success() => StepResult::ok(step_name),
        Ok(s) => {
            rescue_diagnostics(check);
            StepResult::fail(step_name).detail(failure_detail(&installable, &s, &log_path))
        }
        Err(e) => StepResult::fail(step_name).detail(e.to_string()),
    }
}

/// The check's evaluated output store path. On a failed build `--keep-failed`
/// leaves this path on disk, world-readable, even though it is unregistered — so
/// the e2e diagnostics the VM copied into `$out` are recoverable from it (#123/#49).
/// `None` if the eval fails (e.g. an eval-time error unrelated to the build).
fn eval_out_path(check: &str) -> Option<String> {
    let installable = format!(".#checks.{SYSTEM}.{check}.outPath");
    let out = Command::new("nix")
        .args(["eval", "--raw", "--accept-flake-config", &installable])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_owned())
}

/// On a failed `nix build`, best-effort copy any diagnostics bundle from the
/// retained (`--keep-failed`) build dir to `.xtask/diagnostics/<check>/`, so a
/// catastrophic in-sandbox failure still leaves first-hand data for inspection
/// and CI artifact upload. Silent on miss — the kept build dir remains either way.
fn rescue_diagnostics(check: &str) {
    let dest = format!(".xtask/diagnostics/{check}");
    let _ = std::fs::create_dir_all(&dest);
    // #123/#49: a failed e2e VM check leaves its $out store path on disk
    // (--keep-failed, world-readable) though unregistered. Its deterministic path
    // is the evaluated outPath; recover the copied-out diagnostics from it, reusing
    // the success-path copier (which handles the otel directory layout). A no-op for
    // non-e2e checks — their outPath carries no matching files.
    if let Some(out_path) = eval_out_path(check) {
        copy_e2e_diagnostics_between(std::path::Path::new(&out_path), std::path::Path::new(&dest));
    }
    // Resolve the kept-build-dir glob in Rust and copy with explicit `cp` args
    // (no `bash -c`) so the check name can never inject into a shell command.
    // The `emit-out/diagnostics` is_dir guard skips false prefix matches (e.g. a
    // `coverage-gate` dir scanned for the `coverage` rescue — gate has no bundle).
    let prefix = format!("nix-build-jaunder-{check}");
    let Ok(entries) = std::fs::read_dir("/tmp") else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let src = entry.path().join("emit-out/diagnostics");
        if src.is_dir() {
            let _ = Command::new("cp").arg("-r").arg(&src).arg(&dest).status();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sentinel_detail;
    use coverage::status::{CoverageStatus, StatusCategory};

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

    use super::{failure_detail, MultiWriter};

    #[test]
    fn multiwriter_fans_full_input_out_to_both_sinks() {
        // Larger than io::copy's internal buffer (8 KiB) so the input spans
        // multiple write() calls — proves we don't assume a single chunk.
        let input = vec![b'x'; 200_000];
        let mut a: Vec<u8> = Vec::new();
        let mut b: Vec<u8> = Vec::new();
        {
            let mut sink = MultiWriter(&mut a, &mut b);
            let mut reader: &[u8] = &input;
            std::io::copy(&mut reader, &mut sink).unwrap();
        }
        assert_eq!(a, input);
        assert_eq!(b, input);
    }

    #[test]
    fn multiwriter_keeps_draining_when_a_sink_errors() {
        // A sink that always errors must NOT abort the io::copy drain — otherwise a
        // mid-build log-write failure (e.g. disk full) would leave the child's
        // stderr pipe unread and hang `wait()`. The healthy sink still gets it all.
        struct FailingSink;
        impl std::io::Write for FailingSink {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("sink full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("sink full"))
            }
        }
        let input = vec![b'y'; 200_000];
        let mut healthy: Vec<u8> = Vec::new();
        {
            let mut sink = MultiWriter(FailingSink, &mut healthy);
            let mut reader: &[u8] = &input;
            // Completes to EOF and does not error despite the failing sink.
            std::io::copy(&mut reader, &mut sink).unwrap();
        }
        assert_eq!(healthy, input);
    }

    #[test]
    fn failure_detail_names_installable_status_and_log_path() {
        // `false` exits non-zero, giving a real failed ExitStatus to format.
        let status = std::process::Command::new("false").status().unwrap();
        let d = failure_detail(
            ".#checks.x86_64-linux.e2e",
            &status,
            ".xtask/diagnostics/e2e/build.log",
        );
        assert!(d.contains(".#checks.x86_64-linux.e2e"));
        assert!(d.contains("exited with"));
        assert!(d.contains("full build log: .xtask/diagnostics/e2e/build.log"));
    }

    #[test]
    fn copy_e2e_diagnostics_between_copies_journal_otel_and_playwright() {
        let tmp = std::env::temp_dir().join(format!("xtask-j-{}", std::process::id()));
        let src = tmp.join("src");
        let dest = tmp.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("jaunder-journal-sqlite.log"), b"j").unwrap();
        std::fs::write(src.join("playwright-report-sqlite.json"), b"p").unwrap();
        // #123/#49 failure-path artifacts: the trace/screenshot tarball and the
        // full system journal, copied out of the VM before the check is failed.
        std::fs::write(src.join("playwright-artifacts-sqlite.tar.gz"), b"a").unwrap();
        std::fs::write(src.join("system-journal-sqlite.log"), b"s").unwrap();
        std::fs::write(src.join("unrelated.txt"), b"x").unwrap();
        // OTEL traces arrive as a directory (the load-bearing
        // `otel-traces-<backend>.jsonl/otel-traces.jsonl` layout), not a flat file.
        std::fs::create_dir_all(src.join("otel-traces-sqlite.jsonl")).unwrap();
        std::fs::write(
            src.join("otel-traces-sqlite.jsonl")
                .join("otel-traces.jsonl"),
            b"o",
        )
        .unwrap();

        let n = super::copy_e2e_diagnostics_between(&src, &dest);

        assert_eq!(
            n, 5,
            "journal + otel dir + playwright report + artifacts tarball + system journal are copied; unrelated is not"
        );
        assert!(dest.join("jaunder-journal-sqlite.log").exists());
        assert!(dest.join("playwright-report-sqlite.json").exists());
        assert!(dest.join("playwright-artifacts-sqlite.tar.gz").exists());
        assert!(dest.join("system-journal-sqlite.log").exists());
        assert!(dest
            .join("otel-traces-sqlite.jsonl")
            .join("otel-traces.jsonl")
            .exists());
        assert!(!dest.join("unrelated.txt").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
