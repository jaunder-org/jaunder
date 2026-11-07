//! Compatibility transport for the remaining mutating xtask callers.
//!
//! Observation and response classification are shared with `pr-observer`; stdin
//! payloads and prose-returning mutations stay here, outside the read-only tool.

pub use pr_observer::pr::gh::{
    ApiError, Deadline, GitError, classify, enrich_rate_limit, rate_limit_reset, run_gh,
    run_gh_deadline, run_gh_raw_deadline,
};

use serde_json::Value;
use std::io::Write;
use std::process::Command;

fn with_reset(error: ApiError) -> ApiError {
    match error {
        ApiError::RateLimited { .. } => enrich_rate_limit(error, rate_limit_reset()),
        other => other,
    }
}

/// Retain the existing issue payload transport until its final callers migrate.
pub fn run_gh_stdin(args: &[&str], stdin: &str) -> Result<Value, ApiError> {
    let mut child = match Command::new("gh")
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ApiError::GhMissing),
        Err(e) => return Err(ApiError::Transport(format!("could not run gh: {e}"))),
    };
    let Some(mut pipe) = child.stdin.take() else {
        return Err(ApiError::Transport("could not open gh stdin".into()));
    };
    pipe.write_all(stdin.as_bytes())
        .map_err(|e| ApiError::Transport(format!("could not write gh stdin: {e}")))?;
    drop(pipe);
    let out = child
        .wait_with_output()
        .map_err(|e| ApiError::Transport(format!("could not wait for gh: {e}")))?;
    classify(
        out.status.code().unwrap_or(-1),
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    )
    .map_err(with_reset)
}

/// A successful mutation may print prose, so only failures are classified as JSON.
pub fn run_gh_raw(args: &[&str]) -> Result<(), ApiError> {
    let out = match Command::new("gh").args(args).output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ApiError::GhMissing),
        Err(e) => return Err(ApiError::Transport(format!("could not run gh: {e}"))),
    };
    if out.status.success() {
        return Ok(());
    }
    match classify(
        out.status.code().unwrap_or(-1),
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    ) {
        Ok(_) => Err(ApiError::Transport(format!("gh exited {}", out.status))),
        Err(error) => Err(with_reset(error)),
    }
}
