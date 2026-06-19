use std::process::Command;

use crate::result::{CommandResult, StepResult};

/// The flake checks are Linux-only (`optionalAttrs isLinux` in flake.nix);
/// the project's CI host is x86_64-linux.
const SYSTEM: &str = "x86_64-linux";

pub fn run(full: bool, result: &mut CommandResult) {
    result.push(build_check("nix-coverage", "coverage"));
    if full {
        result.push(build_check("nix-e2e-sqlite", "e2e-sqlite"));
        result.push(build_check("nix-e2e-postgres", "e2e-postgres"));
        result.push(build_check(
            "nix-postgres-integration",
            "postgres-integration",
        ));
    }
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
