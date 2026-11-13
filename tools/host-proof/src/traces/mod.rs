//! `trace-analysis` — OTel trace tooling (host-side, ADR-0028/ADR-0246).
//!
//! `analyze` is the reusable seam —
//! [`analyze::analyze`] → [`analyze::Analysis`] → [`render::render`]. `run`
//! nix-builds the e2e checks and drives that seam in-process. `boot_phases` is
//! a separate command rather than another `analyze` section: `analyze` reports
//! maxima and means, and #818's question is entirely medians. Each command
//! returns a report whose `report` field carries the rendered tables.

pub mod analyze;
pub mod boot_phases;
mod command;
pub mod parse;
pub mod render;
pub mod report;
pub mod run;

pub use command::{analyze_command, boot_phases_command, run_command};
