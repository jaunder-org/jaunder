//! The new-post composer's shared reactive state, extracted from the wasm-only
//! `component` so the dispatch payload it builds is **host-tested** rather than left
//! to e2e alone (ADR-0070 §6 — the convention `tags::input_state`,
//! `media::upload_state` and `forms::Field` follow).
//!
//! The composer renders in two shapes (a compact inline row and the full compose
//! page) over one set of signals. Bundling them here is what lets each shape be its
//! own `#[component]` taking a single prop instead of seven, with one `PostInputs`
//! construction instead of a near-identical copy per shape.

use leptos::prelude::*;

use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::render::PostFormat;
use common::seed::{AuthoredPost, TagSummary};
use common::slug::Slug;
use common::time::{self, UtcInstant};
use common::visibility::{AudienceBase, AudienceSelection};

use crate::forms::Field;
use crate::posts::PostInputs;

/// The author's explicit publication choice for a create or update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationIntent {
    Draft,
    PublishNow,
    PublishAt(UtcInstant),
}

/// Map the create form's publish button and optional local schedule to a typed
/// publication choice while preserving its existing invalid-value fallback.
#[must_use]
pub fn publication_from_local(publish: bool, value: &str) -> PublicationIntent {
    if !publish {
        PublicationIntent::Draft
    } else if let Some(at) = time::utc_instant_from_local(value) {
        PublicationIntent::PublishAt(at)
    } else {
        PublicationIntent::PublishNow
    }
}

/// Every signal the composer edits.
///
/// Each field is a `Copy` handle into the reactive runtime (`Field` implements
/// `Copy` by hand for every `T`), so the whole struct is `Copy` and can be handed to
/// each shape and each event closure without per-signal capture.
#[derive(Clone, Copy)]
pub struct ComposeState {
    /// The post body: a parent-owned validated field, so a body that is not a
    /// `PostBody` disables submit and shows the newtype's own message rather than
    /// silently dropping the dispatch (#860, ADR-0105).
    pub body: Field<PostBody>,
    pub format: RwSignal<PostFormat>,
    /// Optional summary: a parent-owned validated field (ADR-0065 direct-bind), so an
    /// invalid excerpt disables submit and shows an error rather than erroring on POST.
    pub summary_field: Field<PostSummary>,
    /// Optional scheduled-publish time (naive local wall-clock from a
    /// `datetime-local` control); empty = publish now / draft. Only the full shape
    /// renders the control; the compact composer leaves it empty (publish-now).
    pub publish_at: RwSignal<String>,
    pub tags: RwSignal<Vec<TagSummary>>,
    /// Whether the author explicitly supplied the current tag collection. New
    /// and freshly-seeded editors leave this false so Org header metadata can
    /// fill the absence; changing tags makes even an empty collection explicit.
    tags_supplied: RwSignal<bool>,
    pub audience: RwSignal<AudienceSelection>,
}

impl ComposeState {
    /// A composer at its initial state.
    ///
    /// `audience` starts at Public as a placeholder: the site-wide default resolves
    /// asynchronously and the composer must render immediately (no `Suspense`), so
    /// the caller seeds this signal over the placeholder once its resource settles.
    #[must_use]
    pub fn new() -> Self {
        Self {
            body: Field::<PostBody>::new(),
            format: RwSignal::new(PostFormat::Markdown),
            summary_field: Field::<PostSummary>::optional(),
            publish_at: RwSignal::new(String::new()),
            tags: RwSignal::new(Vec::new()),
            tags_supplied: RwSignal::new(false),
            audience: RwSignal::new(AudienceSelection {
                base: AudienceBase::Public,
                named: Vec::new(),
            }),
        }
    }

    /// The create/update payload for this composer's current contents.
    ///
    /// `publication` maps explicitly to the server wire pair; `slug_override`
    /// is `None` for the compact shape, which renders no slug field.
    ///
    /// **Infallible by construction**: the caller already holds a parsed
    /// [`PostBody`], so there is no rejection left to represent and no error arm
    /// to swallow. A blank body is unrepresentable (#811, ADR-0105), and the only
    /// way to obtain the `PostBody` this takes is through [`submit_gate`] — the
    /// same call that decides whether the control is enabled. See
    /// `docs/adr/0113-submit-gate-owns-its-parse.md`.
    #[must_use]
    pub fn inputs(
        &self,
        body: PostBody,
        publication: PublicationIntent,
        slug_override: Option<Slug>,
    ) -> PostInputs {
        let (publish, publish_at) = match publication {
            PublicationIntent::Draft => (Some(false), None),
            PublicationIntent::PublishNow => (Some(true), None),
            PublicationIntent::PublishAt(at) => (Some(true), Some(at)),
        };
        PostInputs {
            body,
            format: self.format.get(),
            slug_override,
            publish,
            publish_at,
            tags: self
                .tags_supplied
                .get()
                .then(|| self.tags.get().into_iter().map(|t| t.display).collect()),
            summary: self.summary_field.parsed(),
            audience: Some(self.audience.get()),
        }
    }

