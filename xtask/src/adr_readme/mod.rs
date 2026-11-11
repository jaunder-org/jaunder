//! Read-only ADR documentation validation.
//!
//! `files` owns numbered ADR discovery and format validation, `readme` checks
//! table parity, and `view` checks architecture-view parity. Python owns writes.

mod files;
mod readme;
mod view;

pub(crate) use readme::parity_report;
pub(crate) use view::view_parity_problems;
