//! Purpose-specific, read-only pull-request observation for Jaunder.
//!
//! The `dev`-owned binary is the sole ordinary PR observer. The privileged ADR
//! promoter reuses only the evidence types and pure classification it needs.

pub mod pr;
