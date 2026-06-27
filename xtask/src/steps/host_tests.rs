use xshell::Shell;

use crate::result::CommandResult;
use crate::sh::step;

/// Run the host-only workspace unit tests that no Nix derivation covers. `xtask`
/// and the `tools/` workspace are each excluded from every Nix check, so without
/// this their tests gate nowhere. Fast host suite — runs in every mode (it is NOT
/// the heavy Nix instrumented suite that `--no-test` / `--no-e2e` skip). No
/// coverage here.
pub fn run(sh: &Shell, result: &mut CommandResult) {
    result.push(step(
        sh,
        "xtask-tests",
        "cargo",
        &["test", "--manifest-path", "xtask/Cargo.toml"],
    ));
    // `tools/` (devtool + coverage) is its own virtual workspace, excluded from
    // the coverage check's source and built `doCheck = false`, so its unit tests —
    // devtool's pg/coverage logic — would otherwise only be compiled by clippy,
    // never executed.
    result.push(step(
        sh,
        "tools-test",
        "cargo",
        &["test", "--manifest-path", "tools/Cargo.toml"],
    ));
}
