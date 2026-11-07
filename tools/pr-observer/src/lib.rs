//! Purpose-specific, read-only pull-request observation for Jaunder.
//!
//! `xtask` re-exports the policy modules temporarily while its existing command
//! frontdoor remains in place; the Just-owned binary in this package uses the same
//! implementation rather than carrying a second poller.

pub mod pr;
