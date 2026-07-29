//! The **sessions** vertical: the `#[server]` endpoints (`list`,
//! `create_app_password`, `revoke`) and the `SessionInfo` / `AppPassword`
//! wire DTOs in [`api`], and the co-located reactive UI (`SessionsPage`) in
//! [`component`]. App passwords are labelled sessions used for `AtomPub` HTTP
//! Basic auth.
//!
//! This module is **wiring only** (ADR-0070, amended #530): module declarations
//! and re-exports, no items of its own. The UI is wasm-only ([`component`],
//! `#[cfg(target_arch = "wasm32")]`) and never host-compiles; the re-exports keep
//! `crate::sessions::…` paths stable for the router and the server-fn registrar.

mod api;
/// The wasm-only sessions UI (`SessionsPage`) — never host-compiled (ADR-0070);
/// calls the co-located `api::` endpoints directly.
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    create_app_password, list, revoke, AppPassword, CreateAppPassword, List, Revoke, SessionInfo,
};
#[cfg(target_arch = "wasm32")]
pub use component::SessionsPage;
