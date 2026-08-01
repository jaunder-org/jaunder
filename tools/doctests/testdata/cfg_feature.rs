//! Shrink vector 1: a fence behind a `#[cfg(feature = …)]` the run does not
//! enable. This is the `common/src/render.rs` `sanitize` case that made the
//! issue's own measurement wrong — the fence does not fail, it vanishes.

#[cfg(feature = "off")]
pub mod gated {
    /// ```
    /// # let a = 1;
    /// let _ = a;
    /// ```
    pub struct Gated;
}

pub struct Marker;
