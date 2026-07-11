//! Reactive plumbing shared across `web` verticals.
//!
//! [`Invalidator`] is the canonical revalidation idiom (design record:
//! `docs/adr/0060-web-invalidator-revalidation-idiom.md`): a committed mutation
//! `notify()`s an invalidator, and every resource that `track()`s it refetches.
//! [`Invalidator::patched`] extends it to a keyed `reactive_stores` list (design record:
//! `docs/adr/0061-web-keyed-list-reactive-store.md`): a refetch `patch`es the store in
//! place so unchanged rows keep their DOM, and [`ListState`] tracks the list's load status.
//! [`Invalidator::sticky`] is its flat peer: it stickily retains a resolved `Result` across
//! refetches (no keyed store), surfacing a fetch error to the caller rather than flashing to
//! "Loading…" or swallowing it into a default.

use leptos::prelude::*;
use leptos::server_fn::ServerFn;
use macros::client_only;

/// A revalidation handle. A committed mutation [`notify`](Self::notify)s it; the resources
/// that [`track`](Self::track) it refetch.
///
/// It wraps a counter because a leptos [`Resource`] refetches only when its source *value*
/// changes — a notify-only `Trigger` returning `()` would never fire, so the counter is the
/// mechanism (exactly as `ServerAction::version()` is). The counter is encapsulated: reach
/// for [`resource`](Self::resource) / [`action`](Self::action), or the low-level
/// `notify`/`track`, never a raw signal.
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

    /// A [`Resource`] that refetches whenever this invalidator fires. The fetcher is
    /// nullary — the counter is an internal detail callers never see.
    ///
    /// Client-only: the `server_resource` fetch runs only in the browser, so it is
    /// exercised by the audiences e2e, not host tests.
    #[must_use]
    #[client_only]
    pub fn resource<T, Fut>(&self, fetch: impl Fn() -> Fut + Send + Sync + 'static) -> Resource<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
        Fut: std::future::Future<Output = T> + Send + 'static,
    {
        let this = *self;
        crate::server_resource(move || this.track(), move |_| fetch())
    }

    /// A [`ServerAction`] that fires this invalidator on **success** — after the mutation
    /// commits (`Some(Ok(_))`), never on dispatch or on failure. The gating `Effect` is
    /// wired internally, so no caller writes it.
    ///
    /// Client-only: the success-gating `Effect` fires only in the browser, so it is
    /// exercised by the audiences e2e, not host tests.
    #[must_use]
    #[client_only]
    pub fn action<A>(&self) -> ServerAction<A>
    where
        A: ServerFn + Send + Sync + Clone + 'static,
        A::Output: Send + Sync + 'static,
        A::Error: Send + Sync + 'static,
    {
        let action = ServerAction::<A>::new();
        let this = *self;
        Effect::new(move |_| {
            if action.value().with(|v| matches!(v, Some(Ok(_)))) {
                this.notify();
            }
        });
        action
    }

    /// Drives a keyed [`reactive_stores`](https://docs.rs/reactive_stores) list from a refetch
    /// of `fetch` (revalidated by this invalidator). On each successful refetch it hands the
    /// rows to `patch` — supplied as a closure so the caller's concrete keyed field runs its
    /// **in-place** `patch` (a generic bound would instead resolve to the unkeyed, positional
    /// patch and lose per-row identity). Returns the list's [`ListState`]; a pending or failed
    /// refetch never patches, so the last-good rows are retained.
    ///
    /// Client-only: it drives a `server_resource` refetch through a browser-only `Effect`,
    /// so it is exercised by the audiences e2e, not host tests.
    #[must_use]
    #[client_only]
    pub fn patched<T, Fut, E>(
        &self,
        fetch: impl Fn() -> Fut + Send + Sync + 'static,
        patch: impl Fn(Vec<T>) + 'static,
    ) -> Signal<ListState>
    where
        T: Clone + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
        E: Clone
            + std::fmt::Display
            + serde::Serialize
            + serde::de::DeserializeOwned
            + Send
            + Sync
            + 'static,
        Fut: std::future::Future<Output = Result<Vec<T>, E>> + Send + 'static,
    {
        let resource = self.resource(fetch);
        let state = RwSignal::new(ListState::Loading);
        Effect::new(move |_| match resource.get() {
            None => {}
            Some(Ok(rows)) => {
                let empty = rows.is_empty();
                patch(rows);
                state.set(if empty {
                    ListState::Empty
                } else {
                    ListState::Loaded
                });
            }
            Some(Err(e)) => state.set(ListState::Error(e.to_string())),
        });
        state.into()
    }

    /// A [`Signal`] that stickily retains the latest resolved *result* of a refetch of `fetch`
    /// (revalidated by this invalidator): `None` until the first resolve, then `Some(Ok(v))`
    /// on success or `Some(Err(msg))` on failure — retained across a pending refetch, so a
    /// mutation-triggered refetch never blanks the view back to "Loading…". The fetch error is
    /// **surfaced** (`Err`) for the caller to render, never swallowed into a default: a
    /// swallowed error silently misrepresents state (#346). The flat peer of
    /// [`patched`](Self::patched), which is for keyed stores.
    ///
    /// Client-only: it drives a `server_resource` refetch through a browser-only `Effect`,
    /// so it is exercised by the audiences e2e, not host tests.
    #[must_use]
    #[client_only]
    pub fn sticky<T, Fut, E>(
        &self,
        fetch: impl Fn() -> Fut + Send + Sync + 'static,
    ) -> Signal<Option<Result<T, String>>>
    where
        T: Clone + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
        E: Clone
            + std::fmt::Display
            + serde::Serialize
            + serde::de::DeserializeOwned
            + Send
            + Sync
            + 'static,
        Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
    {
        let resource = self.resource(fetch);
        let signal = RwSignal::new(None::<Result<T, String>>);
        Effect::new(move |_| {
            if let Some(result) = resource.get() {
                signal.set(Some(result.map_err(|e| e.to_string())));
            }
        });
        signal.into()
    }
}

impl Default for Invalidator {
    fn default() -> Self {
        Self::new()
    }
}

/// The load status of a store-backed list, returned by [`Invalidator::patched`]: `Loading`
/// until the first resolve, then `Empty` / `Loaded` per the row count, or `Error` on a failed
/// fetch. Rendered as a sibling to the (unconditionally mounted) list, so the list itself is
/// never inside a branch that could tear it down on a refetch. Derive-only.
#[derive(Clone, Debug)]
pub enum ListState {
    /// The first fetch has not resolved yet.
    Loading,
    /// Resolved with no rows.
    Empty,
    /// Resolved with at least one row.
    Loaded,
    /// The fetch failed; the payload is the error's `Display`.
    Error(String),
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
