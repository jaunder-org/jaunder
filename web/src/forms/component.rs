use std::fmt::Display;
use std::str::FromStr;

use leptos::prelude::*;

use super::field::Field;
use super::submit_gate::request_submit_gate;
use server_fn::ServerFn;

/// Wire a native form to a generated [`ServerAction`] through one input constructor.
///
/// `request` returns the complete generated server-function input `S`. The action
/// therefore supplies its own pending state and dispatch operation; callers supply
/// only the operation-specific construction of a validated request. The returned
/// handler owns the native-form `prevent_default` glue.
///
/// [`request_submit_gate`] remains the host-tested policy seam: the same constructor
/// controls both disabled state and dispatch, and a pending or invalid form dispatches
/// nothing. This wasm-only adapter removes the repeated Leptos wiring from each form.
pub fn server_action_submit<S, F>(
    action: ServerAction<S>,
    request: F,
) -> (Signal<bool>, impl Fn(leptos::ev::SubmitEvent))
where
    S: ServerFn + Send + Sync + Clone + 'static,
    S::Output: Send + Sync + 'static,
    S::Error: Send + Sync + 'static,
    F: Fn() -> Option<S> + Send + Sync + 'static,
{
    let (disabled, dispatch) = request_submit_gate(
        action.pending().into(),
        Callback::new(move |()| request()),
        Callback::new(move |input| {
            action.dispatch(input);
        }),
    );
    let submit = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        dispatch.run(());
    };
    (disabled, submit)
}

/// The chrome shared by every ADR-0065 validated field: the wrapping `<label>` — which
/// associates the control *implicitly*, so no `for=`/`id=` pair is emitted and none can
/// drift — the label text, an optional help line, and the touched-gated inline message.
/// The control itself is supplied by the caller as `children`.
///
/// Deliberately **not** generic over the field's `T`. Taking `Field<T>` would be the
/// tidier signature, but it would make this the repo's first generic component *with
/// children*, and a generic close tag must match its opening generics token-for-token.
/// Taking the validity as two erased signals keeps the touched-gate in exactly one
/// place (docs/adr/0117-labelled-takes-erased-signals.md — including the open
/// question of whether that burden still justifies this shape).
#[component]
fn Labelled(
    label: &'static str,
    /// The wrapping `<label>`'s class — always supplied by the shell, which owns the
    /// `j-form-field` default, so a call site can slot the field into a bespoke layout.
    field_class: &'static str,
    /// The field's true validity (`None` = valid), independent of whether it is shown.
    #[prop(into)]
    error: Signal<Option<String>>,
    /// Whether the field has been blurred — gates only whether `error` is *displayed*.
    touched: Signal<bool>,
    /// Optional hint line rendered under the control, for a field whose format needs
    /// explaining. `optional_no_strip` (not the usual `optional`): the shells hold their
    /// own `help` as an `Option` and forward it as-is, and a plain `#[prop(optional)]` on
    /// an `Option<_>` generates a `strip_option` setter that takes the *inner* type, which
    /// would not accept it.
    #[prop(optional_no_strip)]
    help: Option<&'static str>,
    /// The help span's DOM id — the SAME value the shell puts in its control's
    /// `aria-describedby`, passed down rather than re-derived so the pair is
    /// single-source and cannot drift. Ignored unless `help` is set.
    #[prop(optional_no_strip)]
    help_id: Option<String>,
    children: Children,
) -> impl IntoView {
    view! {
        <label class=field_class>
            <span class="j-form-label">{label}</span>
            {children()}
            {help
                .zip(help_id)
                .map(|(text, id)| {
                    view! {
                        <span id=id class="j-form-help">
                            {text}
                        </span>
                    }
                })}
            {move || {
                touched
                    .get()
                    .then(|| error.get())
                    .flatten()
                    .map(|msg| {
                        view! { <span class="error">{msg}</span> }
                    })
            }}
        </label>
    }
}

