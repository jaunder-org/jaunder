use std::time::Instant;

use xshell::Shell;

use crate::result::StepResult;

/// Run a command as a named step. Non-zero exit becomes a failed StepResult
/// rather than a panic, so one failing step does not abort the others.
/// On failure, stderr (and stdout if non-empty) are captured into `detail`.
pub fn step(sh: &Shell, name: &str, program: &str, args: &[&str]) -> StepResult {
    step_with_env(sh, name, program, args, &[])
}

pub fn step_with_env(
    sh: &Shell,
    name: &str,
    program: &str,
    args: &[&str],
    env: &[(String, String)],
) -> StepResult {
    let start = Instant::now();
    let mut cmd = sh.cmd(program).args(args).quiet().ignore_status();
    for (key, value) in env {
        cmd = cmd.env(key, value);
    }
    match cmd.output() {
        Ok(output) => {
            if output.status.success() {
                StepResult::ok(name).with_duration(start.elapsed())
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
                StepResult::fail(name)
                    .detail(detail)
                    .with_duration(start.elapsed())
            }
        }
        Err(err) => StepResult::fail(name)
            .detail(err.to_string())
            .with_duration(start.elapsed()),
    }
}
