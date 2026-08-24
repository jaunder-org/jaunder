//! Trace-derived `#[server]` fn flow-coverage (#681).
//!
//! ADR: `docs/adr/0081-empirical-server-fn-flow-coverage.md`.
//!
//! Proves, from evidence rather than assertion, which server entry points a real
//! browser session drives. [`extract`] is the pure seam — spans + inventory →
//! [`Coverage`]; the committed snapshot and two-lane gate are built on top of it.

pub mod extract;
pub mod io;
pub mod snapshot;

pub use extract::{Coverage, extract};
pub use snapshot::{REGENERATE_CMD, Snapshot, render, verdict};
