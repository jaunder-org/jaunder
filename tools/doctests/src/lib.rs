//! The doctest gate's reader: what fences exist in the tree, and what the doctest
//! runner actually evaluated.
//!
//! `cargo nextest` structurally cannot run doctests, so the `coverage` check never
//! sees them — which left 31 `compile_fail` proofs in this repo ungated (#763).
//! Running them is half the fix; the other half is proving the run saw all of them,
//! which is what [`check::problems`] reconciles.
//!
//! # The pieces
//!
//! - [`fence`] reads every rustdoc fence out of a source file with `syn`, keyed by
//!   the line its backticks open on — the same key libtest prints.
//! - [`libtest`] reads what the runner evaluated, out of its output.
//! - [`check`] holds the rules (the closed fence vocabulary, the companion rule)
//!   and the bidirectional reconciliation between those two populations.
//! - [`status`] is the sentinel the Nix producer writes and the gate consumes.
//! - [`roots`] is the one home for the scan roots.
//!
//! # The population
//!
//! `devtool doctests emit` runs `cargo test --workspace --doc` inside the Nix
//! producer and reconciles the root workspace's roots, [`roots::WORKSPACE`].
//! The separate `tools/` workspace is deliberately outside the fence population:
//! it has no doctests, and `checks.tools-doctests` runs its `cargo test --doc`
//! without reconciliation. The roots module asserts that every tracked `.rs`
//! file has exactly one owner — the only check that can catch a crate that
//! belongs to neither.

pub mod check;
pub mod fence;
#[cfg(test)]
mod harness;
pub mod libtest;
pub mod roots;
pub mod status;
