//! Browser file-picker → multipart upload glue (#520). Raw browser API access plus the
//! `server_fn` transport type it produces; no domain types (ADR-0069). Relocated from
//! `web::media::component` so `web` carries no `web-sys` dependency at all.

use leptos::html::Input;
use leptos::prelude::{Get, NodeRef};
use server_fn::codec::MultipartData;

/// The first file currently chosen in `input`. `None` when the ref is unmounted, the
/// element exposes no file list, or nothing is selected.
///
/// Split from [`picked_file_multipart`] because reading the picker's selection and
/// wrapping a file for transport are separate concerns — and because the combined
/// five-branch chain exceeded the CRAP threshold. CRAP scores `client` functions even
/// though line coverage never reaches them, so the score can only be lowered by
/// splitting, never by testing (ADR-0069, "Corrected 2026-07-29").
fn picked_file(input: NodeRef<Input>) -> Option<web_sys::File> {
    input.get()?.files()?.get(0)
}

/// The first file currently chosen in `input`, wrapped as multipart form data under the
/// field name `file`. `None` when nothing is picked (see [`picked_file`]) or the browser
/// refuses to build the `FormData`.
#[must_use]
pub fn picked_file_multipart(input: NodeRef<Input>) -> Option<MultipartData> {
    let file = picked_file(input)?;
    let form_data = web_sys::FormData::new().ok()?;
    form_data.append_with_blob("file", &file).ok()?;
    Some(form_data.into())
}
