//! The `media` vertical (ADR-0070, amended #530).
//!
//! This module is **wiring only**: module declarations and re-exports, no items of
//! its own. The `#[server]` endpoints and wire types live in [`api`]; the
//! `#[component]` UI and browser-bound upload glue live in the wasm-only
//! `component` leaf. Re-exports keep the stable `crate::media::…` paths external
//! call sites and the server-fn registrar depend on.

mod api;

#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    delete_media, list_my_media, media_usage, upload_media, DeleteMedia, DeleteMediaResult,
    ListMyMedia, MediaItem, MediaUsage, MediaUsageData, UploadMedia,
};

#[cfg(target_arch = "wasm32")]
pub use component::{MediaPage, MediaUpload};
