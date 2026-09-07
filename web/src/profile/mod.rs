//! The **profile** vertical: profile data and default-post-format preferences.
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
mod page_state;

pub use api::{
    Data, Get, GetDefaultPostFormat, SetDefaultPostFormat, Update, get, get_default_post_format,
    set_default_post_format, update,
};
#[cfg(target_arch = "wasm32")]
pub use component::ProfilePage;
pub use page_state::DefaultPostFormatState;
