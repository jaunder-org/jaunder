//! Wasm-only export layer for diagnostic LLVM profile capture.
//!
//! This module owns the browser-facing wasm-bindgen exports selected from
//! `lib.rs`.

use wasm_bindgen::prelude::wasm_bindgen;

/// Export the profile buffer without giving browser JavaScript access to the
/// process-global runtime. The caller persists these bytes as `.profraw`.
#[wasm_bindgen(js_name = jaunderCoverageProfile)]
pub fn profile() -> Option<Vec<u8>> {
    diagnostic_coverage_runtime::capture_profile()
}

/// Export a decimal module signature so the JavaScript boundary never loses
/// precision by coercing LLVM's `u64` identifier to a Number.
#[wasm_bindgen(js_name = jaunderCoverageModuleSignature)]
pub fn module_signature() -> String {
    diagnostic_coverage_runtime::module_signature()
}

/// Link the diagnostic exports into the CSR entrypoint without capturing a
/// profile during boot. Reading the signature is side-effect-free for coverage
/// counters and makes the module identity available to later browser capture.
pub fn install() {
    let _ = module_signature();
}
