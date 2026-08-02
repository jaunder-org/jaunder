//! A doctest that runs and fails, so the reconciler can be shown reporting it as
//! FAILED rather than as absent. Folding a failure into "never evaluated" would be
//! a misleading message for the commonest case.
//!
//! ```
//! # let a = 1;
//! assert_eq!(a, 2, "this fixture fails on purpose");
//! ```

pub struct Marker;