    /// Mark the tag collection as explicitly supplied after a tag-input mutation.
    ///
    /// The tag widget owns its interaction paths, so callers hand this callback to
    /// it rather than duplicating presence bookkeeping in each event handler.
    #[must_use]
    pub fn tag_input_changed(&self) -> Callback<()> {
        let tags_supplied = self.tags_supplied;
        Callback::new(move |()| tags_supplied.set(true))
    }

    /// Load an existing post's contents into the fields this bundle owns.
    ///
    /// The editor reuses the bundle because it edits the same things and dispatches
    /// the same [`PostInputs`] payload; only the surrounding action (`Update` vs
    /// `Create`) differs. The slug is deliberately **not** seeded here: this type
    /// does not hold that field, because the compact composer uses this same bundle
    /// and renders no slug control — which is why `inputs` takes `slug_override` as a
    /// parameter. The two full-page shapes own the field at page level and hand it to
    /// `ComposeOptions`, so the editor sets it at the call site rather than handing
    /// the field in here to be written once.
    pub fn seed_from(&self, fetched: &AuthoredPost) {
        // `set_input`, not a bare `value.set`: a validated field's error must be written
        // with its value or the two disagree about the same field (#860, #907).
        self.body.set_input(&String::from(fetched.body.clone()));
        self.format.set(fetched.format);
        self.summary_field
            .set_input(fetched.post.summary.as_deref().unwrap_or_default());
        self.tags.set(fetched.post.tags.clone());
        self.tags_supplied.set(false);
    }

    /// Empty the composer for the next post, after a successful create.
    ///
    /// Deliberately leaves `format` and `audience` alone: an author writing a run of
    /// posts keeps their chosen format and audience, which is what the pre-existing
    /// reset did by only clearing these four.
    pub fn reset(&self) {
        self.body.reset();
        self.summary_field.reset();
        self.publish_at.set(String::new());
        self.tags.set(Vec::new());
        self.tags_supplied.set(false);
    }
}

impl Default for ComposeState {
    fn default() -> Self {
        Self::new()
    }
}

/// Pair a submit control's disabled state with the payload it dispatches, so the two
/// **cannot** disagree.
///
/// Returns `(disabled, on_click)` for the button markup to bind: `disabled` gates the
/// control, and `on_click` takes the `publish` flag and runs `on_submit` with an
/// already-parsed `body`. A dispatch closure built this way has no rejection to handle,
/// so it cannot silently drop a click — which is exactly what #860 reported, on the two
/// forms whose hand-written predicate had lost its body clause.
///
/// Both outputs read `body.parsed()`. That is deliberate and load-bearing: it is one
/// call, so "the control is disabled" and "there is no payload" are the same condition.
/// [`Field::is_valid`] is **not** used — it reads a cached `error` signal that a
/// programmatic write can leave stale while `parsed()` re-reads `value`, which would
/// reintroduce the two-source drift under a tidier name (#907).
///
/// `also_blocked` carries the caller's other reasons to disable (an invalid slug or
/// summary). It is a plain predicate, not another field, because each form blocks on a
/// different set.
///
/// Lives here rather than in the `component` module because that module is
/// `#[cfg(target_arch = "wasm32")]` (ADR-0070): a gate placed there would be neither
/// host-testable nor coverage-measured. See
/// `docs/adr/0113-submit-gate-owns-its-parse.md`.
#[must_use]
pub fn submit_gate(
    body: Field<PostBody>,
    also_blocked: Signal<bool>,
    on_submit: Callback<(PostBody, bool)>,
) -> (Signal<bool>, Callback<bool>) {
    let disabled = Signal::derive(move || also_blocked.get() || body.parsed().is_none());
    // A click handler is a `Fn`, so it must be total — some arm has to cover "no value".
    // This is the *only* place that arm is allowed to exist (ADR clause 3): here it is
    // co-conditioned with `disabled` above, so reaching it means the control was
    // disabled, and `a_blocked_gate_dispatches_nothing` pins exactly that. A second such
    // arm in a form is the defect this helper exists to prevent.
    let on_click = Callback::new(move |publish: bool| {
        if let Some(body) = body.parsed() {
            on_submit.run((body, publish));
        }
    });
    (disabled, on_click)
}

