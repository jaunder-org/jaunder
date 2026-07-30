use std::fmt::Display;
use std::str::FromStr;

use leptos::prelude::*;

use super::field::Field;

/// The chrome shared by every ADR-0065 validated field: the wrapping `<label>` — which
/// associates the control *implicitly*, so no `for=`/`id=` pair is emitted and none can
/// drift — the label text, an optional help line, and the touched-gated inline message.
/// The control itself is supplied by the caller as `children`.
///
/// Deliberately **not** generic over the field's `T`. Taking `Field<T>` would be the
/// tidier signature, but it would make this the repo's first generic component *with
/// children*, and a generic close tag must match its opening generics token-for-token —
/// while leptosfmt (which `cargo xtask check` runs in fix mode) writes generic tags with
/// a trailing comma. A formatter pass could then unbalance a hand-matched pair. Taking
/// the validity as two erased signals keeps the touched-gate in exactly one place with
/// no such hazard.
#[component]
fn Labelled(
    label: &'static str,
    name: &'static str,
    /// The wrapping `<label>`'s class — always supplied by the shell, which owns the
    /// `j-form-field` default, so a call site can slot the field into a bespoke layout.
    field_class: &'static str,
    /// The field's true validity (`None` = valid), independent of whether it is shown.
    #[prop(into)]
    error: Signal<Option<String>>,
    /// Whether the field has been blurred — gates only whether `error` is *displayed*.
    touched: Signal<bool>,
    /// Optional hint line rendered under the control and wired to it via
    /// `aria-describedby` (id `{name}-help`), for a field whose format needs explaining.
    #[prop(optional_no_strip)]
    help: Option<&'static str>,
    children: Children,
) -> impl IntoView {
    view! {
        <label class=field_class>
            <span class="j-form-label">{label}</span>
            {children()}
            {help
                .map(|text| {
                    view! {
                        <span id=format!("{name}-help") class="j-form-help">
                            {text}
                        </span>
                    }
                })}
            {move || {
                touched
                    .get()
                    .then(|| error.get())
                    .flatten()
                    .map(|msg| view! { <p class="error">{msg}</p> })
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
    // Only wire `aria-describedby` when a help line is actually rendered (its id must resolve).
    // Derived from `name` here and again in `Labelled`: the attribute belongs on the control
    // while the help span lives in the chrome, and leptos `children` is opaque, so the id
    // cannot be handed down without turning children into a render prop.
    let describedby = help.map(|_| format!("{name}-help"));
    view! {
        <Labelled
            label=label
            name=name
            field_class=field_class
            error=field.error
            touched=Signal::derive(move || field.is_touched())
            help=help
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
