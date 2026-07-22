//! Reactive revalidation helpers — the browser-bound `Effect`/`Resource` plumbing behind
//! `web`'s `Invalidator` idiom (ADR-0060/0061). These are free functions rather than
//! `Invalidator` methods because `client` cannot depend on `web` (the dependency runs
//! `web → client`, and orphan rules forbid `impl`-ing a `web` type here) — each takes the
//! invalidator's `track` / `notify` as closures. Wasm-only, e2e-exercised (the audiences
//! revalidation flows), never host-tested.

use common::list_state::ListState;
use leptos::prelude::*;
use leptos::server_fn::ServerFn;

/// A [`Resource`] that refetches whenever `track`'s revision changes. The fetcher is
/// nullary — the revision counter is an internal detail callers never see.
#[must_use]
pub fn resource<T, Fut>(
    track: impl Fn() -> u32 + Send + Sync + 'static,
    fetch: impl Fn() -> Fut + Send + Sync + 'static,
) -> Resource<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    Resource::new(track, move |_| fetch())
}

/// A [`ServerAction`] that fires `notify` on **success** — after the mutation commits
/// (`Some(Ok(_))`), never on dispatch or on failure. The gating `Effect` is wired
/// internally, so no caller writes it (ADR-0060 §2).
#[must_use]
pub fn action<A>(notify: impl Fn() + Send + Sync + 'static) -> ServerAction<A>
where
    A: ServerFn + Send + Sync + Clone + 'static,
    A::Output: Send + Sync + 'static,
    A::Error: Send + Sync + 'static,
{
    let action = ServerAction::<A>::new();
    Effect::new(move |_| {
        if action.value().with(|v| matches!(v, Some(Ok(_)))) {
            notify();
        }
    });
    action
}

/// Drives a keyed [`reactive_stores`](https://docs.rs/reactive_stores) list from a refetch of
/// `fetch` (revalidated by `track`). On each successful refetch it hands the rows to `patch` —
/// supplied as a closure so the caller's concrete keyed field runs its **in-place** `patch` (a
/// generic bound would instead resolve to the unkeyed, positional patch and lose per-row
/// identity). Returns the list's [`ListState`]; a pending or failed refetch never patches, so
/// the last-good rows are retained (ADR-0061).
#[must_use]
pub fn patched<T, Fut, E>(
    track: impl Fn() -> u32 + Send + Sync + 'static,
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
    let res = resource(track, fetch);
    let state = RwSignal::new(ListState::Loading);
    Effect::new(move |_| match res.get() {
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
/// (revalidated by `track`): `None` until the first resolve, then `Some(Ok(v))` on success or
/// `Some(Err(e))` on failure (the caller's error type `E`, preserved) — retained across a
/// pending refetch, so a mutation-triggered refetch never blanks the view back to "Loading…".
/// The fetch error is **surfaced** (`Err`) for the caller to render, never swallowed into a
/// default: a swallowed error silently misrepresents state (#346). The flat peer of
/// [`patched`], which is for keyed stores.
#[must_use]
pub fn sticky<T, Fut, E>(
    track: impl Fn() -> u32 + Send + Sync + 'static,
    fetch: impl Fn() -> Fut + Send + Sync + 'static,
) -> Signal<Option<Result<T, E>>>
where
    T: Clone + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    E: Clone + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    let res = resource(track, fetch);
    let signal = RwSignal::new(None::<Result<T, E>>);
    Effect::new(move |_| {
        if let Some(result) = res.get() {
            signal.set(Some(result));
        }
    });
    signal.into()
}
