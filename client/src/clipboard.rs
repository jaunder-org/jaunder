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
    let clipboard = clipboard_for(&window.navigator())?;
    let promise = write_promise(&clipboard, value)?;
    JsFuture::from(promise).await.map(|_| ()).map_err(|_| ())
}

fn clipboard_for(navigator: &web_sys::Navigator) -> Result<JsValue, ()> {
    let clipboard =
        Reflect::get(navigator.as_ref(), &JsValue::from_str("clipboard")).map_err(|_| ())?;
    if clipboard.is_null() || clipboard.is_undefined() {
        Err(())
    } else {
        Ok(clipboard)
    }
}

fn write_promise(clipboard: &JsValue, value: &str) -> Result<Promise, ()> {
    let write_text = Reflect::get(clipboard, &JsValue::from_str("writeText"))
        .map_err(|_| ())?
        .dyn_into::<Function>()
        .map_err(|_| ())?;
    write_text
        .call1(clipboard, &JsValue::from_str(value))
        .map_err(|_| ())?
        .dyn_into::<Promise>()
        .map_err(|_| ())
}
