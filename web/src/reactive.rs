//! Reactive revalidation core shared across `web` verticals.
//!
//! [`Invalidator`] is the canonical revalidation idiom (design record:
//! `docs/adr/0060-web-invalidator-revalidation-idiom.md`): a committed mutation
//! `notify()`s an invalidator, and every resource that `track()`s it refetches. This module
//! owns the host-tested core — `new` / `notify` / `track` and the `invalidator_scope!`
//! context-scope newtype. The browser-bound helpers built on it
//! (`resource` / `action` / `patched` / `sticky`, the latter two driving ADR-0061's keyed
//! list and its sticky peer) live in `client::reactive` (#515) — wasm-only and e2e-exercised.

use leptos::prelude::*;

/// A revalidation handle. A committed mutation [`notify`](Self::notify)s it; the resources
/// that [`track`](Self::track) it refetch.
///
/// It wraps a counter because a leptos [`Resource`] refetches only when its source *value*
/// changes — a notify-only `Trigger` returning `()` would never fire, so the counter is the
/// mechanism (exactly as `ServerAction::version()` is). The counter is encapsulated: the
/// browser-bound helpers in `client::reactive` build on `notify` / `track`, never a raw signal.
#[derive(Clone, Copy, Debug)]
pub struct Invalidator(RwSignal<u32>);

impl Invalidator {
    /// A fresh invalidator.
    #[must_use]
    pub fn new() -> Self {
        Self(RwSignal::new(0))
    }

    /// Signal that a mutation committed: every resource tracking this invalidator refetches.
    pub fn notify(&self) {
        self.0.update(|n| *n = n.wrapping_add(1));
    }

    /// Subscribe the current reactive scope to this invalidator. Used as a [`Resource`]
    /// source; the returned value is an opaque revision, not meaningful on its own.
    pub fn track(&self) -> u32 {
        self.0.get()
    }
}

impl Default for Invalidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Declares a distinct context-scope newtype over an [`Invalidator`], with `Deref` so the
/// full `Invalidator` API is available on it. Use one per **cross-component** refetch scope
/// and `provide_context` / `expect_context` it, so scopes never collide by type (a bare
/// `Invalidator` in context would). A *local* scope needs no newtype — a bare `Invalidator`
/// suffices.
///
/// ```ignore
/// invalidator_scope! {
///     /// The audience-list refetch scope.
///     struct AudienceList
/// }
/// ```
// Consumers of this macro are wasm-only `component.rs` files (the generated context-scope
// newtype is browser-bound reactive UI, ADR-0070); the only host build that references it is
// this module's own `#[cfg(test)]` coverage below. Gating the definition to those targets
// keeps the plain host-lib build from flagging it as an unused macro.
#[cfg(any(target_arch = "wasm32", test))]
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
// The re-export is consumed only by the wasm-only `component.rs` files; this module's own
// `#[cfg(test)]` use reaches the definition by textual scope, not through the re-export.
#[cfg(target_arch = "wasm32")]
pub(crate) use invalidator_scope;

#[cfg(test)]
mod tests {
    use super::Invalidator;
    use leptos::reactive::owner::Owner;

    invalidator_scope! {
        /// Throwaway scope exercising the macro-generated newtype (`Deref` + `Copy`).
        struct TestScope
    }

    // The load-bearing property: each `notify` changes the value a `Resource` source
    // observes via `track`, which is what makes the resource refetch. (The `action`
    // success-gating and refetch-on-notify are client-only reactive behavior — `Effect`
    // does not run in a host test — so they are exercised by the audiences e2e.)
    #[test]
    fn notify_changes_the_tracked_revision() {
        let owner = Owner::new();
        owner.set();
        let inv = Invalidator::default(); // also covers `new` (Default delegates to it)
        let v0 = inv.track();
        inv.notify();
        let v1 = inv.track();
        inv.notify();
        let v2 = inv.track();
        drop(owner);
        assert_ne!(v1, v0, "notify must change the tracked revision");
        assert_ne!(v2, v1, "each notify must change it again");
    }

    // The macro-generated newtype is trivial, pure code (`Deref` to the inner `Invalidator`
    // + `Copy`), so it is covered here rather than exempted.
    #[test]
    fn scope_newtype_derefs_to_its_invalidator() {
        let owner = Owner::new();
        owner.set();
        let scope = TestScope(Invalidator::new());
        let copied = scope; // Copy
        let v0 = scope.track(); // via Deref
        copied.notify(); // both wrap the same inner signal
        let v1 = scope.track();
        drop(owner);
        assert_ne!(v1, v0, "Deref reaches the inner Invalidator");
    }
}
