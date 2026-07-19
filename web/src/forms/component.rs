use std::fmt::Display;
use std::str::FromStr;

use leptos::prelude::*;

use super::field::Field;

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
    let describedby = help.map(|_| format!("{name}-help"));
    view! {
        <label class=field_class>
            <span class="j-form-label">{label}</span>
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
            {help
                .map(|text| {
                    view! {
                        <span id=format!("{name}-help") class="j-form-help">
                            {text}
                        </span>
                    }
                })}
            {move || {
                field
                    .is_touched()
                    .then(|| field.error.get())
                    .flatten()
                    .map(|msg| view! { <p class="error">{msg}</p> })
            }}
        </label>
    }
}
