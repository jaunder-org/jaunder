//! ADR promotion commands.
//!
//! `promote` owns the tracked-draft graduation workflow. `rewrite` owns
//! deterministic content transformations used by that workflow.

mod promote;
mod rewrite;

pub use promote::promote;
pub(crate) use promote::run_promote;
