//! Raw browser clipboard access.

use js_sys::{Function, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// Write plain text to the browser clipboard.
///
/// The concrete browser error stays inside `client`; callers only decide how to
/// present the failed operation.
///
/// # Errors
///
/// Returns `Err(())` when no browser window or Clipboard API is available, or
/// when invoking or awaiting the clipboard write fails.
pub async fn write_text(value: &str) -> Result<(), ()> {
    let window = web_sys::window().ok_or(())?;
    let navigator = window.navigator();
    let clipboard =
        Reflect::get(navigator.as_ref(), &JsValue::from_str("clipboard")).map_err(|_| ())?;
    if clipboard.is_null() || clipboard.is_undefined() {
        return Err(());
    }
    let write_text = Reflect::get(&clipboard, &JsValue::from_str("writeText"))
        .map_err(|_| ())?
        .dyn_into::<Function>()
        .map_err(|_| ())?;
    let promise = write_text
        .call1(&clipboard, &JsValue::from_str(value))
        .map_err(|_| ())?
        .dyn_into::<Promise>()
        .map_err(|_| ())?;
    JsFuture::from(promise).await.map(|_| ()).map_err(|_| ())
}
