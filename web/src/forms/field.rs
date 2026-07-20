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
    // When set, an *empty* input is valid (the field may be omitted, e.g. an
    // auto-generated slug override). A non-empty value is validated as normal.
    optional: bool,
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
            optional: false,
            _ty: PhantomData,
        }
    }

    /// An *optional* field: an empty value is valid (the field may be left blank),
    /// so `is_valid()` leaves submit enabled for a pristine empty field. A non-empty
    /// value is validated through the newtype's `FromStr` as normal. First adopter:
    /// `slug_override` (#408).
    #[must_use]
    pub fn optional() -> Self {
        Self::optional_prefilled("")
    }

    /// An optional field seeded from `initial` (empty ⇒ valid; non-empty validated).
    #[must_use]
    pub fn optional_prefilled(initial: &str) -> Self {
        let field = Self {
            value: RwSignal::new(initial.to_owned()),
            error: RwSignal::new(None),
            touched: RwSignal::new(false),
            optional: true,
            _ty: PhantomData,
        };
        // Seed validity through `error_for` (one empty-vs-validate rule, not two).
        field.error.set(field.error_for(initial));
        field
    }

    /// Optionality-aware validation: an empty *optional* field is valid (`None`);
    /// otherwise the newtype's `FromStr` error via [`field_error`]. The component's
    /// on-input handler routes through this so rendered validity honors optionality.
    #[must_use]
    pub fn error_for(&self, input: &str) -> Option<String> {
        // Trim before the empty check so a whitespace-only optional field reads as
        // "not provided" (valid) — matching the `common::text::non_empty` behavior
        // the pre-typing `slug_override` form used.
        if self.optional && input.trim().is_empty() {
            None
        } else {
            field_error::<T>(input)
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

#[cfg(test)]
mod tests {
    use super::*;
    // `common` has no top-level re-exports — qualify by module.
    use common::audience::AudienceName;
    use common::backup::BackupSchedule;
    use common::display_name::DisplayName;
    use common::email::Email;
    use common::password::Password;
    use common::slug::Slug;
    use common::tag::Tag;
    use common::test_support::parse_username;
    use common::username::Username;
    use leptos::reactive::owner::Owner;

    #[test]
    fn valid_input_is_none() {
        assert_eq!(field_error::<Username>("alice"), None);
        assert_eq!(field_error::<Tag>("rust"), None);
        assert_eq!(field_error::<Slug>("hello"), None);
        assert_eq!(field_error::<Password>("hunter2!"), None); // >= 8 chars
        assert_eq!(field_error::<Email>("user@example.com"), None);
        assert_eq!(field_error::<AudienceName>("Close Friends"), None);
        assert_eq!(field_error::<BackupSchedule>("0 0 0 * * *"), None); // six-field cron
        assert_eq!(field_error::<DisplayName>("Ada Lovelace"), None);
    }

    #[test]
    fn invalid_input_is_the_newtypes_own_message() {
        // The message is exactly the newtype's `FromStr::Err` `Display` — one source of truth.
        let expected = "username must be non-empty and match [a-z0-9_-]+";
        assert_eq!(field_error::<Username>("a b").as_deref(), Some(expected));
        assert_eq!(field_error::<Username>("").as_deref(), Some(expected));
        assert!(field_error::<Password>("short").is_some()); // < 8 chars
        assert!(field_error::<Tag>("Bad Tag").is_some());
        // `Email`'s message carries the underlying `email_address` reason after our
        // label, so assert the prefix rather than couple to the crate's wording.
        assert!(field_error::<Email>("not-an-email")
            .is_some_and(|m| m.starts_with("invalid email address")));
        // An empty / whitespace-only audience name yields the newtype's own message.
        assert_eq!(
            field_error::<AudienceName>("   ").as_deref(),
            Some("audience name must not be empty")
        );
        // `BackupSchedule`'s message carries croner's reason after our label, so assert the
        // prefix rather than couple to the crate's wording.
        assert!(field_error::<BackupSchedule>("not a cron")
            .is_some_and(|m| m.starts_with("invalid backup schedule")));
        assert!(field_error::<DisplayName>("").is_some()); // empty
        assert!(field_error::<DisplayName>(&"a".repeat(256)).is_some()); // over 255
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
        assert_eq!(f.parsed(), Some(parse_username("alice")));
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

    // Optional fields (#408): empty is *valid* (e.g. an auto-generated slug
    // override), so a pristine empty optional field leaves submit enabled, while a
    // non-empty invalid entry still gates it.
    #[test]
    fn optional_empty_field_is_valid_and_submittable() {
        let owner = Owner::new();
        owner.set();
        let f = Field::<Slug>::optional();
        assert!(f.is_valid()); // empty optional ⇒ valid ⇒ submit not gated
        assert_eq!(f.error_for(""), None); // the optional empty-is-valid branch
        assert_eq!(f.error_for("   "), None); // whitespace-only ⇒ not provided ⇒ valid
        assert!(!f.is_touched());
        assert_eq!(f.parsed(), None); // Option<Slug> None for empty
        drop(owner);
    }

    #[test]
    fn optional_nonempty_invalid_shows_the_newtypes_message() {
        let owner = Owner::new();
        owner.set();
        let f = Field::<Slug>::optional();
        // Mimic on:input with a bad slug.
        f.value.set("Bad Slug!".to_owned());
        f.error.set(f.error_for("Bad Slug!"));
        assert!(!f.is_valid());
        assert!(f.error.get().is_some()); // exactly InvalidSlug's Display
        drop(owner);
    }

    #[test]
    fn optional_nonempty_valid_parses() {
        let owner = Owner::new();
        owner.set();
        let f = Field::<Slug>::optional();
        f.value.set("hello".to_owned());
        f.error.set(f.error_for("hello"));
        assert!(f.is_valid());
        assert_eq!(f.parsed(), "hello".parse::<Slug>().ok());
        drop(owner);
    }

    #[test]
    fn optional_prefilled_seeds_valid_from_existing_slug() {
        let owner = Owner::new();
        owner.set();
        let f = Field::<Slug>::optional_prefilled("my-post");
        assert!(f.is_valid());
        assert_eq!(f.value.get(), "my-post");
        drop(owner);
    }

    #[test]
    fn required_new_still_invalid_on_empty() {
        // Regression: the required path is unchanged — an empty `new()` is invalid.
        let owner = Owner::new();
        owner.set();
        assert!(!Field::<Slug>::new().is_valid());
        drop(owner);
    }
}
