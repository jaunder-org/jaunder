//! Run one subprocess as a named focused-proof step.
use std::time::Instant;

use xshell::Shell;

use crate::report::StepResult;

/// Non-zero exit becomes a failed step rather than a panic; on failure, stderr
/// (and stdout if non-empty) are captured into `detail`.
pub fn step_with_env(
    sh: &Shell,
    name: &str,
    program: &str,
    args: &[&str],
    env: &[(String, String)],
) -> StepResult {
    let traced = crate::invocation_trace::active()
        && matches!(name, "test-local-image-worker" | "test-local");
    let phase = traced.then(|| {
        crate::invocation_trace::phase(if name == "test-local" {
            "test-local"
        } else {
            "test-local-image-worker"
        })
    });
    let start = (!traced).then(Instant::now);
    let mut cmd = sh.cmd(program).args(args).quiet().ignore_status();
    for (key, value) in env {
        cmd = cmd.env(key, value);
    }
    let result = match cmd.output() {
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
    };
    if let Some(phase) = phase {
        phase.finish(result.ok);
    }
    match start {
        Some(start) => result.with_duration(start.elapsed()),
        None => result,
    }
}

#[cfg(test)]
mod tests {
    use super::step_with_env;
    use xshell::Shell;

    #[test]
    fn step_records_success_and_failure_diagnostics_with_env() {
        let sh = Shell::new().unwrap();
        let env = [("STEP_VALUE".to_string(), "seen".to_string())];
        let ok = step_with_env(
            &sh,
            "ok",
            "sh",
            &["-c", "test \"$STEP_VALUE\" = seen"],
            &env,
        );
        assert!(ok.ok && ok.detail.is_none(), "{ok:?}");
        let failed = step_with_env(
            &sh,
            "bad",
            "sh",
            &["-c", "echo out; echo err >&2; exit 3"],
            &[],
        );
        assert!(!failed.ok);
        assert_eq!(failed.detail.as_deref(), Some("out\nerr"));
        let silent = step_with_env(&sh, "silent", "sh", &["-c", "exit 4"], &[]);
        assert_eq!(silent.detail.as_deref(), Some("exited with status 4"));
        let missing = step_with_env(&sh, "missing", "/nonexistent/program", &[], &[]);
        assert!(!missing.ok && missing.detail.is_some());
    }
}
