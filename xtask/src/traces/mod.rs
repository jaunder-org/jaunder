//! `cargo xtask traces` — OTel trace tooling (host-side, ADR-0028).
//!
//! `analyze` (port of `scripts/analyze-otel-traces`) is the reusable seam —
//! [`analyze::analyze`] → [`analyze::Analysis`] → [`render::render`]. `run` (port
//! of `scripts/run-e2e-trace-analysis`) nix-builds the e2e checks and drives that
//! seam in-process. `boot_phases` is a separate command rather than another
//! `analyze` section: `analyze` reports maxima and means, and #818's question is
//! entirely medians. The CLI handlers in `lib.rs` are thin.

pub mod analyze;
pub mod boot_phases;
pub mod parse;
pub mod render;
pub mod report;
pub mod run;
