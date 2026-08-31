//! Generate and validate the ADR documentation projections.
//!
//! `files` owns numbered ADR discovery and format validation, `readme` owns the
//! generated table and its parity check, and `view` owns architecture-view
//! parity. The existing facade paths are retained for xtask callers.

mod files;
mod readme;
mod view;

pub(crate) use files::{ACCEPTED, PROPOSED, format_problems, status_line};
pub(crate) use readme::{README, parity_report, readme_has_markers, sync_readme, sync_readme_at};
pub(crate) use view::view_parity_problems;
