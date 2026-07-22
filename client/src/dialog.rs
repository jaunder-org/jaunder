//! Raw browser dialog primitives (`window.confirm`). Wasm-only; no domain types.

/// Show a native confirm dialog; `true` only if the user confirmed. `false` off-DOM
/// or if the dialog can't be shown (matching the current `unwrap_or(false)`).
#[must_use]
pub fn confirm(message: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.confirm_with_message(message).ok())
        .unwrap_or(false)
}
