//! `cargo xtask traces` — OTel trace tooling (host-side, ADR-0028).
//!
//! `analyze` (port of `scripts/analyze-otel-traces`) is the reusable seam —
//! [`analyze::analyze`] → [`analyze::Analysis`] → [`render::render`]. `run` (port
//! of `scripts/run-e2e-trace-analysis`) nix-builds the e2e checks and drives that
//! seam in-process. The CLI handlers in `lib.rs` are thin.

pub mod analyze;
pub mod parse;
pub mod render;
pub mod run;
