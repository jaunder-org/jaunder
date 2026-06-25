use std::process::Command;

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

/// The e2e gate: build the `e2e` aggregate check, which depends on both
/// backend VM checks. They are independent derivations, so the host realizes
/// them in parallel up to its `max-jobs` — CI's install-nix-action sets
/// `max-jobs = auto`; a plain dev box defaults to 1 and runs them serially.
/// The "run both backends in parallel" intent is declared in the flake (the
/// `e2e-checks` aggregate), not here. `postgres-integration` is deliberately
/// not dispatched — its tests already run under the coverage check.
pub fn e2e(result: &mut CommandResult) {
    result.push(build_check("nix-e2e", "e2e"));
}

/// `nix build --accept-flake-config --out-link .xtask/gcroots/<check> .#checks.<system>.<check>`.
/// --accept-flake-config honors the jaunder-org cachix substituter for the
/// untrusted local user; --out-link makes the closure a GC root.
fn build_check(step_name: &str, check: &str) -> StepResult {
    let _ = std::fs::create_dir_all(".xtask/gcroots");
    let out_link = format!(".xtask/gcroots/{check}");
    let installable = format!(".#checks.{SYSTEM}.{check}");
    let status = Command::new("nix")
        .args([
            "build",
            // Retain the failed build dir so a catastrophic in-sandbox failure
            // (e.g. ENOSPC that prevented writing `$out`) still leaves
            // first-hand data; `rescue_diagnostics` then copies it out.
            "--keep-failed",
            "--accept-flake-config",
            "--out-link",
            &out_link,
            &installable,
        ])
        .status();
    match status {
        Ok(s) if s.success() => StepResult::ok(step_name),
        Ok(s) => {
            rescue_diagnostics(check);
            StepResult::fail(step_name).detail(format!("nix build {installable} exited with {s}"))
        }
        Err(e) => StepResult::fail(step_name).detail(e.to_string()),
    }
}

/// On a failed `nix build`, best-effort copy any diagnostics bundle from the
/// retained (`--keep-failed`) build dir to `.xtask/diagnostics/<check>/`, so a
/// catastrophic in-sandbox failure still leaves first-hand data for inspection
/// and CI artifact upload. Silent on miss — the kept build dir remains either way.
fn rescue_diagnostics(check: &str) {
    let dest = format!(".xtask/diagnostics/{check}");
    let _ = std::fs::create_dir_all(&dest);
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
}
