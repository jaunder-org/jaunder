use xshell::Shell;

use crate::result::{CommandResult, Mode};
use crate::sh::step;

/// Run the static check suite. In `Mode::Fix`, formatting commands auto-fix
/// in place (matching `scripts/verify`'s check mode but writing changes).
/// In `Mode::Check`, every command is read-only — safe for CI.
///
/// Command invocations are kept verbatim with `scripts/verify` Phase 1 + 2,
/// adjusted only for the Fix/Check switch on the formatting tools.
pub fn run(sh: &Shell, mode: Mode, result: &mut CommandResult) {
    // cargo fmt — scripts/verify uses `cargo fmt --check` (no --all)
    let fmt_args: &[&str] = match mode {
        Mode::Check => &["fmt", "--check"],
        Mode::Fix => &["fmt"],
    };
    result.push(step(sh, "fmt", "cargo", fmt_args));

    // leptosfmt — scripts/verify: leptosfmt -x .direnv -x .git -x target --check '**/*.rs'
    let leptos_args: &[&str] = match mode {
        Mode::Check => &[
            "-x", ".direnv", "-x", ".git", "-x", "target", "--check", "**/*.rs",
        ],
        Mode::Fix => &["-x", ".direnv", "-x", ".git", "-x", "target", "**/*.rs"],
    };
    result.push(step(sh, "leptosfmt", "leptosfmt", leptos_args));

    // prettier — scripts/verify: prettier --check end2end
    let prettier_args: &[&str] = match mode {
        Mode::Check => &["--check", "end2end"],
        Mode::Fix => &["-w", "end2end"],
    };
    result.push(step(sh, "prettier", "prettier", prettier_args));

    // cargo deny — same in both modes
    result.push(step(sh, "cargo-deny", "cargo", &["deny", "check"]));

    // clippy — scripts/verify: cargo clippy --all-targets -- -D warnings (no --workspace)
    result.push(step(
        sh,
        "clippy",
        "cargo",
        &["clippy", "--all-targets", "--", "-D", "warnings"],
    ));
}
