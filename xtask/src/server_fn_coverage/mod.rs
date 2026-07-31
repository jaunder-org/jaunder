//! Trace-derived `#[server]` fn flow-coverage (#681).
//!
//! ADR: `docs/adr/0081-empirical-server-fn-flow-coverage.md`.
//!
//! Proves, from evidence rather than assertion, which server entry points a real
//! browser session drives. [`extract`] is the pure seam — spans + inventory →
//! [`Coverage`]; everything else (the committed snapshot, the allowlist, the
//! two-lane gate) is built on top of it.

pub mod extract;
pub mod io;
pub mod snapshot;

pub use extract::{extract, Coverage};
pub use snapshot::{
    evidence_verdict, render, verdict, AllowlistEntry, Evidence, Snapshot, REGENERATE_CMD,
};
