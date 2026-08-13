//! PR observation: `cargo xtask pr watch` / `pr land` (#729).
//!
//! Layered boundary → pure → loop (ADR draft `xtask-github-pr-observation`): only
//! `gh` runs a subprocess, `snapshot` turns its JSON into typed values, `decide` is a
//! pure state machine over those values, and `watch`/`land` drive the loop. Above
//! `snapshot` nothing sees JSON, a string status, or an exit code.

pub mod decide;
pub mod gh;
pub mod land;
pub mod snapshot;
#[cfg(test)]
pub(crate) mod test_support;
pub mod watch;

mod execute;
mod invocation;
mod types;

pub use execute::{execute, execute_with, into_result};
pub use invocation::{GitFacts, Invocation};
pub use types::{Event, EventKind, Outcome, PrNumber, PrReport, Subject};
