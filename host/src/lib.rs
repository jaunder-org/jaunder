//! Strictly-host-focused shared code — the host-side sibling of the target-agnostic
//! `common` crate (a future strictly-client crate would be its symmetric peer). Code here
//! never compiles to wasm, so it may use `std::fs`/`std::env` freely without the
//! `#[cfg(not(target_arch = "wasm32"))]` gating `common` would demand (ADR-0058).
//!
//! Tenants live in their own modules. The first is [`capture`] — the `JAUNDER_CAPTURE_DIR`
//! contract (issue #227, ADR-0057); [`error`] holds the server-side error carrier
//! (issue #334, ADR-0058 as clarified); [`auth`] holds host-side HTTP credential
//! parsing + session-cookie construction pushed down from `web` (issue #334).

pub mod auth;
pub mod capture;
pub mod error;
