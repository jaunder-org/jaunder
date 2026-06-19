use std::process::Command;

use crate::coverage;
use crate::result::{CommandResult, Mode, StepResult};

/// The flake checks are Linux-only (`optionalAttrs isLinux` in flake.nix);
/// the project's CI host is x86_64-linux.
const SYSTEM: &str = "x86_64-linux";

/// The Nix coverage check: the instrumented test suite (SQLite- and
/// PostgreSQL-backed tests together in one pass under an ephemeral PostgreSQL)
/// emits the reports; the regression gate + auto-heal then runs host-side over
/// the check's `$out`.
pub fn coverage(result: &mut CommandResult, mode: Mode) {
    let build = build_check("nix-coverage", "coverage");
    if !build.ok {
        // The instrumented suite (or the report emit) failed — there is no
        // usable `$out` to post-process. Record the failed build only.
        result.push(build);
        return;
    }
    result.push(build);
    let (step, report) = coverage::run(".xtask/gcroots/coverage", mode);
    result.push(step);
    result.coverage = report;
}

/// The e2e VM checks (both backends). `postgres-integration` is deliberately
/// not dispatched — its tests already run under the coverage check.
pub fn e2e(result: &mut CommandResult) {
    result.push(build_check("nix-e2e-sqlite", "e2e-sqlite"));
    result.push(build_check("nix-e2e-postgres", "e2e-postgres"));
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
            "--accept-flake-config",
            "--out-link",
            &out_link,
            &installable,
        ])
        .status();
    match status {
        Ok(s) if s.success() => StepResult::ok(step_name),
        Ok(s) => {
            StepResult::fail(step_name).detail(format!("nix build {installable} exited with {s}"))
        }
        Err(e) => StepResult::fail(step_name).detail(e.to_string()),
    }
}
