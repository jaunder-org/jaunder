//! Coverage post-processing engine: parse the instrumented text report and the
//! CRAP report the Nix `coverage` check emits, then apply the **stateless** gate.
//!
//! The gate is history-free: an executable line FAILS iff it is uncovered AND not
//! structurally exempt (inside a message-carrying `unreachable!` invocation, see
//! [`exempt`]) AND not marked `cov:ignore` (stripped in [`report`]). A *covered*
//! line inside an exempt span trips the A1 guard (an `unreachable!` assertion was
//! actually reached, so the exemption's premise is violated). CRAP is
//! gated against a fixed threshold (see [`crap`]), minus in-source `crap:allow`
//! overrides. There is no baseline, anchor, or manifest.

pub mod crap;
pub mod exempt;
pub mod gate;
pub mod probe;
pub mod report;

mod model;
mod run;

pub use model::{CoverageReport, FileCoverage, LineCov};
pub use run::run;
