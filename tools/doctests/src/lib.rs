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
//! - [`roots`] is the one home for the scan roots, shared by both consumers.
//!
//! # The two halves of the run
//!
//! The population does not fit one invocation. `devtool doctests emit` runs
//! `cargo test --workspace --doc` inside the Nix producer and reconciles the root
//! workspace; the host-side `doctest-fences` xtask step covers `xtask/` and
//! `tools/`, which the flake `src` filter and the workspace boundary respectively
//! put out of reach. Each reconciles only its own roots, and [`roots::ALL`] is
//! asserted to cover every tracked `.rs` file — the only check that can catch a
//! crate belonging to neither half.

pub mod check;
pub mod fence;
#[cfg(test)]
mod harness;
pub mod libtest;
pub mod roots;
pub mod status;
