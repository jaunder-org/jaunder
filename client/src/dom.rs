//! Generic browser DOM primitives — "text content of element by id" and "remove
//! element by id". Raw `web_sys`, no domain types (the `navigation`/`dialog`
//! precedent, ADR-0069). The CSR boot (`csr`) reads the projector's seed blob and
//! drops the server-painted container through these.

/// The `text_content` of the element with `id`, if the element exists.
#[must_use]
pub fn text_content_by_id(id: &str) -> Option<String> {
    web_sys::window()?
        .document()?
        .get_element_by_id(id)?
        .text_content()
}

/// Remove the element with `id` from the document if present; no-op otherwise.
pub fn remove_element_by_id(id: &str) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
    {
        el.remove();
    }
}
