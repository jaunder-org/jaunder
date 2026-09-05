//! Strictly-host-focused shared code — the host-side sibling of the target-agnostic
//! `common` crate (the strictly-client `client` crate is its symmetric peer). Code here
//! never compiles to wasm, so it may use `std::fs`/`std::env` freely without the
//! `#[cfg(not(target_arch = "wasm32"))]` gating `common` would demand (ADR-0058).
//!
//! Tenants live in their own modules. [`atompub`] owns protocol models and
//! serialization below the server routing boundary; [`capture`] owns the
//! `JAUNDER_CAPTURE_DIR` contract; [`error`] holds the server-side error carrier;
//! [`auth`] holds host-side HTTP credential parsing and session-cookie construction;
//! [`etag`], [`feed`], and [`render`] own host-only content construction,
//! Syndication Feed policy/serialization, and sanitization; [`config_key`]
//! owns the validated site-wide and per-user configuration registries; [`password`] owns the
//! validated domain secret and Argon2 operations, paired with the persisted
//! [`stored_password_hash::StoredPasswordHash`]; [`retention`] owns the bounded
//! database-maintenance vocabulary; [`metrics`] and [`telemetry`] own process
//! observability; and [`smtp_config`] holds the validated outbound relay aggregate.

pub mod atompub;
pub mod auth;
pub mod capture;
pub mod config_key;
pub mod error;
pub mod etag;
pub mod feed;
pub mod invite;
pub mod metrics;
pub mod password;
pub mod render;
pub mod retention;
pub mod smtp_config;
pub mod smtp_password;
pub mod stored_password_hash;
pub mod telemetry;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod token;

/// True only when test-only cheap Argon2 parameters are compiled in.
pub const CHEAP_KDF_ENABLED: bool = cfg!(feature = "cheap-kdf");

// An optimized build must never carry test-only Argon2 parameters.
#[cfg(all(feature = "cheap-kdf", not(debug_assertions)))]
compile_error!("cheap-kdf must not be enabled in a release/optimized build");
