//! The `invalidator_scope!` context-scope newtype macro. Wasm-only by its `mod`
//! declaration in [`super`] (ADR-0070's file-level split, one level down), plus a
//! `test` arm there so the generated newtype stays host-tested and coverage-measured
//! rather than exempted. This file carries no cfg of its own.
//!
//! The macro's test lives in [`super`]'s `tests` module rather than here: it must
//! exercise the re-export chain, since a `macro_rules!` is textually in scope within
//! its own file and a test here would leave `pub(crate) use` below unconsumed.

/// Declares a distinct context-scope newtype over an [`Invalidator`](super::Invalidator),
/// with `Deref` so the full `Invalidator` API is available on it. Use one per
/// **cross-component** refetch scope and `provide_context` / `expect_context` it, so
/// scopes never collide by type (a bare `Invalidator` in context would). A *local*
/// scope needs no newtype — a bare `Invalidator` suffices.
///
/// Illustration, not a test: the macro is `pub(crate) use` only, and this module is
/// reached solely through `#[cfg(any(target_arch = "wasm32", test))]` in [`super`].
/// rustdoc sets `cfg(doctest)` but **not** `cfg(test)`, so a host doc run never
/// compiles this file at all. The real exercise is [`super`]'s `tests` module, which
/// has to go through the re-export chain anyway (see the module header).
///
/// ```text
/// invalidator_scope! {
///     /// The audience-list refetch scope.
///     struct AudienceList
/// }
/// ```
macro_rules! invalidator_scope {
    ($(#[$meta:meta])* $vis:vis struct $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy)]
        $vis struct $name($vis $crate::reactive::Invalidator);

        impl ::core::ops::Deref for $name {
            type Target = $crate::reactive::Invalidator;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
    };
}

pub(crate) use invalidator_scope;
