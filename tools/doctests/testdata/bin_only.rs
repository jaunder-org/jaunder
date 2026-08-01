//! Shrink vector 5: cargo collects doctests from **lib targets only**. This crate
//! has `src/main.rs` and no `src/lib.rs`, so the fence below can never appear in
//! any run — the state `tools/devtool` is in today.

/// ```
/// # let a = 1;
/// let _ = a;
/// ```
pub struct Documented;

fn main() {}
