//! Browser file-picker → multipart upload glue (#520). Raw browser API access plus
//! the `server_fn` transport type it produces; no domain types (ADR-0069).

use leptos::html::Input;
use leptos::prelude::{Get, NodeRef};
use server_fn::codec::MultipartData;

pub use crate::telemetry::{UploadError, UploadOutcome};

/// The first file currently chosen in `input`. `None` when the ref is unmounted,
/// the element exposes no file list, or nothing is selected.
fn picked_file(input: NodeRef<Input>) -> Option<web_sys::File> {
    input.get()?.files()?.get(0)
}

/// The first file currently chosen in `input`, wrapped as multipart form data
/// under the field name `file`.
///
/// [`UploadOutcome::NoFile`] is expected control flow. Browser exceptions during
/// `FormData` construction or append are returned as distinct errors.
///
/// # Errors
///
/// Returns [`UploadError::FormDataCreate`] or [`UploadError::FormDataAppend`]
/// for the corresponding thrown browser operation.
pub fn picked_file_multipart(
    input: NodeRef<Input>,
) -> Result<UploadOutcome<MultipartData>, UploadError> {
    let Some(file) = picked_file(input) else {
        return Ok(UploadOutcome::NoFile);
    };
    let form_data = web_sys::FormData::new().map_err(|_| UploadError::FormDataCreate)?;
    form_data
        .append_with_blob("file", &file)
        .map_err(|_| UploadError::FormDataAppend)?;
    Ok(UploadOutcome::Ready(form_data.into()))
}
