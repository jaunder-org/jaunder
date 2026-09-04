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

/// A validated form field whose current error is derived from its live input.
///
/// The reactive primitives are private: consumers may replace the input through
/// [`Self::set_value`] and observe the read-only error through [`Self::error`],
/// but cannot create a stale value/error pair. `touched` gates only whether the
/// current message is shown.
pub struct Field<T: 'static> {
    value: RwSignal<String>,
    error: Memo<Option<String>>,
    touched: RwSignal<bool>,
    // Parsing needs the same policy as the error memo: blank optional input is
    // valid absence even when `T::from_str` itself accepts an empty string.
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

    /// A required field seeded from `initial`. Its derived error makes a
    /// pristine invalid value immediately gate submission.
    #[must_use]
    pub fn prefilled(initial: &str) -> Self {
        Self::prefilled_with_optionality(initial, false)
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
        Self::prefilled_with_optionality(initial, true)
    }

    fn prefilled_with_optionality(initial: &str, optional: bool) -> Self {
        let value = RwSignal::new(initial.to_owned());
        let error = Memo::new(move |_| {
            let input = value.get();
            // Whitespace-only optional input is "not provided", matching
            // `common::text::non_empty`; required input still uses the newtype.
            if optional && input.trim().is_empty() {
                None
            } else {
                field_error::<T>(&input)
            }
        });
        Self {
            value,
            error,
            touched: RwSignal::new(false),
            optional,
            _ty: PhantomData,
        }
    }

    /// A snapshot of the current input value.
    #[must_use]
    pub fn value(&self) -> String {
        self.value_signal().get()
    }

    /// Replace the current input without marking the field touched.
    pub fn set_value(&self, input: &str) {
        self.value.set(input.to_owned());
    }

    /// The read-only validation error derived from the current input.
    #[must_use]
    pub fn error(&self) -> Memo<Option<String>> {
        self.error
    }

    /// The private binding handle used by the standard form controls.
    pub(super) fn value_signal(&self) -> RwSignal<String> {
        self.value
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.error.get().is_none()
    }

    /// The current parsed value. Blank optional input is valid absence even
    /// when `T::from_str` accepts an empty string.
    #[must_use]
    pub fn parsed(&self) -> Option<T> {
        let input = self.value.get();
        if self.optional && input.trim().is_empty() {
            None
        } else {
            input.parse::<T>().ok()
        }
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
        self.touched.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `common` has no top-level re-exports — qualify by module.
    use common::audience::AudienceName;
    use common::backup::BackupSchedule;
    use common::bio::{Bio, MAX_BIO_CHARS};
    use common::display_name::DisplayName;
    use common::email::Email;
    use common::post_summary::{MAX_POST_SUMMARY_CHARS, PostSummary};
    use common::slug::Slug;
    use common::tag::Tag;
    use common::test_support::parse_username;
    use common::username::Username;
    use host::password::Password;
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
        assert!(
            field_error::<Email>("not-an-email")
                .is_some_and(|m| m.starts_with("invalid email address"))
        );
        // An empty / whitespace-only audience name yields the newtype's own message.
        assert_eq!(
            field_error::<AudienceName>("   ").as_deref(),
            Some("audience name must not be empty")
        );
        // `BackupSchedule`'s message carries croner's reason after our label, so assert the
        // prefix rather than couple to the crate's wording.
        assert!(
            field_error::<BackupSchedule>("not a cron")
                .is_some_and(|m| m.starts_with("invalid backup schedule"))
        );
        assert!(field_error::<DisplayName>("").is_some()); // empty
        assert!(field_error::<DisplayName>(&"a".repeat(256)).is_some()); // over 255
    }

    // `Field<T>`'s methods are signal-only (no `Effect`/`Resource`), so — like
    // `Invalidator::{new, notify, track}` — they are host-tested under an `Owner`
    // rather than left to e2e.

    #[test]
    fn post_summary_field_error_reports_over_cap_and_allows_empty_when_optional() {
        // Over-cap is the newtype's own `FromStr::Err` message; a valid summary is None.
        let over = "a".repeat(MAX_POST_SUMMARY_CHARS + 1);
        assert!(field_error::<PostSummary>(&over).is_some());
        assert_eq!(field_error::<PostSummary>("A short summary"), None);
        // Under `Field::optional`, an empty summary is "not provided" (valid → None);
        // a non-empty over-cap value still gates submit.
        Owner::new().with(|| {
            let field = Field::<PostSummary>::optional();
            assert_eq!(field.error().get(), None);
            field.set_value(&over);
            assert!(field.error().get().is_some());
            field.set_value("A short summary");
            assert_eq!(field.error().get(), None);
        });
    }

    #[test]
    fn bio_field_error_reports_over_cap_and_allows_empty_when_optional() {
        // Over-cap is the newtype's own `FromStr::Err` message; a valid bio is None.
        let over = "a".repeat(MAX_BIO_CHARS + 1);
        assert!(field_error::<Bio>(&over).is_some());
        assert_eq!(field_error::<Bio>("About me"), None);
        // Under `Field::optional`, an empty bio is "not provided" (valid → None);
        // a non-empty over-cap value still gates submit.
        Owner::new().with(|| {
            let field = Field::<Bio>::optional();
            assert_eq!(field.error().get(), None);
            field.set_value(&over);
            assert!(field.error().get().is_some());
            field.set_value("About me");
            assert_eq!(field.error().get(), None);
        });
    }

    #[test]
    fn programmatic_value_write_recomputes_required_validation() {
        Owner::new().with(|| {
            let field = Field::<Username>::new();

            assert!(!field.is_valid());
            assert!(!field.is_touched());
            assert_eq!(field.parsed(), None);

            field.set_value("alice");
            assert!(field.is_valid());
            assert_eq!(field.parsed(), Some(parse_username("alice")));
            assert!(!field.is_touched());

            field.set_value("not a username");
            assert!(!field.is_valid());
            assert_eq!(field.parsed(), None);
        });
    }

    #[test]
    fn field_prefilled_seeds_from_initial_and_aliases_on_copy() {
        Owner::new().with(|| {
            let f = Field::<Username>::prefilled("alice");
            assert!(f.is_valid());
            assert_eq!(f.value(), "alice");
            // `Copy` and the hand-written `Clone` both alias the same underlying state.
            let c = Clone::clone(&f);
            c.set_value("not a username");
            assert_eq!(f.value(), "not a username");
            assert!(!f.is_valid());
        });
    }

    #[test]
    fn field_default_matches_new() {
        Owner::new().with(|| {
            assert!(!Field::<Username>::default().is_valid());
        });
    }

    // Optional fields (#408): empty is *valid* (e.g. an auto-generated slug
    // override), so a pristine empty optional field leaves submit enabled, while a
    // non-empty invalid entry still gates it.
    #[test]
    fn optional_empty_field_is_valid_and_submittable() {
        Owner::new().with(|| {
            let f = Field::<Slug>::optional();
            assert!(f.is_valid()); // empty optional ⇒ valid ⇒ submit not gated
            assert_eq!(f.error().get(), None);
            assert!(!f.is_touched());
            assert_eq!(f.parsed(), None); // Option<Slug> None for empty
            f.set_value("   ");
            assert!(f.is_valid()); // whitespace-only ⇒ not provided ⇒ valid
            assert_eq!(f.error().get(), None);
            assert_eq!(f.parsed(), None);
        });
    }

    #[test]
    fn optional_absence_precedes_a_permissive_parser() {
        Owner::new().with(|| {
            let field = Field::<String>::optional();

            assert_eq!(field.parsed(), None);
            field.set_value("   ");
            assert_eq!(field.parsed(), None);
            field.set_value("present");
            assert_eq!(field.parsed(), Some("present".to_owned()));
        });
    }

    #[test]
    fn optional_nonempty_invalid_shows_the_newtypes_message() {
        Owner::new().with(|| {
            let f = Field::<Slug>::optional();
            f.set_value("Bad Slug!");
            assert!(!f.is_valid());
            assert!(f.error().get().is_some()); // exactly InvalidSlug's Display
        });
    }

    #[test]
    fn optional_nonempty_valid_parses() {
        Owner::new().with(|| {
            let f = Field::<Slug>::optional();
            f.set_value("hello");
            assert!(f.is_valid());
            assert_eq!(f.parsed(), "hello".parse::<Slug>().ok());
        });
    }

    #[test]
    fn optional_prefilled_seeds_valid_from_existing_slug() {
        Owner::new().with(|| {
            let f = Field::<Slug>::optional_prefilled("my-post");
            assert!(f.is_valid());
            assert_eq!(f.value(), "my-post");
        });
    }

    #[test]
    fn optional_reset_restores_valid_untouched_absence() {
        Owner::new().with(|| {
            let field = Field::<Slug>::optional();
            field.set_value("Bad Slug!");
            field.touch();

            field.reset();

            assert_eq!(field.value(), "");
            assert!(field.is_valid());
            assert_eq!(field.parsed(), None);
            assert_eq!(field.error().get(), None);
            assert!(!field.is_touched());
        });
    }

    #[test]
    fn required_new_still_invalid_on_empty() {
        // Regression: the required path is unchanged — an empty `new()` is invalid.
        Owner::new().with(|| {
            assert!(!Field::<Slug>::new().is_valid());
        });
    }

    #[test]
    fn set_value_honors_optionality() {
        Owner::new().with(|| {
            let optional = Field::<Slug>::optional();
            optional.set_value("");
            assert!(optional.is_valid(), "an empty optional field is valid");

            let required = Field::<Slug>::new();
            required.set_value("");
            assert!(!required.is_valid(), "an empty required field is not valid");
        });
    }

    #[test]
    fn set_value_does_not_touch_the_field() {
        // Seeding is not interaction: touching here would flash an error on editor load.
        Owner::new().with(|| {
            let field = Field::<Slug>::new();
            field.set_value("Bad Slug!");
            assert!(!field.is_touched());
        });
    }
}
