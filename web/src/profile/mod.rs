//! The **profile** vertical: the `#[server]` endpoints (`get_profile`,
//! `update_profile`, `get_default_post_format`, `set_default_post_format`) and
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
    get_default_post_format, get_profile, set_default_post_format, update_profile,
    GetDefaultPostFormat, GetProfile, ProfileData, SetDefaultPostFormat, UpdateProfile,
};
#[cfg(target_arch = "wasm32")]
pub use component::ProfilePage;
