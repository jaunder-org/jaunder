//! PR observation and serialized ADR promoter orchestration.
//!
//! Layered boundary → pure → loop (ADR draft `xtask-github-pr-observation`):
//! [`gh`] is the sole GitHub subprocess boundary, `snapshot` turns its JSON into
//! typed values, and `decide` holds pure policy. `watch`/`land` drive human PR
//! observation; `promoter` separately owns bot policy and delegates Git to
//! [`crate::git`].

pub mod cleanup;
pub mod decide;
pub mod gh;
pub mod land;
pub mod promoter;
pub mod snapshot;
#[cfg(test)]
pub(crate) mod test_support;
pub mod watch;

mod execute;
mod invocation;
mod types;

pub use execute::{PrOperation, execute, execute_with, into_result};
pub use invocation::{GitFacts, Invocation};
pub use types::{Event, EventKind, Outcome, PrNumber, PrReport, Subject};
