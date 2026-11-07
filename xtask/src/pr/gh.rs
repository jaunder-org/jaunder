//! Compatibility transport for the remaining mutating xtask callers.
//!
//! Observation and response classification are shared with `pr-observer`;
//! prose-returning PR mutations stay here, outside the read-only tool.

pub use pr_observer::pr::gh::{
    ApiError, Deadline, GitError, classify, enrich_rate_limit, rate_limit_reset, run_gh,
    run_gh_deadline, run_gh_raw_deadline,
};

use std::process::Command;

fn with_reset(error: ApiError) -> ApiError {
    match error {
        ApiError::RateLimited { .. } => enrich_rate_limit(error, rate_limit_reset()),
        other => other,
    }
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