#[cfg(test)]
mod tests {
    use super::{ComposeState, PublicationIntent, publication_from_local, submit_gate};
    use crate::forms::Field;
    use common::post_body::PostBody;
    use common::render::PostFormat;
    use common::time::UtcInstant;
    use common::visibility::AudienceBase;
    use leptos::prelude::*;

    /// Run `body` under a fresh reactive `Owner`, so `RwSignal`s work host-side
    /// without a browser (the `web::reactive` / `forms::Field` convention).
    fn with_owner(body: impl FnOnce()) {
        let owner = Owner::new();
        owner.set();
        body();
        drop(owner);
    }

    #[test]
    fn inputs_map_every_publication_intent_to_the_wire_contract() {
        with_owner(|| {
            let state = ComposeState::new();
            let body: PostBody = "hello".parse().expect("a non-blank body parses");
            let scheduled_at: UtcInstant = "2999-02-03T10:15:00Z".parse().unwrap();

            let draft = state.inputs(body.clone(), PublicationIntent::Draft, None);
            assert_eq!(draft.body.as_ref(), "hello");
            assert_eq!(draft.publish, Some(false));
            assert_eq!(draft.publish_at, None);
            assert_eq!(draft.format, PostFormat::Markdown);
            assert!(draft.slug_override.is_none());
            assert_eq!(
                draft.tags, None,
                "an untouched new composer leaves Org header tags absent"
            );

            let now = state.inputs(body.clone(), PublicationIntent::PublishNow, None);
            assert_eq!(now.publish, Some(true));
            assert_eq!(now.publish_at, None);

            let scheduled = state.inputs(body, PublicationIntent::PublishAt(scheduled_at), None);
            assert_eq!(scheduled.publish, Some(true));
            assert_eq!(scheduled.publish_at, Some(scheduled_at));
        });
    }

    /// The body is a validated field, so seeding must leave it consistent: a real
    /// post's body is valid, and the seeded field must say so rather than keeping the
    /// "blank" error `Field::new` seeded at construction. The summary goes through the
    /// same door, for the same reason (#860).
    #[test]
    fn seed_from_leaves_the_seeded_fields_consistent() {
        with_owner(|| {
            let state = ComposeState::new();
            assert!(!state.body.is_valid(), "a pristine composer is invalid");

            state.seed_from(&crate::posts::render::test_fixtures::sample_post());

            assert_eq!(state.body.value.get(), "raw");
            assert!(state.body.is_valid(), "a seeded body is valid");
            assert!(!state.body.is_touched(), "seeding is not interaction");
            assert!(
                state.summary_field.is_valid(),
                "an absent summary seeds an empty, valid optional field"
            );
        });
    }

    /// After a successful create the composer returns to pristine: empty, untouched and
    /// invalid again — which is what re-disables the submit buttons (#860).
    #[test]
    fn reset_returns_the_body_field_to_pristine() {
        with_owner(|| {
            let state = ComposeState::new();
            state.body.set_input("some text");
            state.body.touch();

            state.reset();

            assert_eq!(state.body.value.get(), "");
            assert!(!state.body.is_touched());
            assert!(!state.body.is_valid(), "an empty body is not a PostBody");
        });
    }

    /// The gate blocks when the body does not parse — whatever the other predicate says.
    #[test]
    fn the_gate_blocks_an_unparseable_body() {
        with_owner(|| {
            let body = Field::<PostBody>::new();
            let (disabled, _) = submit_gate(body, Signal::derive(|| false), Callback::new(|_| {}));

            assert!(disabled.get(), "an empty body blocks");

            body.set_input("   \n\t ");
            assert!(disabled.get(), "a whitespace-only body blocks");

            body.set_input("real text");
            assert!(!disabled.get(), "a parsing body with nothing else blocking");
        });
    }

    /// The gate also blocks on the caller's predicate (an invalid slug or summary),
    /// independently of the body.
    #[test]
    fn the_gate_blocks_on_the_callers_predicate() {
        with_owner(|| {
            let body = Field::<PostBody>::new();
            body.set_input("real text");
            let blocked = RwSignal::new(true);
            let (disabled, _) = submit_gate(
                body,
                Signal::derive(move || blocked.get()),
                Callback::new(|_| {}),
            );

            assert!(disabled.get(), "blocked by the caller despite a valid body");
            blocked.set(false);
            assert!(
                !disabled.get(),
                "unblocked once the caller's predicate clears"
            );
        });
    }

