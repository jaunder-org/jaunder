use leptos::prelude::*;

/// Register an `Effect` that runs `on_ok` with the resolved value each time `resolved`
/// settles to a success.
///
/// Every async lifecycle hook in this vertical spelled out the same shape —
/// `if let Some(Ok(v)) = <resource-or-action>.get() { … }` — a branch over "not yet"
/// and "failed" that says nothing about the component it sat in. Taking the read as a
/// closure serves both `Resource::get` and `ServerAction::value().get()` without naming
/// either type, and keeps the branch out of the component bodies (#306). The read stays
/// *inside* the effect, so the reactive dependency is unchanged.
pub(super) fn on_settled_ok<T, E, R, F>(resolved: R, on_ok: F)
where
    R: Fn() -> Option<Result<T, E>> + 'static,
    F: Fn(T) + 'static,
{
    Effect::new(move |_| {
        if let Some(value) = resolved().and_then(Result::ok) {
            on_ok(value);
        }
    });
}

/// Register an `Effect` that runs for either settled result, leaving the caller's
/// outcome algebra intact when failure itself affects a host-tested decision.
pub(super) fn on_settled<T, E, R, F>(resolved: R, on_settled: F)
where
    R: Fn() -> Option<Result<T, E>> + 'static,
    F: Fn(Result<T, E>) + 'static,
{
    Effect::new(move |_| {
        if let Some(value) = resolved() {
            on_settled(value);
        }
    });
}
