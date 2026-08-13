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
    #[must_use]
    pub fn track(&self) -> u32 {
        self.0.get()
    }
}

impl Default for Invalidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Invalidator;
    use crate::reactive::invalidator_scope;
    use leptos::reactive::owner::Owner;

    invalidator_scope! {
        /// Throwaway scope exercising the macro-generated newtype (`Deref` + `Copy`).
        struct TestScope
    }

    // The macro-generated newtype is trivial, pure code (`Deref` to the inner
    // `Invalidator` + `Copy`), so it is covered here rather than exempted. Reaching the
    // macro through `crate::reactive::invalidator_scope` — rather than by textual scope —
    // is also what keeps the `scope.rs` → `mod.rs` re-export chain consumed on the host
    // build.
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
}
