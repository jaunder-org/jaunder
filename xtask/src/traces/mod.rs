//! `cargo xtask traces analyze` — OTel trace analysis (host-side, ADR-0028).
//!
//! Port of `scripts/analyze-otel-traces`. The reusable seam is [`analyze::analyze`]
//! → [`analyze::Analysis`] → [`render::render`]; the CLI handler in `lib.rs` is
//! thin, and #33's `traces run` will call `analyze`/`render` in-process.

pub mod analyze;
pub mod parse;
pub mod render;
