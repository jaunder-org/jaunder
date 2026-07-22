//! Raw browser navigation primitives (`window.location`). Wasm-only; no domain types.

/// Replace the current history entry with `url` (`location.replace`). No-op off-DOM;
/// the navigation `Result` is swallowed, matching the `web` call sites it replaces.
pub fn replace(url: impl AsRef<str>) {
    if let Some(window) = web_sys::window() {
        let _ = window.location().replace(url.as_ref());
    }
}

/// Reload the current URL by replacing it with itself — a full server round-trip
/// (hands a non-SPA route back to the server). No-op off-DOM.
pub fn reload() {
    if let Some(window) = web_sys::window() {
        let location = window.location();
        if let Ok(href) = location.href() {
            let _ = location.replace(&href);
        }
    }
}
