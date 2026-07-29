//! Trace-derived `#[server]` fn flow-coverage (#681).
//!
//! ADR: `docs/adr/drafts/empirical-server-fn-flow-coverage.md`.
//!
//! Proves, from evidence rather than assertion, which server entry points a real
//! browser session drives. [`extract`] is the pure seam — spans + inventory →
//! [`Coverage`]; everything else (the committed snapshot, the allowlist, the
//! two-lane gate) is built on top of it.

pub mod extract;

pub use extract::{extract, Coverage};
