//! Shrink vector 3: a wholly unrecognized info string makes rustdoc treat the
//! block as non-Rust and skip it — with no warning at all. A typo like this
//! deletes a proof and reports green forever, which is why the vocabulary denies
//! by default rather than trying to emulate rustdoc's rules.
//!
//! ```nocheck
//! # let a = 1;
//! let _: i32 = "definitely not an i32";
//! ```

pub struct Marker;
