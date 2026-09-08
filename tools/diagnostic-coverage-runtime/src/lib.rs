//! Diagnostic-only boundary around minicov's unsafe process-global runtime.
//!
//! Nix injects this crate only into the diagnostic wasm build. The direct
//! minicov calls remain wasm-only; target-independent result and signature
//! handling is compiled and tested on the host.

#[cfg(target_arch = "wasm32")]
/// Capture the current nonempty raw LLVM profile bytes, or report that minicov
/// failed to produce usable evidence.
pub fn capture_profile() -> Option<Vec<u8>> {
    let mut profile = Vec::new();
    // SAFETY: the diagnostic producer invokes this after its one-worker browser
    // flow completes. No concurrent dump or reset operation is exposed.
    let succeeded = unsafe { minicov::capture_coverage(&mut profile).is_ok() };
    captured_profile(profile, succeeded)
}

#[cfg(target_arch = "wasm32")]
/// Return the module identity in decimal form for the JavaScript boundary.
#[must_use]
pub fn module_signature() -> String {
    decimal_signature(minicov::module_signature())
}

#[cfg(any(target_arch = "wasm32", test))]
fn captured_profile(profile: Vec<u8>, succeeded: bool) -> Option<Vec<u8>> {
    (succeeded && !profile.is_empty()).then_some(profile)
}

#[cfg(any(target_arch = "wasm32", test))]
fn decimal_signature(signature: u64) -> String {
    signature.to_string()
}

#[cfg(test)]
mod tests {
    use super::{captured_profile, decimal_signature};

    #[test]
    fn capture_requires_successful_nonempty_profile() {
        assert_eq!(captured_profile(vec![1, 2], true), Some(vec![1, 2]));
        assert_eq!(captured_profile(Vec::new(), true), None);
        assert_eq!(captured_profile(vec![1], false), None);
    }

    #[test]
    fn signature_uses_decimal_browser_wire_format() {
        assert_eq!(
            decimal_signature(6_362_899_886_360_772_764),
            "6362899886360772764"
        );
    }
}
