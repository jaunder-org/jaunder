//! Client-side domain-value form validation (#414): validate a field by parsing input
//! into a domain newtype — the same `FromStr` the typed `#[server]`-arg `Deserialize`
//! routes through — and surface the newtype's own message inline. See ADR (draft):
//! `docs/adr/0065-client-side-domain-validation.md`.

use std::fmt::Display;
use std::marker::PhantomData;
use std::str::FromStr;

use leptos::prelude::*;

/// `None` when `input` parses into the domain newtype `T`; otherwise the newtype's own
/// validation message (its `FromStr::Err` `Display`). The single client/server validation
/// source — re-implementing a newtype's rule in the client is prohibited (#416).
#[must_use]
pub fn field_error<T>(input: &str) -> Option<String>
where
    T: FromStr,
    T::Err: Display,
{
    input.parse::<T>().err().map(|e| e.to_string())
}

/// A validated form field: its live input value + current validation error, bundled so a
/// form declares one `Copy` handle per field. `error` always holds the true validity
/// (`None` = valid); `touched` gates only whether the message is *shown*.
pub struct Field<T: 'static> {
    pub value: RwSignal<String>,
    pub error: RwSignal<Option<String>>,
    touched: RwSignal<bool>,
    // `fn() -> T`, not `T`: a phantom marker that owns no `T`, so `Field<T>` is
    // unconditionally `Send`/`Sync`/`Copy` (the reactive closures leptos builds must be
    // `Send`) — `PhantomData<T>` would spuriously couple those to `T`'s own bounds.
    _ty: PhantomData<fn() -> T>,
}

// Hand-written, unconditional: `Field` holds no `T` by value (only the `fn() -> T` phantom),
// so it is `Copy` for every `T`. A `#[derive]` would wrongly demand `T: Copy`/`T: Clone`,
// which the `String`-backed newtypes don't satisfy.
impl<T> Copy for Field<T> {}
impl<T> Clone for Field<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Default for Field<T>
where
    T: FromStr + 'static,
    T::Err: Display,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Field<T>
where
    T: FromStr + 'static,
    T::Err: Display,
{
    #[must_use]
    pub fn new() -> Self {
        Self::prefilled("")
    }

    /// Seed `error` from `initial` so a pristine field is already invalid — disable-until-valid
    /// must gate the empty form.
    #[must_use]
    pub fn prefilled(initial: &str) -> Self {
        Self {
            value: RwSignal::new(initial.to_owned()),
            error: RwSignal::new(field_error::<T>(initial)),
            touched: RwSignal::new(false),
            _ty: PhantomData,
        }
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.error.get().is_none()
    }

    /// The already-parsed value (`None` if invalid) — the seam for request-aggregate DTOs (#417).
    #[must_use]
    pub fn parsed(&self) -> Option<T> {
        self.value.get().parse::<T>().ok()
    }

    #[must_use]
    pub fn is_touched(&self) -> bool {
        self.touched.get()
    }

    pub fn touch(&self) {
        self.touched.set(true);
    }

    pub fn reset(&self) {
        self.value.set(String::new());
        self.error.set(field_error::<T>(""));
        self.touched.set(false);
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
        field.error.set(field_error::<T>(&v));
    };
    view! {
        <label class="j-form-field">
            <span class="j-form-label">{label}</span>
            <input
                class="j-form-input"
                type=input_type
                name=name
                autocomplete=autocomplete
                prop:value=field.value
                on:input=on_input
                on:blur=move |_| field.touch()
            />
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

#[cfg(test)]
mod tests {
    use super::*;
    // `common` has no top-level re-exports — qualify by module.
    use common::password::Password;
    use common::slug::Slug;
    use common::tag::Tag;
    use common::username::Username;
    use leptos::reactive::owner::Owner;

    #[test]
    fn valid_input_is_none() {
        assert_eq!(field_error::<Username>("alice"), None);
        assert_eq!(field_error::<Tag>("rust"), None);
        assert_eq!(field_error::<Slug>("hello"), None);
        assert_eq!(field_error::<Password>("hunter2!"), None); // >= 8 chars
    }

    #[test]
    fn invalid_input_is_the_newtypes_own_message() {
        // The message is exactly the newtype's `FromStr::Err` `Display` — one source of truth.
        let expected = "username must be non-empty and match [a-z0-9_-]+";
        assert_eq!(field_error::<Username>("a b").as_deref(), Some(expected));
        assert_eq!(field_error::<Username>("").as_deref(), Some(expected));
        assert!(field_error::<Password>("short").is_some()); // < 8 chars
        assert!(field_error::<Tag>("Bad Tag").is_some());
    }

    // `Field<T>`'s methods are signal-only (no `Effect`/`Resource`), so — like
    // `Invalidator::{new, notify, track}` — they are host-tested under an `Owner`, not
    // `#[client_only]`-exempted.

    #[test]
    fn field_seeds_validity_and_tracks_input() {
        let owner = Owner::new();
        owner.set();
        let f = Field::<Username>::new();
        // A pristine empty field is seeded invalid, so disable-until-valid gates the empty form.
        assert!(!f.is_valid());
        assert!(!f.is_touched());
        assert_eq!(f.parsed(), None);
        // Mimic the component's on:input: set the value, recompute the error.
        f.value.set("alice".to_owned());
        f.error.set(field_error::<Username>("alice"));
        assert!(f.is_valid());
        assert_eq!(f.parsed(), "alice".parse::<Username>().ok());
        f.touch();
        assert!(f.is_touched());
        f.reset();
        assert!(!f.is_valid());
        assert!(!f.is_touched());
        assert_eq!(f.value.get(), "");
        drop(owner);
    }

    #[test]
    fn field_prefilled_seeds_from_initial_and_aliases_on_copy() {
        let owner = Owner::new();
        owner.set();
        let f = Field::<Username>::prefilled("alice");
        assert!(f.is_valid());
        assert_eq!(f.value.get(), "alice");
        // `Copy` and the hand-written `Clone` both alias the same underlying signals.
        let c = Clone::clone(&f);
        c.value.set("bob".to_owned());
        assert_eq!(f.value.get(), "bob");
        drop(owner);
    }

    #[test]
    fn field_default_matches_new() {
        let owner = Owner::new();
        owner.set();
        assert!(!Field::<Username>::default().is_valid());
        drop(owner);
    }
}