    /// The click hands through the *parsed* body — the dispatch closure never parses.
    #[test]
    fn the_click_hands_through_a_parsed_body() {
        with_owner(|| {
            let body = Field::<PostBody>::new();
            body.set_input("real text");
            let seen: RwSignal<Vec<(String, bool)>> = RwSignal::new(Vec::new());
            let (_, on_click) = submit_gate(
                body,
                Signal::derive(|| false),
                Callback::new(move |(b, publish): (PostBody, bool)| {
                    seen.update(|v| v.push((b.as_ref().to_owned(), publish)));
                }),
            );

            on_click.run(false);
            on_click.run(true);

            assert_eq!(
                seen.get(),
                vec![
                    ("real text".to_owned(), false),
                    ("real text".to_owned(), true),
                ],
                "each click runs on_submit once with the parsed body and its flag"
            );
        });
    }

    /// A click that should be impossible runs nothing — and, crucially, the two
    /// conditions are the same one: disabled iff there is no payload.
    #[test]
    fn a_blocked_gate_dispatches_nothing() {
        with_owner(|| {
            let body = Field::<PostBody>::new();
            let ran = RwSignal::new(0_u32);
            let (disabled, on_click) = submit_gate(
                body,
                Signal::derive(|| false),
                Callback::new(move |_: (PostBody, bool)| ran.update(|n| *n += 1)),
            );

            on_click.run(true);
            assert_eq!(ran.get(), 0, "an unparseable body dispatches nothing");
            assert!(disabled.get(), "and the control reporting that is disabled");

            body.set_input("real text");
            on_click.run(true);
            assert_eq!(ran.get(), 1);
            assert!(!disabled.get());
        });
    }

    #[test]
    fn publication_from_local_preserves_create_form_behavior() {
        assert_eq!(
            publication_from_local(false, "2999-02-03T10:15"),
            PublicationIntent::Draft,
            "draft wins even when the schedule field has a value",
        );
        assert_eq!(
            publication_from_local(true, ""),
            PublicationIntent::PublishNow,
        );
        assert_eq!(
            publication_from_local(true, "not-a-date"),
            PublicationIntent::PublishNow,
            "invalid input retains the existing publish-now fallback",
        );
        assert!(matches!(
            publication_from_local(true, "2999-02-03T10:15"),
            PublicationIntent::PublishAt(_),
        ));
    }

    /// The editor's entry point: an existing post's fields land in the bundle. Uses
    /// the render layer's own sample so the fixture cannot drift from the one the
    /// projector tests paint.
    #[test]
    fn seed_from_loads_an_existing_post_into_the_editor_fields() {
        with_owner(|| {
            let state = ComposeState::new();
            let fetched = crate::posts::render::test_fixtures::sample_post();

            state.seed_from(&fetched);

            assert_eq!(state.body.value.get(), "raw");
            assert_eq!(state.format.get(), PostFormat::Markdown);
            assert_eq!(state.tags.get().len(), 1);
            let inputs = state.inputs(
                "edited body".parse().expect("a non-blank body parses"),
                PublicationIntent::PublishNow,
                None,
            );
            assert_eq!(
                inputs.tags, None,
                "loaded tags remain implicit until the author changes them"
            );
            assert_eq!(
                state.summary_field.value.get(),
                "",
                "a post with no summary seeds an empty field, not the string \"None\""
            );
        });
    }

    #[test]
    fn tag_interactions_make_even_an_empty_collection_explicit() {
        with_owner(|| {
            let state = ComposeState::new();
            let input =
                crate::tags::InputState::new(state.tags).with_on_change(state.tag_input_changed());
            let tag = crate::posts::render::test_fixtures::sample_post()
                .post
                .tags
                .into_iter()
                .next()
                .expect("the sample post has a tag");
            let body: PostBody = "body".parse().expect("a non-blank body parses");

            input.commit(tag.clone());
            assert_eq!(
                state
                    .inputs(body.clone(), PublicationIntent::PublishNow, None)
                    .tags,
                Some(vec![tag.display.clone()]),
                "adding a tag supplies the structured collection"
            );

            input.remove(&tag);
            assert_eq!(
                state.inputs(body, PublicationIntent::PublishNow, None).tags,
                Some(Vec::new()),
                "clearing tags after interaction remains an explicit empty collection"
            );
        });
    }

    #[test]
    fn reset_clears_the_post_body_but_keeps_format_and_audience() {
        with_owner(|| {
            // `default()` here rather than `new()` so the `Default` impl is exercised
            // too; it delegates, so this covers both (the `web::reactive` precedent).
            let state = ComposeState::default();
            state.body.set_input("draft text");
            state.publish_at.set("2026-01-01T00:00".to_string());
            state.format.set(PostFormat::Org);

            state.reset();

            assert_eq!(state.body.value.get(), "");
            assert_eq!(state.publish_at.get(), "");
            assert!(state.tags.get().is_empty());
            assert_eq!(
                state.format.get(),
                PostFormat::Org,
                "an author writing a run of posts keeps their format"
            );
            assert_eq!(state.audience.get().base, AudienceBase::Public);
        });
    }
}
