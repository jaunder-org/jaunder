//! The host half of [`crate::perf`] — there is no `performance` off the browser.
//!
//! It exists so [`crate::perf`] presents one signature on both targets and the
//! mark-name contract stays host-testable.

/// Host counterpart of [`crate::perf::mark`]: does nothing, successfully.
pub fn mark(_name: &str) {}
