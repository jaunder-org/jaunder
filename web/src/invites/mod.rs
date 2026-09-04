//! Invites vertical — module wiring (ADR-0070). The API surface lives in
//! `api.rs`; the wasm-only UI in `component.rs`.

#[cfg(any(test, target_arch = "wasm32"))]
mod access;
mod api;
#[cfg(target_arch = "wasm32")]
mod component;

#[cfg(target_arch = "wasm32")]
pub(crate) use access::{PageAccess, resolve_page_access};
pub use api::{Create, CreateInviteRequest, Info, List, create, list};
#[cfg(target_arch = "wasm32")]
pub use component::InvitesPage;
