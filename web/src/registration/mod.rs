//! The **registration** vertical: account provisioning — the `#[server]` endpoints
//! `register` and `get_registration_policy` in [`api`], and the co-located
//! `RegisterPage` UI (with the invite-guidance view) in [`component`].
//!
//! Registration is distinct from authentication: it applies the site's registration
//! policy and redeems invite codes to *create* an account, then establishes the new
//! user's session through the sibling `crate::auth` vertical (`set_session_cookie`
//! plus the advisory `auth::marker_storage`). `login`/`logout` live in `auth`.
//!
//! This module is **wiring only** (ADR-0070, amended #530): module declarations
//! and re-exports, no items of its own. The UI is wasm-only ([`component`],
//! `#[cfg(target_arch = "wasm32")]`) and never host-compiles.

mod api;
/// The wasm-only registration UI (`RegisterPage` + the invite-guidance view) —
/// never host-compiled (ADR-0070).
#[cfg(target_arch = "wasm32")]
mod component;

// The API surface — re-exported so external call sites and the server-fn registrar
// keep the stable `crate::registration::…` paths despite living in `api.rs`.
pub use api::{get_registration_policy, register, GetRegistrationPolicy, Register};
#[cfg(target_arch = "wasm32")]
pub use component::RegisterPage;
