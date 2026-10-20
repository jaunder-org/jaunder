//! Mandatory private upload preparation, owned across request cancellation.
//! Native metadata editing runs only in the same-executable worker; admission,
//! private files and process reaping stay owned until placement or cleanup.
mod native;
mod process;
#[cfg(any(test, feature = "test-utils"))]
mod test_control;
mod wire;

pub use native::{WORKER_ARGUMENT, run_worker};
pub use process::{UploadAttempt, UploadPreparer};
#[cfg(any(test, feature = "test-utils"))]
pub use test_control::{READY_FILE, WorkerTestControl, WorkerTestMode};
pub use wire::PreparationError;
