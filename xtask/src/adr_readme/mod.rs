//! Read-only ADR documentation validation and the fixed synchronization delegate.
//!
//! `files` owns numbered ADR discovery and format validation, `readme` checks
//! table parity, and `view` checks architecture-view parity. Python owns writes;
//! the existing synchronization facade remains a narrow compatibility delegate.

mod files;
mod readme;
mod view;

pub(crate) use crate::adr::sync_readme;
pub(crate) use files::format_problems;
pub(crate) use readme::parity_report;
pub(crate) use view::view_parity_problems;
