use client::telemetry;
use common::{MutationOutcome, client_telemetry::ClientErrorContext};
use leptos::prelude::*;

use crate::error::WebError;

/// Renders only non-confirmed mutation feedback for Post controls.
pub(super) fn mutation_feedback<T>(
    result: Result<MutationOutcome<T>, WebError>,
    indeterminate_message: &'static str,
) -> Option<AnyView> {
    match crate::mutation_feedback::classify(result, indeterminate_message) {
        crate::mutation_feedback::MutationFeedback::Confirmed(_) => None,
        crate::mutation_feedback::MutationFeedback::Error(message) => {
            Some(view! { <p class="error">{message}</p> }.into_any())
        }
    }
}

/// Dispatches a destructive Post action only after browser confirmation.
///
/// A dialog transport failure is intentionally reported through the client
/// telemetry boundary rather than pretending the user canceled.
pub(super) fn dispatch_after_confirm(
    message: &str,
    context: ClientErrorContext,
    dispatch: impl FnOnce(),
) {
    match client::dialog::confirm(message) {
        Ok(outcome) => {
            if outcome.should_dispatch() {
                dispatch();
            }
        }
        Err(error) => {
            let source_kind = error.source_kind();
            telemetry::report_swallowed(telemetry::error_kind(source_kind), context, source_kind);
        }
    }
}

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
