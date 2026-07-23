//! Generic browser DOM primitives — element text/removal by id, and element removal by
//! CSS selector. Raw `web_sys`, no domain types (the `navigation`/`dialog` precedent,
//! ADR-0069). The CSR boot (`csr`) reads the projector's seed blob and drops the
//! server-painted `#app` container + the duplicate `<head>` autodiscovery links through
//! these.

use wasm_bindgen::JsCast;

/// The current `Document`, or `None` off-DOM (no `window`, e.g. a host/test build).
fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

/// The `text_content` of the element with `id`, if the element exists.
#[must_use]
pub fn text_content_by_id(id: &str) -> Option<String> {
    document()?.get_element_by_id(id)?.text_content()
}

/// Remove the element with `id` from the document if present; no-op otherwise.
pub fn remove_element_by_id(id: &str) {
    if let Some(el) = document().and_then(|d| d.get_element_by_id(id)) {
        el.remove();
    }
}

/// Remove every element matching `selector` from the document; no-op off-DOM, on a
/// selector that matches nothing, or on an invalid selector (`query_selector_all` errs,
/// swallowed). Used to drop the projector-painted `<link>`s at CSR boot (#198).
pub fn remove_elements_by_selector(selector: &str) {
    if let Some(document) = document() {
        if let Ok(nodes) = document.query_selector_all(selector) {
            for i in 0..nodes.length() {
                if let Some(el) = nodes
                    .item(i)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                {
                    el.remove();
                }
            }
        }
    }
}
