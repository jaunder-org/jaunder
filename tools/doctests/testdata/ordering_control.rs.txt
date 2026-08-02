//! The compiler fact the ordering proofs rest on.
//!
//! Task 12 makes three previously non-discriminating `compile_fail` blocks real by
//! giving each fixture `PartialEq, Eq`. That only proves anything if `PartialEq +
//! Eq` alone does **not** admit `<` — otherwise `a < b` would fail for the missing
//! `PartialOrd` either way and the "proof" is as vacuous as before.
//!
//! Control — with `PartialOrd`, `<` compiles:
//! ```
//! # #[derive(PartialEq, Eq, PartialOrd, Ord)]
//! # struct Ordered(&'static str);
//! # let a = Ordered("a");
//! # let b = Ordered("b");
//! assert!(a < b);
//! ```
//!
//! Without it, `<` does not — and `PartialEq`/`Eq` are present, so that is the
//! only thing it can be failing for:
//! ```compile_fail
//! # #[derive(PartialEq, Eq, PartialOrd, Ord)]
//! # struct Ordered(&'static str);
//! # let a = Ordered("a");
//! # let b = Ordered("b");
//! #[derive(PartialEq, Eq)]
//! struct Unordered(&'static str);
//! let _ = Unordered("a") < Unordered("b");
//! ```

pub struct Marker;
