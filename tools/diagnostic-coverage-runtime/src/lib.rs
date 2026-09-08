//! Diagnostic-only boundary around minicov's unsafe process-global runtime.
//!
#![cfg(target_arch = "wasm32")]

//! This crate is injected only into the copied Nix diagnostic manifest and is
//! compiled only for the diagnostic wasm target.

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
