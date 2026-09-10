use leptos::prelude::*;

use crate::auth;
use crate::avatar::Avatar;
use crate::error::WebError;
use crate::forms::{self, Field, ValidatedBareInput, ValidatedTextarea};
use crate::media::MediaUpload;
use crate::posts;
use crate::posts::{
    ComposeState, Create, InvalidSchedule, LoadedPublication, NamedAudienceState, SavedPost,
    ScheduledEditState,
};
use crate::tags::TagInput;
use crate::topbar::Topbar;
use common::MutationOutcome;
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::render::PostFormat;
use common::slug::Slug;
use common::time::{self, UtcInstant};
use common::username::Username;

use super::audience::AudiencePickerWithState;
use super::{audience, support};

/// The `.j-seg` Markdown/Org format toggle, shared by every post editor. Renders one
/// button per user-selectable `PostFormat` — those carrying a `strum` editor message;
/// `Html` has none (renderer-internal, #445), so it is filtered out. Adding a format is
/// a one-attribute change on `PostFormat`, not new markup here.
#[component]
fn FormatToggle(
    format: RwSignal<PostFormat>,
    /// Extra inline style for the `.j-seg` wrapper (e.g. spacing). Omitted when unset.
    #[prop(optional, into)]
    style: Option<&'static str>,
) -> impl IntoView {
    use strum::{EnumMessage, VariantArray};
    view! {
        <div class="j-seg" style=style>
            {PostFormat::VARIANTS
                .iter()
                .copied()
                .filter_map(|f| f.get_message().map(|label| (f, label)))
                .map(|(f, label)| {
                    view! {
                        <button
                            type="button"
                            class=move || {
                                if format.get() == f { "j-btn is-selected" } else { "j-btn" }
                            }
                            on:click=move |_| format.set(f)
                        >
                            {label}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

/// Shared body + format fields used by all post editors.
///
/// Renders a `name="body"` textarea through [`ValidatedTextarea`], so a body that is
/// not a `PostBody` shows the newtype's own message once touched — the same treatment
/// the summary and slug already get (#860). When `show_seg` is true (default), also
/// renders the `.j-seg` format toggle.
///
/// Neither `field_class` nor `textarea_class` has a default, on purpose: `ValidatedTextarea`
/// wraps the control in a `<label>`, which changes what the surrounding flex column lays
/// out, so each caller must name the classes that suit its own layout rather than
/// inheriting a default that only one of the three sites actually wanted.
#[component]
pub fn ComposerFields(
    body: Field<PostBody>,
    format: RwSignal<PostFormat>,
    field_class: &'static str,
    #[prop(default = "Write something\u{2026}")] placeholder: &'static str,
    #[prop(default = 16u32)] rows: u32,
    textarea_class: &'static str,
    /// When false, the `.j-seg` format toggle is not rendered (caller places it elsewhere).
    #[prop(default = true)]
    show_seg: bool,
    /// Optional callback fired on every body input event (e.g. to clear a flash message).
    /// `optional_no_strip`, so a caller holding its own `Option` forwards it as-is rather
    /// than unwrapping it into a do-nothing callback.
    #[prop(optional_no_strip)]
    on_input: Option<Callback<()>>,
) -> impl IntoView {
    view! {
        <ValidatedTextarea<PostBody>
            label="Body"
            name="body"
            field=body
            rows=rows
            placeholder=placeholder
            field_class=field_class
            class=textarea_class
            on_input=on_input
        />
        {show_seg.then(move || view! { <FormatToggle format=format /> })}
    }
}

/// Provisional publication-time inputs for the full new-Post composer.
///
/// The committed wire value remains on [`ComposeState`]; these fields exist only while
/// the author is choosing a replacement, so neither an incomplete selection nor a
/// cancelled change can leak into a create request.
#[derive(Clone, Copy)]
pub(super) struct CreationSchedule {
    disclosed: RwSignal<bool>,
    date: RwSignal<String>,
    time: RwSignal<String>,
    scheduled: RwSignal<bool>,
    error: RwSignal<Option<String>>,
}

impl CreationSchedule {
    fn new() -> Self {
        Self {
            disclosed: RwSignal::new(false),
            date: RwSignal::new(String::new()),
            time: RwSignal::new(String::new()),
            scheduled: RwSignal::new(false),
            error: RwSignal::new(None),
        }
    }

    fn restore_committed(self, committed: &str) {
        let (date, time) = committed.split_once('T').unwrap_or(("", ""));
        self.date.set(date.to_owned());
        self.time.set(time.to_owned());
        self.error.set(None);
    }

    fn has_unapplied_changes(self, committed: &str) -> bool {
        self.disclosed.get() && {
            let value = match (self.date.get().as_str(), self.time.get().as_str()) {
                ("", "") => String::new(),
                (date, time) => format!("{date}T{time}"),
            };
            value != committed
        }
    }
}

/// The new-post composer. Renders in one of two shapes over a single
/// [`ComposeState`]: a compact inline row, or the full compose page.
///
/// This component owns only what both shapes share — the state bundle, the
/// async default-audience seed, and the post-create hand-off — so the shape
/// choice is the one branch it carries.
#[component]
pub fn PostCreateForm(
    compact: bool,
    #[prop(optional)] username: Option<Username>,
    #[prop(into)] on_success: Callback<SavedPost>,
    #[prop(optional)] on_mutation: Option<Callback<bool>>,
    #[prop(default = 6)] rows: u32,
    #[prop(default = "What\u{2019}s on your mind?")] placeholder: &'static str,
    /// Called on every textarea input event (compact mode only).
    #[prop(optional)]
    on_input: Option<Callback<()>>,
) -> impl IntoView {
    let create_action = ServerAction::<Create>::new();
    let state = ComposeState::new();

    let default_audience = Resource::new(|| (), |()| posts::get_default_audience_selection());
    // The site-wide default audience resolves asynchronously; the composer must
    // render immediately (no Suspense), so seed the editable `audience` signal
    // once the Resource resolves, over the Public placeholder `ComposeState::new`
    // sets. The author can then edit the selection via `AudiencePicker`.
    support::on_settled_ok(
        move || default_audience.get(),
        move |default| state.audience.set(default),
    );

    // Revalidate parent state after either outcome, but reserve success UI and
    // form reset for a confirmed create.
    support::on_settled_ok(
        move || create_action.value().get(),
        move |outcome| {
            if posts::notify_create_settlement(outcome, on_mutation, on_success) {
                state.reset();
            }
        },
    );

    if compact {
        view! {
            <CompactComposer
                state=state
                create_action=create_action
                username=username
                rows=rows
                placeholder=placeholder
                on_input=on_input
            />
        }
        .into_any()
    } else {
        view! { <FullComposer state=state create_action=create_action rows=rows placeholder=placeholder /> }
        .into_any()
    }
}

/// The inline composer row: avatar, body, summary, tags, and the two dispatch
/// buttons. Split out of [`PostCreateForm`] (#301); no slug or schedule control,
/// so it dispatches with `slug_override: None` and an empty `publish_at`.
#[component]
fn CompactComposer(
    state: ComposeState,
    create_action: ServerAction<Create>,
    /// Passed through from `PostCreateForm`, which is the only caller — so these two
    /// are plain `Option` props rather than `#[prop(optional)]` ones, which would
    /// take the inner value and re-wrap it.
    username: Option<Username>,
    rows: u32,
    placeholder: &'static str,
    on_input: Option<Callback<()>>,
) -> impl IntoView {
    // The gate and the payload come from one `submit_gate` call (#860, ADR-0105), so a
    // control that cannot dispatch is disabled rather than inert. No slug in this shape,
    // so the only other blocker is the summary.
    let (submit_disabled, dispatch) = posts::submit_gate(
        state.body,
        Signal::derive(move || !state.summary_field.is_valid()),
        Callback::new(move |(body, publish): (PostBody, bool)| {
            let publication = posts::publication_from_local(publish, &state.publish_at.get());
            create_action.dispatch(Create {
                post: state.inputs(body, publication, None),
            });
        }),
    );
    view! {
        <div class="j-composer-row">
            {username.map(|u| view! { <Avatar name=&u size=36 /> })} <div class="j-composer-body">
                <ComposerFields
                    body=state.body
                    format=state.format
                    rows=rows
                    placeholder=placeholder
                    field_class="j-composer-field"
                    textarea_class=""
                    show_seg=false
                    on_input=on_input
                />
                <MediaUpload show_result=true />
                <div style="margin-top:10px">
                    <ValidatedTextarea<PostSummary>
                        label="Summary"
                        name="summary"
                        field=state.summary_field
                        placeholder="Optional summary or excerpt"
                    />
                </div>
                <TagInput tags=state.tags on_change=state.tag_input_changed() />
                <div class="j-composer-toolbar">
                    <FormatToggle format=state.format />
                    <span class="j-spacer"></span>
                    <PostSaveActions
                        publication=LoadedPublication::Draft
                        disabled=submit_disabled
                        on_save=dispatch
                    />
                </div>
            </div>
        </div>
        <CreateErrorFlash action=create_action />
    }
}

/// The full compose page: body column plus the options aside ([`ComposeOptions`]), the
/// media column ([`MediaSection`]) and the dispatch buttons. Split out of
/// [`PostCreateForm`] (#301). The slug field is owned here and passed down — see
/// [`ComposeState::seed_from`] for why the bundle does not hold it.
#[component]
fn FullComposer(
    state: ComposeState,
    create_action: ServerAction<Create>,
    rows: u32,
    placeholder: &'static str,
) -> impl IntoView {
    let slug_field = Field::<Slug>::optional();
    let named = audience::load_named_audiences();
    let schedule = CreationSchedule::new();
    // The one-call form gate also carries the named-audience load decision: a
    // failed or unresolved picker cannot dispatch as though an empty list had
    // loaded. The callback repeats the pure guard so direct invocation cannot
    // bypass the disabled buttons.
    let (submit_disabled, dispatch) = posts::submit_gate(
        state.body,
        Signal::derive(move || {
            !slug_field.is_valid()
                || !state.summary_field.is_valid()
                || schedule.has_unapplied_changes(&state.publish_at.get())
                || state.audience.with(|selection| {
                    named.with(|state| state.selection_for_submit(selection).is_none())
                })
        }),
        Callback::new(move |(body, publish): (PostBody, bool)| {
            let publication = posts::publication_from_local(publish, &state.publish_at.get());
            if state.audience.with(|selection| {
                named.with(|state| state.selection_for_submit(selection).is_some())
            }) {
                create_action.dispatch(Create {
                    post: state.inputs(body, publication, slug_field.parsed()),
                });
            }
        }),
    );
    view! {
        <div class="j-compose-grid">
            <div class="j-compose-body">
                <ComposerFields
                    body=state.body
                    format=state.format
                    rows=rows
                    placeholder=placeholder
                    field_class="j-composer-field"
                    textarea_class="j-edit-form-textarea"
                    show_seg=false
                />
            </div>
            <aside class="j-compose-aside">
                <ComposeOptions
                    state=state
                    slug_field=slug_field
                    publication=LoadedPublication::Draft
                    scheduled=None
                    schedule_error=Signal::derive(|| None::<InvalidSchedule>)
                    creation_schedule=Some(schedule)
                    named=named
                />
                <MediaSection />
                <div style="margin-top:auto;display:flex;align-items:center;gap:8px">
                    <CreationPostActions
                        publish_at=state.publish_at
                        scheduled=schedule.scheduled
                        disabled=submit_disabled
                        on_save=dispatch
                    />
                </div>
            </aside>
        </div>
        <CreateErrorFlash action=create_action />
    }
}

/// The create action's error flash. Both composer shapes ended with the identical
/// block; extracting it means a change to how a failed create reads happens once.
#[component]
fn CreateErrorFlash(action: ServerAction<Create>) -> impl IntoView {
    view! {
        {move || {
            action
                .value()
                .get()
                .and_then(|result: Result<MutationOutcome<SavedPost>, WebError>| match result {
                    Ok(MutationOutcome::Confirmed(_)) => None,
                    Ok(MutationOutcome::CommitIndeterminate(_)) => {
                        Some(
                            view! {
                                <p class="error">
                                    "The post may have been saved, but its status could not be confirmed. Refresh to check."
                                </p>
                            }
                                .into_any(),
                        )
                    }
                    Err(error) => {
                        Some(view! { <p class="error">{error.to_string()}</p> }.into_any())
                    }
                })
        }}
    }
}

#[component]
pub fn InlineComposer(username: Username, on_publish: Callback<()>) -> impl IntoView {
    let flash: RwSignal<Option<(String, String)>> = RwSignal::new(None);

    let on_success = Callback::new(move |created: SavedPost| {
        use leptos_dom::helpers::set_timeout;
        use std::time::Duration;
        let url = created.permalink.to_string();
        let msg = if created.published_at.is_some() {
            "Post published!".to_string()
        } else {
            "Draft saved!".to_string()
        };
        flash.set(Some((url, msg)));
        set_timeout(move || flash.set(None), Duration::from_secs(30));
    });
    let on_mutation = Callback::new(move |published: bool| {
        if published {
            on_publish.run(());
        }
    });

    view! {
        <div class="j-composer">
            <PostCreateForm
                compact=true
                username=username
                on_success=on_success
                on_mutation=on_mutation
                rows=6
                placeholder="What\u{2019}s on your mind?"
                on_input=Callback::new(move |()| flash.set(None))
            />
            {move || {
                flash
                    .get()
                    .map(|(url, msg)| {
                        view! {
                            <p class="success">
                                <a href=url>{msg}</a>
                            </p>
                        }
                    })
            }}
        </div>
    }
}

// ---------------------------------------------------------------------------
// Routed page components (#323).
// ---------------------------------------------------------------------------

#[component]
pub fn CreatePostPage() -> impl IntoView {
    // Server-confirmed gate: await the shared session reconcile (an expired cookie
    // must not show the create form) (#591).
    let session = auth::use_session();
    let last_result: RwSignal<Option<SavedPost>> = RwSignal::new(None);

    view! {
        <Topbar title="New post" sub="Long-form" />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match session.reconcile.await {
                    Ok(Some(_)) => {
                        view! {
                            <PostCreateForm
                                compact=false
                                rows=16
                                placeholder="Write something\u{2026}"
                                on_success=Callback::new(move |created| {
                                    last_result.set(Some(created));
                                })
                            />
                            <CreateResultSummary result=last_result />
                        }
                            .into_any()
                    }
                    Ok(None) => {
                        view! {
                            <div style="padding:32px">
                                <p>"You must be logged in to create a post."</p>
                                <p>
                                    <a href="/login" class="j-btn is-primary">
                                        "Sign in"
                                    </a>
                                </p>
                            </div>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}

/// The confirmed create result shown below the routed full composer.
///
/// This is intentionally separate from [`CreatePostPage`]: it owns the publication
/// outcome wording and the stable post-navigation test hooks, while the page owns
/// authentication reconciliation and form presentation.
#[component]
fn CreateResultSummary(result: RwSignal<Option<SavedPost>>) -> impl IntoView {
    view! {
        {move || {
            result
                .get()
                .map(|created| {
                    let message = match created.published_at {
                        Some(at) if at.value() > UtcInstant::now().value() => "Post scheduled.",
                        Some(_) => "Post published.",
                        None => "Draft saved.",
                    };
                    let slug_value = created.slug.to_string();
                    let slug_for_attr = slug_value.clone();
                    view! {
                        <div class="j-save-summary">
                            <p class="success">{message}</p>
                            <p data-test="slug-value" data-slug=slug_for_attr>
                                "Slug: "
                                {slug_value}
                            </p>
                            <a data-test="permalink-link" href=created.permalink.to_string()>
                                "View post"
                            </a>
                        </div>
                    }
                })
        }}
    }
}

/// Shared creation and editor save controls: "Save draft" + "Publish" for a draft,
/// and a lone "Save" for scheduled or live Posts.
///
/// The callers retain their layout wrappers; this component owns only the stable
/// publication branch and button contract. `on_save` is a plain data callback (the
/// `publish` flag), not a view closure — ADR-0083 §3 rules out passing markup as a prop.
#[component]
pub(super) fn PostSaveActions(
    /// Publication state that selects the draft pair or the scheduled/live Save control.
    publication: LoadedPublication,
    /// Whether saving is currently blocked by invalid form state.
    disabled: Signal<bool>,
    /// Dispatches the save; the argument is the requested `publish` flag.
    on_save: Callback<bool>,
) -> impl IntoView {
    view! {
        {if matches!(publication, LoadedPublication::Draft) {
            view! {
                <button
                    class="j-btn"
                    type="button"
                    name="publish"
                    value="false"
                    prop:disabled=move || disabled.get()
                    on:click=move |_| on_save.run(false)
                >
                    "Save draft"
                </button>
                <button
                    class="j-btn is-primary"
                    type="button"
                    name="publish"
                    value="true"
                    prop:disabled=move || disabled.get()
                    on:click=move |_| on_save.run(true)
                >
                    "Publish"
                </button>
            }
                .into_any()
        } else {
            view! {
                <button
                    class="j-btn is-primary"
                    type="button"
                    name="publish"
                    value="true"
                    prop:disabled=move || disabled.get()
                    on:click=move |_| on_save.run(true)
                >
                    "Save"
                </button>
            }
                .into_any()
        }}
    }
}

/// Full-composer creation controls, whose primary action reflects a committed schedule.
#[component]
fn CreationPostActions(
    /// The committed value distinguishes Draft from either publication state; reset clears it.
    publish_at: RwSignal<String>,
    /// Future-versus-already-due classification fixed when the value was applied.
    scheduled: RwSignal<bool>,
    disabled: Signal<bool>,
    on_save: Callback<bool>,
) -> impl IntoView {
    view! {
        {move || match (!publish_at.get().is_empty(), scheduled.get()) {
            (true, true) => {
                view! {
                    <button
                        class="j-btn is-primary"
                        type="button"
                        name="publish"
                        value="true"
                        prop:disabled=move || disabled.get()
                        on:click=move |_| on_save.run(true)
                    >
                        "Schedule"
                    </button>
                }
                    .into_any()
            }
            (true, false) => {
                view! {
                    <button
                        class="j-btn is-primary"
                        type="button"
                        name="publish"
                        value="true"
                        prop:disabled=move || disabled.get()
                        on:click=move |_| on_save.run(true)
                    >
                        "Publish"
                    </button>
                }
                    .into_any()
            }
            (false, _) => {
                view! {
                    <button
                        class="j-btn"
                        type="button"
                        name="publish"
                        value="false"
                        prop:disabled=move || disabled.get()
                        on:click=move |_| on_save.run(false)
                    >
                        "Save draft"
                    </button>
                    <button
                        class="j-btn is-primary"
                        type="button"
                        name="publish"
                        value="true"
                        prop:disabled=move || disabled.get()
                        on:click=move |_| on_save.run(true)
                    >
                        "Publish"
                    </button>
                }
                    .into_any()
            }
        }}
    }
}

/// Draft-only slug override control shared by the full composer and editor.
///
/// The control is deliberately slug-specific: ADR-0065's generic labelled
/// components remain the default for ordinary fields, while this options-aside row
/// keeps its bespoke grid layout and direct `Field<Slug>` binding.
#[component]
pub(super) fn SlugOverrideInput(slug_field: Field<Slug>) -> impl IntoView {
    view! {
        <label class="j-field-row" style="grid-template-columns:auto 1fr">
            <span class="j-field-label">"Slug"</span>
            <ValidatedBareInput<Slug>
                name="slug_override"
                field=slug_field
                placeholder=Some("auto")
                class=Some("j-field-val")
            />
            {forms::validated_error(
                slug_field.error(),
                Signal::derive(move || slug_field.is_touched()),
                |msg| view! { <span class="error">{msg}</span> }.into_any(),
            )}
        </label>
    }
}

/// The options aside shared by the full-page composer and editor.
///
/// The immutable loaded publication state owns which controls exist: drafts show
/// slug and schedule, scheduled Posts show only their schedule, and live Posts show
/// neither. The remaining fields are common to every branch.
#[component]
pub(super) fn ComposeOptions(
    state: ComposeState,
    /// Page-level rather than held by [`ComposeState`] — see
    /// [`ComposeState::seed_from`].
    slug_field: Field<Slug>,
    publication: LoadedPublication,
    scheduled: Option<ScheduledEditState>,
    schedule_error: Signal<Option<InvalidSchedule>>,
    /// Present only for the full new-Post composer, whose time choice is provisional.
    creation_schedule: Option<CreationSchedule>,
    /// The named-audience load shared by the picker and the action gate.
    named: RwSignal<NamedAudienceState>,
) -> impl IntoView {
    view! {
        <div>
            <div class="j-sb-head" style="padding:0 0 10px">
                "Options"
            </div>
            {match publication {
                LoadedPublication::Draft => {
                    view! {
                        <SlugOverrideInput slug_field=slug_field />
                        {match creation_schedule {
                            Some(schedule) => {
                                view! { <CreationScheduleControl state=state schedule=schedule /> }
                                    .into_any()
                            }
                            None => {
                                view! {
                                    <ScheduleControl
                                        state=state
                                        scheduled=None
                                        schedule_error=schedule_error
                                    />
                                }
                                    .into_any()
                            }
                        }}
                    }
                        .into_any()
                }
                LoadedPublication::Scheduled(_) => {
                    view! {
                        <ScheduleControl
                            state=state
                            scheduled=scheduled
                            schedule_error=schedule_error
                        />
                    }
                        .into_any()
                }
                LoadedPublication::Live => ().into_any(),
            }}
            <div style="margin-top:10px">
                <ValidatedTextarea<PostSummary>
                    label="Summary"
                    name="summary"
                    field=state.summary_field
                    placeholder="Optional summary or excerpt"
                />
            </div>
            <div style="margin-top:10px">
                <TagInput tags=state.tags on_change=state.tag_input_changed() />
            </div>
            <div style="margin-top:10px">
                <AudiencePickerWithState selection=state.audience named=named />
            </div>
            <FormatToggle format=state.format style="margin-top:10px" />
        </div>
    }
}

/// The inline schedule disclosure used only by the full new-Post composer.
#[component]
fn CreationScheduleControl(state: ComposeState, schedule: CreationSchedule) -> impl IntoView {
    let open = move |_| {
        schedule.restore_committed(&state.publish_at.get());
        schedule.disclosed.set(true);
    };
    let clear = move |_| {
        state.publish_at.set(String::new());
        schedule.scheduled.set(false);
        schedule.error.set(None);
    };
    view! {
        <div style="margin-top:10px">
            {move || {
                if schedule.disclosed.get() {
                    view! { <CreationScheduleEditor state=state schedule=schedule /> }.into_any()
                } else if state.publish_at.get().is_empty() {
                    view! {
                        <button class="j-btn" type="button" on:click=open>
                            "Set publication time…"
                        </button>
                    }
                        .into_any()
                } else {
                    let value = state.publish_at.get().replace('T', " ");
                    view! {
                        <p>{format!("Publication time: {value} local time")}</p>
                        <button class="j-btn" type="button" on:click=open>
                            "Change publication time…"
                        </button>
                        <button class="j-btn" type="button" on:click=clear>
                            "Clear schedule"
                        </button>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

/// The provisional date/time editor for a new Post's creation-only schedule.
#[component]
fn CreationScheduleEditor(state: ComposeState, schedule: CreationSchedule) -> impl IntoView {
    let cancel = move |_| {
        schedule.restore_committed(&state.publish_at.get());
        schedule.disclosed.set(false);
    };
    let apply = move |_| {
        let value = format!("{}T{}", schedule.date.get(), schedule.time.get());
        if let Some(at) = time::strict_utc_instant_from_local(&value) {
            schedule
                .scheduled
                .set(at.value() > UtcInstant::now().value());
            state.publish_at.set(value);
            schedule.error.set(None);
            schedule.disclosed.set(false);
        } else {
            schedule
                .error
                .set(Some("Enter a valid local date and time.".to_owned()));
        }
    };
    view! {
        <p>"Publication time uses your browser's local timezone."</p>
        <label class="j-field-label">
            "Date"
            <input
                type="date"
                name="publish_date"
                class="j-field-val"
                prop:value=schedule.date
                on:input=move |ev| {
                    let date = event_target_value(&ev);
                    schedule.date.set(date.clone());
                    if !date.is_empty() && schedule.time.get().is_empty() {
                        schedule.time.set("00:00".to_owned());
                    }
                    schedule.error.set(None);
                }
            />
        </label>
        <label class="j-field-label">
            "Time"
            <input
                type="time"
                name="publish_time"
                class="j-field-val"
                prop:value=schedule.time
                on:input=move |ev| {
                    schedule.time.set(event_target_value(&ev));
                    schedule.error.set(None);
                }
            />
        </label>
        {move || schedule.error.get().map(|error| view! { <p class="error">{error}</p> })}
        <button class="j-btn" type="button" on:click=apply>
            "Apply"
        </button>
        <button class="j-btn" type="button" on:click=cancel>
            "Cancel"
        </button>
    }
}

/// The draft/scheduled publication-time control.
///
/// A scheduled editor writes through [`ScheduledEditState`] so changing display
/// text marks the exact loaded instant as replaced. A draft writes the composer's
/// ordinary optional schedule field.
#[component]
pub(super) fn ScheduleControl(
    state: ComposeState,
    scheduled: Option<ScheduledEditState>,
    schedule_error: Signal<Option<InvalidSchedule>>,
) -> impl IntoView {
    view! {
        <div style="margin-top:10px">
            {match scheduled {
                Some(schedule) => {
                    view! {
                        <label class="j-field-label">
                            "Publish at"
                            <input
                                type="datetime-local"
                                name="publish_at"
                                class="j-field-val"
                                prop:value=schedule.value
                                on:input=move |ev| schedule.set_input(event_target_value(&ev))
                            />
                            {move || {
                                schedule_error
                                    .get()
                                    .map(|err| {
                                        view! { <span class="error">{err.to_string()}</span> }
                                    })
                            }}
                        </label>
                        <button class="j-btn" type="button" on:click=move |_| schedule.clear()>
                            "Clear schedule"
                        </button>
                    }
                        .into_any()
                }
                None => {
                    view! {
                        <label class="j-field-label">
                            "Publish at (optional)"
                            <input
                                type="datetime-local"
                                name="publish_at"
                                class="j-field-val"
                                prop:value=state.publish_at
                                on:input=move |ev| state.publish_at.set(event_target_value(&ev))
                            />
                        </label>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

/// The media column shared by the two full-page compose shapes.
///
/// Extracted from [`FullComposer`] and [`EditPostForm`] (#863), which held
/// byte-identical copies. Emits a single wrapping `<div>` on purpose: both asides are
/// flex columns with `gap:18px`, so a bare fragment would space the heading off the
/// control it labels.
#[component]
pub(super) fn MediaSection() -> impl IntoView {
    view! {
        <div style="margin-top:16px">
            <div class="j-sb-head" style="padding:0 0 10px">
                "Media"
            </div>
            <MediaUpload show_result=true />
        </div>
    }
}
