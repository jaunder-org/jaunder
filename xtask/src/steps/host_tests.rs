use xshell::Shell;

use crate::result::CommandResult;
use crate::sh::step;

/// Run the host-only workspace unit tests that no Nix derivation covers. xtask is
/// its own workspace, excluded from every Nix check, so without this its tests
/// gate nowhere. Fast host suite — runs in every mode (it is NOT the heavy Nix
/// instrumented suite that `--no-test` / `--no-e2e` skip). No coverage here.
pub fn run(sh: &Shell, result: &mut CommandResult) {
    result.push(step(
        sh,
        "xtask-tests",
        "cargo",
        &["test", "--manifest-path", "xtask/Cargo.toml"],
    ));
}
