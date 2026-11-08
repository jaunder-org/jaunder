//! Read-only PR observation policy and its GitHub evidence boundary.

pub mod decide;
pub mod evidence;
pub mod gh;
pub mod git;
pub mod observe;
pub mod shared_failure;
pub mod snapshot;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod watch;
pub mod workflow;

mod types;

pub use shared_failure::{SharedFailure, SubjectFailure};
pub use types::{Event, EventKind, Outcome, PrNumber, PrReport, Subject};
