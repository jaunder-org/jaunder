use xshell::Shell;

use crate::result::CommandResult;
use crate::sh::step;

/// Run fast host-side unit tests for auxiliary workspaces whose unit suites are
/// not executed by root application coverage or Nix test gates. Runs in every
/// mode; it is NOT the heavy Nix instrumented suite that `--no-test` /
/// `--no-e2e` skip. No coverage here.
pub fn run(sh: &Shell, result: &mut CommandResult) {
    result.push(step(
        sh,
        "xtask-tests",
        "cargo",
        &["test", "--manifest-path", "xtask/Cargo.toml"],
    ));
    // `tools/` (devtool + coverage + doctests) is its own auxiliary virtual
    // workspace. Static checks may still build or use tool crates, but they do
    // not execute this workspace's unit suite; `tools-test` is the compensating
    // host gate.
    result.push(step(
        sh,
        "tools-test",
        "cargo",
        &["test", "--manifest-path", "tools/Cargo.toml"],
    ));
}
