//! Diagnostic-only boundary around minicov's unsafe process-global runtime.
//!
//! This crate is injected only into the copied Nix diagnostic manifest. It
//! intentionally stays outside the product workspace and its lint policy.

/// Capture the current raw LLVM profile bytes, or report that minicov failed.
pub fn capture_profile() -> Option<Vec<u8>> {
    let mut profile = Vec::new();
    // SAFETY: the diagnostic producer invokes this after its one-worker browser
    // flow completes. No concurrent dump or reset operation is exposed.
    unsafe { minicov::capture_coverage(&mut profile).ok()? };
    Some(profile)
}

/// Return the module identity in decimal form for the JavaScript boundary.
#[must_use]
pub fn module_signature() -> String {
    minicov::module_signature().to_string()
}
