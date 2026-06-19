use xshell::Shell;

use crate::result::StepResult;

/// Run a command as a named step. Non-zero exit becomes a failed StepResult
/// rather than a panic, so one failing step does not abort the others.
/// On failure, stderr (and stdout if non-empty) are captured into `detail`.
pub fn step(sh: &Shell, name: &str, program: &str, args: &[&str]) -> StepResult {
    match sh.cmd(program).args(args).quiet().ignore_status().output() {
        Ok(output) => {
            if output.status.success() {
                StepResult::ok(name)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let detail = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
                    (false, false) => format!("{}\n{}", stdout.trim(), stderr.trim()),
                    (false, true) => stdout.trim().to_string(),
                    (true, false) => stderr.trim().to_string(),
                    (true, true) => {
                        format!("exited with status {}", output.status.code().unwrap_or(-1))
                    }
                };
                StepResult::fail(name).detail(detail)
            }
        }
        Err(err) => StepResult::fail(name).detail(err.to_string()),
    }
}
