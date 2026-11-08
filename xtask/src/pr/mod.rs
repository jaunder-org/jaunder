//! Shared native GitHub transport and dormant ADR promoter compatibility.
//!
//! Ordinary PR observation and landing belong to Just and `pr-observer`, not
//! xtask. Python owns production promoter policy; the historical controller and
//! its armer remain test-only until the observation-gated PR 2 retirement.

pub mod gh;
#[cfg(test)]
pub mod land;
#[cfg(test)]
pub mod promoter;

pub use pr_observer::pr::{PrNumber, Subject, decide, snapshot};
