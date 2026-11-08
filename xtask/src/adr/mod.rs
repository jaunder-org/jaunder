//! ADR promotion commands.
//!
//! Fixed Nix-selected compatibility delegates; Python owns mutations and policy.

mod delegate;
mod promote;

pub use delegate::mutation_status;
pub(crate) use delegate::{controller, sync_readme};
pub use promote::promote;
#[cfg(test)]
pub(crate) use promote::run_promote;
