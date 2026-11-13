//! Stateless reconciliation for the producer-owned Emacs Lisp coverage artifacts.
//!
//! The producer owns execution state and its pre-test census. This module owns
//! the host-side verdict: it re-discovers the current flat production population,
//! checks that the handoff still names it exactly, and reconciles every census
//! point with LCOV and a source-local marker. Unknown input is deliberately an
//! error; coverage artifacts must never narrow the denominator silently.

mod consumer;
mod lcov;
mod model;
mod source;
#[cfg(test)]
mod tests;

pub use consumer::consume;
pub use model::{
    CoverageError, CoverageReport, FormCensus, ModuleCensus, PointCensus, PointKind,
    ProducerOutcome, ProducerStatus,
};
