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
use common::post_title::PostTitle;
use common::render::PostFormat;
use common::seed::{AuthoredPost, TagSummary};
use common::slug::Slug;
use common::time::{self, UtcInstant};
use common::visibility::AudienceSelection;

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
    /// Only an existing editor with a seeded Org title has a synthetic header
    /// to remove when switching formats; new-composer source belongs to its author.
    seeded_org_title: RwSignal<bool>,
}

/// Comparable values that determine whether a creation composer has unsaved input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationComposerSnapshot {
    body: String,
    format: PostFormat,
    summary: String,
    publish_at: String,
    tags: Vec<TagSummary>,
    audience: AudienceSelection,
    slug: String,
    schedule_date: String,
    schedule_time: String,
}

impl CreationComposerSnapshot {
    /// Capture every creation input, including fields owned outside [`ComposeState`].
    #[must_use]
    pub fn capture(
        state: ComposeState,
        slug_field: Field<Slug>,
        schedule_date: String,
        schedule_time: String,
    ) -> Self {
        Self {
            body: state.body.value(),
            format: state.format.get(),
            summary: state.summary_field.value(),
            publish_at: state.publish_at.get(),
            tags: state.tags.get(),
            audience: state.audience.get(),
            slug: slug_field.value(),
            schedule_date,
            schedule_time,
        }
    }

    /// Adopt an asynchronously resolved initial audience without accepting other edits.
    #[must_use]
    pub fn with_audience(mut self, audience: AudienceSelection) -> Self {
        self.audience = audience;
        self
    }
}

/// The editor cannot safely submit an Org Post whose title cannot round-trip
/// through nonblank `#+TITLE:` directives.
#[derive(Debug, PartialEq, Eq)]
pub enum OrgEditorSeedError {
    UnrepresentableTitle,
}

fn org_editor_title(title: &PostTitle) -> Result<String, OrgEditorSeedError> {
    let lines: Vec<_> = title.as_ref().split('\n').collect();
    if lines
        .iter()
        .any(|line| line.is_empty() || line.trim() != *line || line.contains('\r'))
    {
        return Err(OrgEditorSeedError::UnrepresentableTitle);
    }
    Ok(format!("#+TITLE: {}\n", lines.join("\n#+TITLE: ")))
}

fn is_title_line(line: &str) -> bool {
    line.trim_start()
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("#+TITLE:"))
}

/// Find the editor's first title block, including its blank separator. Match
/// whole directive lines rather than substrings in authored prose; the current
/// directives may have been edited since seeding, so old bytes are not an anchor.
fn editor_title_range(source: &str) -> Option<std::ops::Range<usize>> {
    let mut start = 0;
    for line in source.split_inclusive('\n') {
        if is_title_line(line) {
            break;
        }
        start += line.len();
    }
    if start == source.len() {
        return None;
    }
    let mut header_end = start;
    for line in source[start..].split_inclusive('\n') {
        if !is_title_line(line) {
            break;
        }
        header_end += line.len();
    }
    let end = header_end + usize::from(source[header_end..].starts_with('\n'));
    Some(start..end)
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
                public: true,
                ..AudienceSelection::default()
            }),
            seeded_org_title: RwSignal::new(false),
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
    ///
    /// # Errors
    ///
    /// Refuses an Org title that cannot round-trip through nonblank title lines.
    pub fn seed_from(&self, fetched: &AuthoredPost) -> Result<(), OrgEditorSeedError> {
        // The header exists only in the editable projection; persistence still owns
        // the canonical metadata-free body (ADR-0024/0155).
        let title = if fetched.format == PostFormat::Org {
            fetched.title.as_ref().map(org_editor_title).transpose()?
        } else {
            None
        };
        let body = title.as_ref().map_or_else(
            || fetched.body.to_string(),
            |title| format!("{title}\n{}", fetched.body),
        );
        // Set through the validated field API so value and validity stay consistent (#860, #907).
        self.body.set_value(&body);
        self.format.set(fetched.format);
        self.seeded_org_title.set(title.is_some());
        self.summary_field
            .set_value(fetched.post.summary.as_deref().unwrap_or_default());
        self.tags.set(fetched.post.tags.clone());
        self.tags_supplied.set(false);
        Ok(())
    }

    /// Change format without carrying an editor-projected Org title into other source formats.
    pub fn switch_format(&self, format: PostFormat) {
        if self.format.get() == PostFormat::Org
            && format != PostFormat::Org
            && self.seeded_org_title.get()
        {
            let source = self.body.value();
            if let Some(range) = editor_title_range(&source) {
                let mut body = source;
                body.replace_range(range, "");
                self.body.set_value(&body);
            }
            self.seeded_org_title.set(false);
        }
        self.format.set(format);
    }

    /// Empty the composer for the next post, after a successful create.
    ///
    /// Deliberately leaves `format` and `audience` alone: an author writing a run of
    /// posts keeps their chosen format and audience; only their content inputs
    /// return to the pristine state.
    pub fn reset(&self) {
        self.body.reset();
        self.summary_field.reset();
        self.publish_at.set(String::new());
        self.tags.set(Vec::new());
        self.tags_supplied.set(false);
        self.seeded_org_title.set(false);
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
/// Both outputs deliberately call `body.parsed()`: the gate couples the
/// required body payload directly to its disabled state. [`Field::is_valid`]
/// is also consistent after #907 because it and `parsed()` derive from the same
/// private current input; callers use it in `also_blocked` for optional fields
/// whose valid absence is intentionally not a parsed payload.
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
        if !also_blocked.get()
            && let Some(body) = body.parsed()
        {
            on_submit.run((body, publish));
        }
    });
    (disabled, on_click)
}

