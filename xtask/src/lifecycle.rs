//! Lifecycle completion for retained xtask commands.
use crate::result::CommandResult;

/// Best-effort hook preparation follows native checkout/source admission.
pub fn ensure_hooks_installed() {
    repository_policy::checkout::ensure_hooks_installed("xtask");
}

pub(crate) fn finalize(result: &mut CommandResult, start: std::time::Instant) {
    result.duration_ms = start.elapsed().as_millis();
    result.finished_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
}
