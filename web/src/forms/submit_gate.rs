use leptos::prelude::*;

/// Couples a form's disabled state and dispatched payload to one request
/// constructor (ADR-0113). The constructor may parse any number or shape of
/// fields; the sole no-request arm stays here beside the gate.
#[must_use]
pub(super) fn request_submit_gate<R>(
    pending: Signal<bool>,
    request: Callback<(), Option<R>>,
    on_submit: Callback<R>,
) -> (Signal<bool>, Callback<()>)
where
    R: 'static,
{
    let disabled = Signal::derive(move || pending.get() || request.run(()).is_none());
    let submit = Callback::new(move |()| {
        if !pending.get()
            && let Some(request) = request.run(())
        {
            on_submit.run(request);
        }
    });
    (disabled, submit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_constructor_controls_gate_and_payload() {
        Owner::new().with(|| {
            let input = RwSignal::new(String::new());
            let pending = RwSignal::new(false);
            let seen = RwSignal::new(None::<String>);
            let (disabled, submit) = request_submit_gate(
                pending.into(),
                Callback::new(move |()| {
                    input.with(|value| (!value.is_empty()).then(|| value.clone()))
                }),
                Callback::new(move |value| seen.set(Some(value))),
            );

            assert!(disabled.get());
            submit.run(());
            assert_eq!(seen.get(), None);

            input.set("request".to_owned());
            assert!(!disabled.get());
            submit.run(());
            assert_eq!(seen.get().as_deref(), Some("request"));

            seen.set(None);
            pending.set(true);
            assert!(disabled.get());
            submit.run(());
            assert_eq!(seen.get(), None);
        });
    }
}
