use xshell::Shell;

use crate::result::StepResult;

/// Run a command as a named step. Non-zero exit becomes a failed StepResult
/// rather than a panic, so one failing step does not abort the others.
pub fn step(sh: &Shell, name: &str, program: &str, args: &[&str]) -> StepResult {
    match sh.cmd(program).args(args).quiet().run() {
        Ok(()) => StepResult::ok(name),
        Err(err) => StepResult::fail(name).detail(err.to_string()),
    }
}
