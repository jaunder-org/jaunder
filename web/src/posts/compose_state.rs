//! The new-post composer's shared reactive state, extracted from the wasm-only
//! `component` so the dispatch payload it builds is **host-tested** rather than left
//! to e2e alone (ADR-0070 §6 — the convention `tags::input_state`,
//! `media::upload_state` and `forms::Field` follow).
//!
//! The composer renders in two shapes (a compact inline row and the full compose
//! page) over one set of signals. Bundling them here is what lets each shape be its
//! own `#[component]` taking a single prop instead of seven, and it removes the
//! three near-identical `PostInputs` constructions the two shapes used to carry.

use leptos::prelude::*;

use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::render::PostFormat;
use common::seed::{AuthoredPost, TagSummary};
use common::slug::Slug;
use common::time::utc_instant_from_local;
use common::visibility::{AudienceBase, AudienceSelection};

use crate::forms::Field;
use crate::posts::PostInputs;

/// Every signal the composer edits.
///
/// Each field is a `Copy` handle into the reactive runtime (`Field` implements
/// `Copy` by hand for every `T`), so the whole struct is `Copy` and can be handed to
/// each shape and each event closure without per-signal capture.
#[derive(Clone, Copy)]
pub struct ComposeState {
    pub body: RwSignal<String>,
    pub format: RwSignal<PostFormat>,
    /// Optional summary: a parent-owned validated field (ADR-0065 direct-bind), so an
    /// invalid excerpt disables submit and shows an error rather than erroring on POST.
    pub summary_field: Field<PostSummary>,
    /// Optional scheduled-publish time (naive local wall-clock from a
    /// `datetime-local` control); empty = publish now / draft. Only the full shape
    /// renders the control; the compact composer leaves it empty (publish-now).
    pub publish_at: RwSignal<String>,
    pub tags: RwSignal<Vec<TagSummary>>,
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
            body: RwSignal::new(String::new()),
            format: RwSignal::new(PostFormat::Markdown),
            summary_field: Field::<PostSummary>::optional(),
            publish_at: RwSignal::new(String::new()),
            tags: RwSignal::new(Vec::new()),
            audience: RwSignal::new(AudienceSelection {
                base: AudienceBase::Public,
                named: Vec::new(),
            }),
        }
    }

    /// The create/update payload for this composer's current contents, or `None`
    /// when the body is not a valid [`PostBody`].
    ///
    /// `publish` distinguishes "Save draft" from "Publish"; `slug_override` is
    /// `None` for the compact shape, which renders no slug field.
    ///
    /// The body is **parsed, not coerced** (#811, ADR-0102): a blank `PostBody` is
    /// unrepresentable, and the submit buttons' disabled state is a UI affordance
    /// rather than a guarantee — a dispatch can still be driven by other means. The
    /// caller drops a `None` on the floor, exactly as the pre-decomposition
    /// `let Ok(body) = … else { return; }` guard did; returning the `Option` is what
    /// keeps that decision host-testable rather than buried in an event closure.
    #[must_use]
    pub fn inputs(&self, publish: bool, slug_override: Option<Slug>) -> Option<PostInputs> {
        let body = self.body.get().parse::<PostBody>().ok()?;
        Some(PostInputs {
            body,
            format: self.format.get(),
            slug_override,
            publish,
            publish_at: utc_instant_from_local(&self.publish_at.get()),
            tags: Some(self.tags.get().into_iter().map(|t| t.display).collect()),
            summary: self.summary_field.parsed(),
            audience: Some(self.audience.get()),
        })
    }

    /// Load an existing post's contents into the fields this bundle owns.
    ///
    /// The editor reuses the bundle because it edits the same things and dispatches
    /// the same [`PostInputs`] payload; only the surrounding action (`Update` vs
    /// `Create`) differs. The slug is deliberately **not** seeded here: this type
    /// does not hold that field — the composer's full shape owns its own, local to
    /// that shape — so the editor sets it at the call site rather than handing the
    /// field in to be written once.
    pub fn seed_from(&self, fetched: &AuthoredPost) {
        self.body.set(String::from(fetched.body.clone()));
        self.format.set(fetched.format);
        self.summary_field.value.set(
            fetched
                .post
                .summary
                .as_deref()
                .unwrap_or_default()
                .to_owned(),
        );
        self.tags.set(fetched.post.tags.clone());
    }

    /// Empty the composer for the next post, after a successful create.
    ///
    /// Deliberately leaves `format` and `audience` alone: an author writing a run of
    /// posts keeps their chosen format and audience, which is what the pre-existing
    /// reset did by only clearing these four.
    pub fn reset(&self) {
        self.body.set(String::new());
        self.summary_field.reset();
        self.publish_at.set(String::new());
        self.tags.set(Vec::new());
    }
}

impl Default for ComposeState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::ComposeState;
    use common::render::PostFormat;
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
    fn inputs_carry_the_edited_body_and_the_publish_flag() {
        with_owner(|| {
            let state = ComposeState::new();
            state.body.set("hello".to_string());

            let draft = state.inputs(false, None).expect("a non-blank body parses");
            assert_eq!(draft.body.as_ref(), "hello");
            assert!(!draft.publish);
            assert_eq!(draft.format, PostFormat::Markdown);
            assert!(draft.slug_override.is_none());

            assert!(
                state.inputs(true, None).expect("still parses").publish,
                "publish flag passes through"
            );
        });
    }

    /// The #811 / ADR-0102 invariant, enforced where it is testable: a blank body is
    /// not a `PostBody`, so there is no payload to dispatch. The buttons' disabled
    /// state is an affordance, not the guarantee.
    #[test]
    fn a_blank_body_yields_no_payload() {
        with_owner(|| {
            let state = ComposeState::new();
            assert!(state.inputs(true, None).is_none(), "empty body");

            state.body.set("   \n\t ".to_string());
            assert!(state.inputs(true, None).is_none(), "whitespace-only body");
        });
    }

    /// Empty means "publish now": the naive-local conversion must yield `None`
    /// rather than an epoch instant, or every unscheduled post would backdate.
    #[test]
    fn an_empty_publish_at_schedules_nothing() {
        with_owner(|| {
            let state = ComposeState::new();
            state.body.set("body".to_string());
            assert!(
                state
                    .inputs(true, None)
                    .expect("a non-blank body parses")
                    .publish_at
                    .is_none()
            );
        });
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

            assert_eq!(state.body.get(), "raw");
            assert_eq!(state.format.get(), PostFormat::Markdown);
            assert_eq!(state.tags.get().len(), 1);
            assert_eq!(
                state.summary_field.value.get(),
                "",
                "a post with no summary seeds an empty field, not the string \"None\""
            );
        });
    }

    #[test]
    fn reset_clears_the_post_body_but_keeps_format_and_audience() {
        with_owner(|| {
            // `default()` here rather than `new()` so the `Default` impl is exercised
            // too; it delegates, so this covers both (the `web::reactive` precedent).
            let state = ComposeState::default();
            state.body.set("draft text".to_string());
            state.publish_at.set("2026-01-01T00:00".to_string());
            state.format.set(PostFormat::Org);

            state.reset();

            assert_eq!(state.body.get(), "");
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
