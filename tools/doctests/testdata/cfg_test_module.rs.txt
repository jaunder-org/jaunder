//! Shrink vector 2: rustdoc sets `cfg(doctest)`, **not** `cfg(test)`, so a module
//! reached only under `cfg(test)` is never compiled for the doc run and its fences
//! are invisible. This is `web/src/reactive/scope.rs`.

#[cfg(test)]
pub mod gated {
    /// ```
    /// # let a = 1;
    /// let _ = a;
    /// ```
    pub struct Gated;
}

pub struct Marker;