/// A labelled input bound to a [`Field<T>`]: validates on input via [`field_error`], and
/// shows the newtype's own message inline once the field is touched (blur). `name` MUST match
/// the `#[server]` struct field and the e2e selector.
#[component]
pub fn ValidatedInput<T>(
    label: &'static str,
    name: &'static str,
    field: Field<T>,
    #[prop(default = "text")] input_type: &'static str,
    #[prop(optional)] autocomplete: Option<&'static str>,
    /// Override the wrapping `<label>` class (default `j-form-field`) so the field slots into a
    /// bespoke layout — e.g. a grid cell that must span full width.
    #[prop(default = "j-form-field")]
    field_class: &'static str,
    /// Override the input's CSS class (default `j-form-input`) so a form with bespoke styling
    /// keeps its look; the validation behavior is unchanged.
    #[prop(default = "j-form-input")]
    class: &'static str,
    /// Optional hint line rendered under the input and wired to it via `aria-describedby`
    /// (id `{name}-help`), for a field whose format needs explaining (e.g. a cron expression).
    #[prop(optional)]
    help: Option<&'static str>,
    /// Live input massaging before validation/display, e.g. `transform=str::to_lowercase`
    /// for a username. `fn(&str) -> String`; a call site passes the bare fn (leptos wraps the
    /// optional prop, and the fn-item coerces to the pointer at the known type — an `into`
    /// on the prop would instead block that coercion).
    #[prop(optional)]
    transform: Option<fn(&str) -> String>,
) -> impl IntoView
where
    T: FromStr + 'static,
    T::Err: Display,
{
    let on_input = move |ev| {
        let raw = event_target_value(&ev);
        let v = match transform {
            Some(f) => f(&raw),
            None => raw,
        };
        field.value.set(v.clone());
        field.error.set(field.error_for(&v));
    };
    // Only wire `aria-describedby` when a help line is actually rendered (its id must
    // resolve). Derived once here and handed to `Labelled` as `help_id`: the attribute
    // belongs on the control, the span lives in the chrome, and passing the one value
    // both ways keeps them from drifting.
    let describedby = help.map(|_| format!("{name}-help"));
    view! {
        <Labelled
            label=label
            field_class=field_class
            error=field.error
            touched=Signal::derive(move || field.is_touched())
            help=help
            help_id=describedby.clone()
        >
            <input
                class=class
                type=input_type
                name=name
                autocomplete=autocomplete
                aria-describedby=describedby
                prop:value=field.value
                on:input=on_input
                on:blur=move |_| field.touch()
            />
        </Labelled>
    }
}

/// The multi-line sibling of [`ValidatedInput`]: a labelled `<textarea>` bound to a
/// [`Field<T>`], validating on input and showing the newtype's own message inline once the
/// field is touched (blur). `name` MUST match the `#[server]` struct field and the e2e
/// selector.
///
/// No `id` prop: [`Labelled`]'s wrapping `<label>` associates the control implicitly, so
/// there is no `for=`/`id=` pair to keep in sync. No `transform` prop either — nothing
/// multi-line needs live input massaging the way a username does.
#[component]
pub fn ValidatedTextarea<T>(
    label: &'static str,
    name: &'static str,
    field: Field<T>,
    /// Visible rows. Defaults to 3: the browser default of 2 is too short for the summary
    /// and bio fields this serves.
    #[prop(default = 3)]
    rows: u32,
    #[prop(optional)] placeholder: Option<&'static str>,
    /// Override the wrapping `<label>` class (default `j-form-field`) so the field slots
    /// into a bespoke layout.
    #[prop(default = "j-form-field")]
    field_class: &'static str,
    /// Override the textarea's CSS class (default `j-form-input`) so a form with bespoke
    /// styling keeps its look; the validation behavior is unchanged.
    #[prop(default = "j-form-input")]
    class: &'static str,
    /// Optional hint line rendered under the textarea and wired to it via
    /// `aria-describedby` (id `{name}-help`).
    #[prop(optional)]
    help: Option<&'static str>,
    /// Optional callback fired on every input event, **after** the value and error are
    /// written — so a consumer that reads the field's validity from here sees the new
    /// state. `ComposerFields` forwards the composer's flash-clearing callback through
    /// it (#860); every other call site omits it.
    ///
    /// `optional_no_strip` (not the usual `optional`), for the reason [`Labelled`]'s
    /// `help` documents: `ComposerFields` holds its own `on_input` as an `Option` and
    /// forwards it as-is, and a plain `#[prop(optional)]` on an `Option<_>` generates a
    /// `strip_option` setter that takes the *inner* type, which would not accept it.
    #[prop(optional_no_strip)]
    on_input: Option<Callback<()>>,
) -> impl IntoView
where
    T: FromStr + 'static,
    T::Err: Display,
{
    let handle_input = move |ev| {
        field.set_input(&event_target_value(&ev));
        if let Some(cb) = on_input {
            cb.run(());
        }
    };
    // Only wire `aria-describedby` when a help line is actually rendered (its id must
    // resolve). Derived once and handed down as `help_id`, per `ValidatedInput`.
    let describedby = help.map(|_| format!("{name}-help"));
    view! {
        <Labelled
            label=label
            field_class=field_class
            error=field.error
            touched=Signal::derive(move || field.is_touched())
            help=help
            help_id=describedby.clone()
        >
            <textarea
                class=class
                name=name
                rows=rows
                placeholder=placeholder
                aria-describedby=describedby
                prop:value=field.value
                on:input=handle_input
                on:blur=move |_| field.touch()
            ></textarea>
        </Labelled>
    }
}
