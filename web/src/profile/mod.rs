//! The **profile** vertical: the `#[server]` endpoints (`get`,
//! `update`, `get_default_post_format`, `set_default_post_format`) and
//! the `ProfileData` wire DTO in [`api`], and the co-located reactive UI
//! (`ProfilePage`) in [`component`].
//!
//! This module is **wiring only** (ADR-0070, amended #530): module declarations
//! and re-exports, no items of its own. The UI is wasm-only ([`component`],
//! `#[cfg(target_arch = "wasm32")]`) and never host-compiles; the re-exports keep
//! `crate::profile::…` paths stable for the `email` vertical, the router, and the
//! server-fn registrar.

mod api;
/// The wasm-only profile UI (`ProfilePage`) — never host-compiled (ADR-0070);
/// calls the co-located `api::` endpoints directly.
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    get, get_default_post_format, set_default_post_format, update, Get, GetDefaultPostFormat,
    ProfileData, SetDefaultPostFormat, Update,
};
#[cfg(target_arch = "wasm32")]
pub use component::ProfilePage;
