//! Privileged ADR promoter and its required native arming/evidence helpers.
//! Ordinary PR observation and landing have no xtask CLI or report surface.

pub mod gh;
pub mod land;
pub mod promoter;

pub use pr_observer::pr::{PrNumber, Subject, decide, snapshot};
