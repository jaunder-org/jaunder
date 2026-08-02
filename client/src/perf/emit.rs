//! The browser half of [`crate::perf`] — the actual `performance.mark` call.

/// Emit `performance.mark(name)`.
///
/// Every lookup on the way to the API is fallible and every failure is
/// swallowed: instrumentation must never be able to break the boot it measures,
/// so neither the missing-`window` nor the missing-`performance` path unwraps.
pub fn mark(name: &str) {
    let Some(performance) = web_sys::window().and_then(|window| window.performance()) else {
        return;
    };
    _ = performance.mark(name);
}
