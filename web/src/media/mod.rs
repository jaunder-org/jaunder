//! The `media` vertical (ADR-0070, amended #530).
//!
//! This module is **wiring only**: module declarations and re-exports, no items of
//! its own. The `#[server]` endpoints and wire types live in [`api`]; the
//! `#[component]` UI and browser-bound upload glue live in the wasm-only
//! `component` leaf. Re-exports keep the stable `crate::media::…` paths external
//! call sites and the server-fn registrar depend on.

mod api;
mod format;
/// The upload widget's host-compiled reactive state and outcome fold (#306,
/// ADR-0083): ungated and coverage-measured, exercised under an `Owner`.
mod upload_state;

#[cfg(target_arch = "wasm32")]
mod component;

// Exported rather than `pub(crate)`: its only caller is the wasm-only `component`,
// so a crate-internal item would read as dead code on the host build, where that
// leaf is compiled out.
pub use format::{format_bytes, storage_usage_percent};
// Same reason as `format_bytes` above — the wasm-only `component` is the only
// caller, so these must stay reachable on the host build to avoid `dead_code`.
pub use upload_state::{
    UploadCallbacks, UploadOutcome, UploadPresentation, UploadState,
    delete_invalidates_media_resources, upload_presentation,
};

pub use api::{
    Delete, DeleteMediaRequest, GetUploadsEnabled, GetUsage, Item, ListMine, MediaDeletion, Upload,
    UsageData, delete, get_uploads_enabled, get_usage, list_mine, upload,
};

#[cfg(target_arch = "wasm32")]
pub use component::{MediaPage, MediaUpload};
