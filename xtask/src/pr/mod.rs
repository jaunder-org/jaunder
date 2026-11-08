//! Independent PR observation exports and shared transport for performance evidence.

pub mod gh;

pub use pr_observer::pr::{PrNumber, Subject, decide, snapshot};