#[cfg(test)]
mod tests {
    use super::{
        ComposeState, CreationComposerSnapshot, OrgEditorSeedError, PublicationIntent,
        publication_from_local, submit_gate,
    };
    use crate::forms::Field;
    use common::post_body::PostBody;
    use common::render::PostFormat;
    use common::slug::Slug;
    use common::time::UtcInstant;
    use common::visibility::AudienceSelection;
    use leptos::prelude::*;

    #[test]
    fn creation_snapshot_detects_owned_and_provisional_input_changes() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            let slug = Field::<Slug>::optional();
            let initial =
                CreationComposerSnapshot::capture(state, slug, String::new(), String::new());

            state.body.set_value("draft");
            assert_ne!(
                CreationComposerSnapshot::capture(state, slug, String::new(), String::new()),
                initial
            );
            state.body.reset();
            slug.set_value("chosen-slug");
            assert_ne!(
                CreationComposerSnapshot::capture(state, slug, String::new(), String::new()),
                initial
            );
            slug.reset();
            assert_ne!(
                CreationComposerSnapshot::capture(
                    state,
                    slug,
                    "2999-01-02".to_owned(),
                    "12:30".to_owned(),
                ),
                initial
            );
        });
    }

    #[test]
    fn creation_snapshot_adopts_only_the_resolved_initial_audience() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            let slug = Field::<Slug>::optional();
            let initial =
                CreationComposerSnapshot::capture(state, slug, String::new(), String::new());
            state.body.set_value("draft");
            let resolved = AudienceSelection::default();

            let baseline = initial.with_audience(resolved.clone());
            state.audience.set(resolved);
            assert_ne!(
                CreationComposerSnapshot::capture(state, slug, String::new(), String::new()),
                baseline,
                "resolving the audience must not absorb an in-progress body edit"
            );
        });
    }

    #[test]
    fn inputs_map_every_publication_intent_to_the_wire_contract() {
        Owner::new().with(|| {
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
        Owner::new().with(|| {
            let state = ComposeState::new();
            assert!(!state.body.is_valid(), "a pristine composer is invalid");

            state
                .seed_from(&crate::posts::render::test_fixtures::sample_post())
                .unwrap();

            assert_eq!(state.body.value(), "raw");
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
        Owner::new().with(|| {
            let state = ComposeState::new();
            state.body.set_value("some text");
            state.body.touch();

            state.reset();

            assert_eq!(state.body.value(), "");
            assert!(!state.body.is_touched());
            assert!(!state.body.is_valid(), "an empty body is not a PostBody");
        });
    }

    /// The gate blocks when the body does not parse — whatever the other predicate says.
    #[test]
    fn the_gate_blocks_an_unparseable_body() {
        Owner::new().with(|| {
            let body = Field::<PostBody>::new();
            let (disabled, _) = submit_gate(body, Signal::derive(|| false), Callback::new(|_| {}));

            assert!(disabled.get(), "an empty body blocks");

            body.set_value("   \n\t ");
            assert!(disabled.get(), "a whitespace-only body blocks");

            body.set_value("real text");
            assert!(!disabled.get(), "a parsing body with nothing else blocking");
        });
    }

    /// The gate also blocks on the caller's predicate (an invalid slug or summary),
    /// independently of the body.
    #[test]
    fn the_gate_blocks_on_the_callers_predicate() {
        Owner::new().with(|| {
            let body = Field::<PostBody>::new();
            body.set_value("real text");
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
        Owner::new().with(|| {
            let body = Field::<PostBody>::new();
            body.set_value("real text");
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

    /// A direct callback invocation obeys every disabled predicate, not only body parsing.
    #[test]
    fn a_blocked_gate_dispatches_nothing() {
        Owner::new().with(|| {
            let body = Field::<PostBody>::new();
            body.set_value("real text");
            let blocked = RwSignal::new(true);
            let ran = RwSignal::new(0_u32);
            let (disabled, on_click) = submit_gate(
                body,
                Signal::derive(move || blocked.get()),
                Callback::new(move |_: (PostBody, bool)| ran.update(|n| *n += 1)),
            );

            on_click.run(true);
            assert_eq!(ran.get(), 0, "a caller-blocked control dispatches nothing");
            assert!(disabled.get(), "and the control reporting that is disabled");

            blocked.set(false);
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
        Owner::new().with(|| {
            let state = ComposeState::new();
            let fetched = crate::posts::render::test_fixtures::sample_post();

            state.seed_from(&fetched).unwrap();

            assert_eq!(state.body.value(), "raw");
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
                state.summary_field.value(),
                "",
                "a post with no summary seeds an empty field, not the string \"None\""
            );
        });
    }

    #[test]
    fn org_editor_seeds_editable_title_lines_without_changing_canonical_body() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            let mut fetched = crate::posts::render::test_fixtures::sample_post();
            fetched.format = PostFormat::Org;
            fetched.title = Some("First line\nSecond line".parse().unwrap());
            fetched.body = "#+AUTHOR: Kept\n\nOrg content".parse().unwrap();

            state.seed_from(&fetched).unwrap();

            assert_eq!(
                state.body.value(),
                "#+TITLE: First line\n#+TITLE: Second line\n\n#+AUTHOR: Kept\n\nOrg content"
            );
            assert_eq!(fetched.body.as_ref(), "#+AUTHOR: Kept\n\nOrg content");
            assert_eq!(state.format.get(), PostFormat::Org);
        });
    }

    #[test]
    fn org_editor_keeps_titleless_source_and_refuses_unrepresentable_title() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            let mut fetched = crate::posts::render::test_fixtures::sample_post();
            fetched.format = PostFormat::Org;
            fetched.title = None;
            fetched.body = "Org content".parse().unwrap();
            state.seed_from(&fetched).unwrap();
            assert_eq!(state.body.value(), "Org content");

            fetched.title = Some("Line one\n\nLine three".parse().unwrap());
            assert_eq!(
                state.seed_from(&fetched),
                Err(OrgEditorSeedError::UnrepresentableTitle)
            );
            assert_eq!(state.body.value(), "Org content");
        });
    }

    #[test]
    fn switching_editor_from_org_removes_synthetic_title_but_keeps_body() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            let mut fetched = crate::posts::render::test_fixtures::sample_post();
            fetched.format = PostFormat::Org;
            fetched.title = Some("Title".parse().unwrap());
            fetched.body = "Org content".parse().unwrap();
            state.seed_from(&fetched).unwrap();
            assert_eq!(state.body.value(), "#+TITLE: Title\n\nOrg content");

            state.switch_format(PostFormat::Markdown);

            assert_eq!(state.body.value(), "Org content");
            assert_eq!(state.format.get(), PostFormat::Markdown);
        });
    }

    #[test]
    fn switching_formats_drops_a_seeded_title_after_prepended_content() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            let mut fetched = crate::posts::render::test_fixtures::sample_post();
            fetched.format = PostFormat::Org;
            fetched.title = Some("Title".parse().unwrap());
            fetched.body = "Org content".parse().unwrap();
            state.seed_from(&fetched).unwrap();
            state
                .body
                .set_value("Example: #+TITLE: Title\n#+TITLE: Title\n\nOrg content");

            state.switch_format(PostFormat::Markdown);

            assert_eq!(state.body.value(), "Example: #+TITLE: Title\nOrg content");
        });
    }

    #[test]
    fn switching_formats_also_removes_a_title_whose_blank_separator_was_deleted() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            let mut fetched = crate::posts::render::test_fixtures::sample_post();
            fetched.format = PostFormat::Org;
            fetched.title = Some("Title".parse().unwrap());
            fetched.body = "Org content".parse().unwrap();
            state.seed_from(&fetched).unwrap();
            state.body.set_value("#+TITLE: Title\nOrg content");

            state.switch_format(PostFormat::Markdown);

            assert_eq!(state.body.value(), "Org content");
        });
    }

    #[test]
    fn switching_formats_removes_edited_single_and_multiline_title_headers() {
        Owner::new().with(|| {
            for (old_title, edited_source) in [
                ("Old", "#+TITLE: New\n\nOrg content"),
                (
                    "Old first\nOld second",
                    "#+TITLE: New first\n#+TITLE: New second\n\nOrg content",
                ),
            ] {
                let state = ComposeState::new();
                let mut fetched = crate::posts::render::test_fixtures::sample_post();
                fetched.format = PostFormat::Org;
                fetched.title = Some(old_title.parse().unwrap());
                fetched.body = "Org content".parse().unwrap();
                state.seed_from(&fetched).unwrap();
                state.body.set_value(edited_source);

                state.switch_format(PostFormat::Markdown);

                assert_eq!(state.body.value(), "Org content");
            }
        });
    }

    #[test]
    fn switching_formats_keeps_an_authored_title_line_matching_the_old_title() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            let mut fetched = crate::posts::render::test_fixtures::sample_post();
            fetched.format = PostFormat::Org;
            fetched.title = Some("Old".parse().unwrap());
            fetched.body = "Intro\n#+TITLE: Old\n\nTail".parse().unwrap();
            state.seed_from(&fetched).unwrap();
            state
                .body
                .set_value("#+TITLE: New\n\nIntro\n#+TITLE: Old\n\nTail");

            state.switch_format(PostFormat::Markdown);

            assert_eq!(state.body.value(), "Intro\n#+TITLE: Old\n\nTail");
        });
    }

    #[test]
    fn switching_without_a_projected_header_preserves_author_source() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            state.format.set(PostFormat::Org);
            state.body.set_value("#+TITLE: Authored\n\nOrg content");

            state.switch_format(PostFormat::Markdown);

            assert_eq!(state.body.value(), "#+TITLE: Authored\n\nOrg content");
        });
    }

    #[test]
    fn switching_after_deleting_the_title_keeps_the_remaining_body() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            let mut fetched = crate::posts::render::test_fixtures::sample_post();
            fetched.format = PostFormat::Org;
            fetched.title = Some("Title".parse().unwrap());
            fetched.body = "Org content".parse().unwrap();
            state.seed_from(&fetched).unwrap();
            state.body.set_value("Org content");

            state.switch_format(PostFormat::Markdown);

            assert_eq!(state.body.value(), "Org content");
        });
    }

    #[test]
    fn tag_interactions_make_even_an_empty_collection_explicit() {
        Owner::new().with(|| {
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
        Owner::new().with(|| {
            // `default()` here rather than `new()` so the `Default` impl is exercised
            // too; it delegates, so this covers both (the `web::reactive` precedent).
            let state = ComposeState::default();
            state.body.set_value("draft text");
            state.publish_at.set("2026-01-01T00:00".to_string());
            state.format.set(PostFormat::Org);

            state.reset();

            assert_eq!(state.body.value(), "");
            assert_eq!(state.publish_at.get(), "");
            assert!(state.tags.get().is_empty());
            assert_eq!(
                state.format.get(),
                PostFormat::Org,
                "an author writing a run of posts keeps their format"
            );
            assert!(state.audience.get().public);
        });
    }
}
