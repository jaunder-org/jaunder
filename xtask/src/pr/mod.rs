//! PR observation compatibility frontdoor and privileged ADR promoter.
//!
//! The observer policy and read-only evidence algorithm live in the focused
//! `pr-observer` package; this module keeps the existing `cargo xtask pr` callers
//! working until their Task 4 migration. The privileged promoter remains here.

pub mod cleanup;
pub mod gh;
pub mod land;
pub mod promoter;

mod execute;
mod invocation;

#[cfg(test)]
pub(crate) use pr_observer::pr::test_support;
pub use pr_observer::pr::{decide, evidence, observe, shared_failure, snapshot, watch, workflow};

pub use execute::{PrOperation, execute, execute_with, into_result};
pub use pr_observer::pr::{Event, EventKind, Outcome, PrNumber, PrReport, Subject};
pub use pr_observer::pr::{SharedFailure, SubjectFailure};
