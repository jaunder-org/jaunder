//! Raw browser clipboard access.

use wasm_bindgen_futures::JsFuture;

/// Write plain text to the browser clipboard.
///
/// The concrete browser error stays inside `client`; callers only decide how to
/// present the failed operation.
///
/// # Errors
///
/// Returns `Err(())` when no browser window is available or the browser rejects
/// the clipboard write.
pub async fn write_text(value: &str) -> Result<(), ()> {
    let window = web_sys::window().ok_or(())?;
    JsFuture::from(window.navigator().clipboard().write_text(value))
        .await
        .map(|_| ())
        .map_err(|_| ())
}
