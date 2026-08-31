//! The **posts** vertical's wasm-only UI (ADR-0070): the reactive post widgets —
//! the composer/create form and its shared body fields, the inline composer, the
//! post card/display, and the audience picker. Declared
//! `#[cfg(target_arch = "wasm32")] mod component;` in `posts/mod.rs`, so this file
//! is wasm-only by its `mod` declaration and carries no cfg gates of its own; it
//! calls browser APIs directly. The pure, projector-coincident render twins live
//! in the host-tested [`super::render`]; the scheduled-publish datetime conversion
//! is the host-tested [`common::time::utc_instant_from_local`].

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_params_map};

use client::telemetry;
use common::permalink_route::PermalinkRoute;
use common::seed::Page;
// `Summary` is module-qualified at its use site: this file already has
// `PostSummary` and `TagSummary` in scope, and a bare `Summary` among them says
// nothing about which one it is.
use crate::audiences;
use crate::auth;
use crate::avatar::Avatar;
use crate::error::WebError;
use crate::feed_discovery::{FeedDiscovery, RsdDiscovery};
use crate::forms::{self, Field, ValidatedBareInput, ValidatedTextarea};
use crate::media::MediaUpload;
// `Get`/`Update` are deliberately absent from this list: naming the generated
// structs here would shadow the identically-named `leptos::prelude` traits this
// file's 86 `.get()`/`.update()` calls resolve through. `Update` is spelled
// `super::Update` at its use sites; `Get` is not needed in this file at all, and
// is named here only so a future author does not add it to the list.
use super::render;
use crate::posts::{
    ComposeState, Create, Delete, DraftRowDisplay, EditPublicationState, InvalidSchedule,
    ListingRoute, LoadedPublication, NamedAudienceState, PublicationIntent, Publish, SavedPost,
    ScheduledEditState, Unpublish, UnpublishedPost,
};
use crate::subscriptions::SubscribeButton;
use crate::taglist::TagCtx;
use crate::tags::TagInput;
use crate::timeline::{self, TimelineGate, TimelineState};
use crate::topbar::Topbar;
use common::pagination::PageSize;
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::render::PostFormat;
use common::root_relative_url::RootRelativeUrl;
use common::seed::{AuthoredPost, PageSeed, RenderedPost};
use common::slug::Slug;
use common::tag::Tag;
use common::username::Username;
use common::visibility::{AudienceBase, AudienceSelection};
use common::{client_telemetry::ClientErrorContext, feed::FeedSurface, ids::PostId};

/// Register an `Effect` that runs `on_ok` with the resolved value each time `resolved`
/// settles to a success.
///
/// Every async lifecycle hook in this vertical spelled out the same shape —
/// `if let Some(Ok(v)) = <resource-or-action>.get() { … }` — a branch over "not yet"
/// and "failed" that says nothing about the component it sat in. Taking the read as a
/// closure serves both `Resource::get` and `ServerAction::value().get()` without naming
/// either type, and keeps the branch out of the component bodies (#306). The read stays
/// *inside* the effect, so the reactive dependency is unchanged.
fn on_settled_ok<T, E, R, F>(resolved: R, on_ok: F)
where
    R: Fn() -> Option<Result<T, E>> + 'static,
    F: Fn(T) + 'static,
{
    Effect::new(move |_| {
        if let Some(value) = resolved().and_then(Result::ok) {
            on_ok(value);
        }
    });
}

fn canonical_username_display(username: Memo<Option<Username>>) -> impl Fn() -> String {
    move || username.get().map(String::from).unwrap_or_default()
}

/// The `.j-seg` Markdown/Org format toggle, shared by every post editor. Renders one
/// button per user-selectable `PostFormat` — those carrying a `strum` editor message;
/// `Html` has none (renderer-internal, #445), so it is filtered out. Adding a format is
/// a one-attribute change on `PostFormat`, not new markup here.
#[component]
pub fn FormatToggle(
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

#[component]
pub fn PostDisplay<'a>(
    post: &'a RenderedPost,
    banner: Option<&'a str>,
    /// Linking context for the tag chips in the footer; defaults to
    /// site-wide.
    #[prop(default = &TagCtx::SiteWide)]
    tag_context: &'a TagCtx,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView + use<> {
    let time_label = render::format_post_time(post.display_time());
    // Built once and shared by both arms so the authored content column is the SAME
    // pure, viewer-independent render the projector paints (#181, ADR-0044 D4) — no
    // hand-rebuilt markup and no is_author-driven content change that could diverge
    // and reintroduce a flash. The action column is layered on additively.
    let view = render::PostView {
        username: &post.username,
        title: post.title.as_ref(),
        banner,
        summary: post.summary.as_ref(),
        rendered_html: &post.rendered_html,
        time: &time_label,
        permalink: post.permalink.as_ref(),
        tags: &post.tags,
        tag_ctx: tag_context,
    };
    match children {
        // Anonymous / no-action layout: the WHOLE article inner is produced by the
        // pure `render` layer — the SAME code the public projector server-renders
        // (#179) — and injected via `inner_html`, so a seeded first paint and this
        // reactive re-render are byte-identical (flash-free). "Share the pure fn,
        // not the component" (ADR-0041 §4). The projector only ever renders this
        // anonymous view, so this is the only path that must coincide.
        None => {
            let inner = render::render_post_inner(&view);
            inner
                .inject_into(leptos::html::article().class("j-post"))
                .into_any()
        }
        // Authored layout (own posts, with the action column). The content column is
        // the SAME `render_post_content` the anonymous arm wraps, injected via
        // `inner_html` so it coincides with the projector's paint (#181); only the
        // reactive action column (`children`, carrying edit/delete handlers that
        // `inner_html` can't) overlays it as a sibling — hand-rebuilt reactive
        // markup here would diverge from the projector and reintroduce the flash.
        Some(children) => {
            let inner_content = render::render_post_content(&view);
            view! {
                <article class="j-post">
                    <Avatar name=&post.username size=38 />
                    <div style="min-width:0;display:flex;gap:8px;align-items:flex-start">
                        {inner_content.inject_into(leptos::html::div().style("flex:1;min-width:0"))}
                        {children()}
                    </div>
                </article>
            }
            .into_any()
        }
    }
}

/// `true` when the shared session's username equals `author` (#181/#591): the
/// client-side signal that the viewer owns this post, so its action column shows
/// even though the anonymous seed data has `is_author = false`. `false` on the host
/// build / outside the provider (no context) — the affordance is wasm-only chrome.
/// Uses `use_context` (not `use_session`) so the host build yields `None`→`false`
/// rather than panicking; reads untracked to match the original non-reactive read.
fn marker_matches(author: &Username) -> bool {
    use_context::<crate::auth::SessionContext>()
        .and_then(|ctx| ctx.current.get_untracked())
        .as_ref()
        .map(|user| &user.username)
        == Some(author)
}
fn dispatch_after_confirm(message: &str, context: ClientErrorContext, dispatch: impl FnOnce()) {
    match client::dialog::confirm(message) {
        Ok(outcome) => {
            if outcome.should_dispatch() {
                dispatch();
            }
        }
        Err(error) => {
            let source_kind = error.source_kind();
            telemetry::report_swallowed(telemetry::error_kind(source_kind), context, source_kind);
        }
    }
}
fn primary_post_action(
    is_draft: bool,
    post_id: PostId,
    publish_action: ServerAction<Publish>,
    unpublish_action: ServerAction<Unpublish>,
) -> AnyView {
    if is_draft {
        view! {
            <button
                type="button"
                class="j-btn"
                on:click=move |_| {
                    dispatch_after_confirm(
                        "Publish this draft?",
                        ClientErrorContext::PublishConfirm,
                        || {
                            publish_action.dispatch(Publish { post_id });
                        },
                    );
                }
            >
                "Publish"
            </button>
        }
        .into_any()
    } else {
        view! {
            <button
                type="button"
                class="j-btn"
                on:click=move |_| {
                    unpublish_action.dispatch(Unpublish { post_id });
                }
            >
                "Unpublish"
            </button>
        }
        .into_any()
    }
}

#[component]
pub fn PostCard<'a>(
    post: &'a RenderedPost,
    banner: Option<&'a str>,
    /// Linking context for the footer tag chips; defaults to site-wide.
    #[prop(default = &TagCtx::SiteWide)]
    tag_context: &'a TagCtx,
    #[prop(optional)] on_mutate: Option<Callback<()>>,
    #[prop(optional)] on_unpublish: Option<Callback<()>>,
    /// Fired only after a successful *publish* (distinct from `on_mutate`, which delete
    /// and unpublish share). The permalink page uses it to refetch itself in place when a
    /// same-URL publish leaves navigation a no-op, without delete/unpublish also
    /// refetching into a not-found (#592).
    #[prop(optional)]
    on_publish: Option<Callback<()>>,
) -> impl IntoView + use<> {
    // The seed/anonymous data has `is_author = false` (the projector paints
    // anonymous-only), so on the Local timeline the owner's own posts would show no
    // action column. Decide it client-side from the auth marker (#181, ADR-0044 D4)
    // so the affordance appears synchronously at mount. The server still authorizes
    // the actual edit/delete by session — the marker only gates visibility.
    let is_author = post.is_author || marker_matches(&post.username);
    let post_id = post.post_id;
    // A draft rendered at its permalink gets a Publish affordance instead of
    // Unpublish (#23): an Unpublish column would be a no-op on an already-
    // unpublished post.
    let is_draft = post.is_draft();
    let edit_url = super::edit_post_url(post_id);
    let history_url = format!("/posts/{}/history", i64::from(post_id));
    let delete_action = ServerAction::<Delete>::new();
    let unpublish_action = ServerAction::<Unpublish>::new();
    let publish_action = ServerAction::<Publish>::new();
    let deleted = RwSignal::new(false);

    on_settled_ok(
        move || delete_action.value().get(),
        move |()| {
            deleted.set(true);
            super::notify(on_mutate);
        },
    );
    on_settled_ok(
        move || unpublish_action.value().get(),
        // Unpublish prefers its own callback and falls back to the shared mutate one —
        // a per-caller policy, so it is the host-tested `notify_with_fallback` (#306).
        // The returned `SavedPost` is deliberately unread: this caller navigates to
        // /drafts regardless (#783 tracks using the moved permalink here).
        move |_| super::notify_with_fallback(on_unpublish, on_mutate),
    );
    // Client-only navigation side-effect (web-style-guide §9): react to the
    // resolved publish action, mirroring EditPostPage's publish redirect.
    let navigate = use_navigate();
    on_settled_ok(
        move || publish_action.value().get(),
        move |published: SavedPost| {
            // Publishing can move the permalink (a draft's URL is created_at-based;
            // once published it becomes published_at-based), so navigate client-side to
            // the server-returned canonical permalink rather than the now-stale current
            // URL. When it does NOT move (a same-UTC-day publish → identical URL), the
            // navigate is a no-op, so also fire `on_publish` to refetch the current page's
            // resource — otherwise a permalink page would keep showing the draft state
            // (#592). The unpublish path navigates to /drafts; this is its mirror.
            navigate(&published.permalink, NavigateOptions::default());
            super::notify(on_publish);
        },
    );

    let primary_action = primary_post_action(is_draft, post_id, publish_action, unpublish_action);

    // Additive action column (#181, ADR-0044 D4): edit / publish-or-unpublish /
    // delete. The timestamp deliberately stays in the (coincident) content-column
    // header rather than moving here, so the owner's own post doesn't diverge from
    // the anon paint.
    let action_col = is_author.then(move || {
        view! {
            <div class="j-post-acts">
                <a class="j-btn" href=String::from(edit_url)>
                    "Edit"
                </a>
                <a class="j-btn" data-test="post-history-link" href=history_url>
                    "History"
                </a>
                {primary_action}
                <button
                    type="button"
                    class="j-btn is-danger"
                    on:click=move |_| {
                        dispatch_after_confirm(
                            "Delete this post?",
                            ClientErrorContext::DeleteConfirm,
                            || {
                                delete_action.dispatch(Delete { post_id });
                            },
                        );
                    }
                >
                    "Delete"
                </button>
            </div>
        }
    });

    view! {
        {move || {
            deleted.get().then(|| view! { <p class="success">"Post deleted."</p> }.into_any())
        }}
        <PostDisplay post=post banner=banner tag_context=tag_context>
            {action_col}
        </PostDisplay>
    }
}

/// Start the named-audience load and project every resource outcome into the
/// explicit host-tested state consumed by both the picker and its submit gate.
fn load_named_audiences() -> RwSignal<NamedAudienceState> {
    let state = RwSignal::new(NamedAudienceState::Loading);
    let named = Resource::new(|| (), |()| audiences::list_mine());
    Effect::new(move |_| {
        state.set(NamedAudienceState::resolve(named.get()));
    });
    state
}

/// Per-post visibility control for the editor.
///
/// Drives a shared `selection` signal: a mutually-exclusive base
/// (Public / Private / Subscribers) plus a checkbox per named audience the
/// author owns (union semantics — e.g. Public + a named audience). `Private`
/// is author-only and the storage layer drops any named selection for it
/// (see `audience_selection_to_targets`); the named checkboxes are disabled
/// while Private is chosen to make that explicit.
#[component]
pub fn AudiencePicker(selection: RwSignal<AudienceSelection>) -> impl IntoView {
    let named = load_named_audiences();
    view! { <AudiencePickerWithState selection=selection named=named /> }
}

/// The picker view over a load state shared with its owning action gate.
#[component]
fn AudiencePickerWithState(
    selection: RwSignal<AudienceSelection>,
    named: RwSignal<NamedAudienceState>,
) -> impl IntoView {
    let change_base = move |ev| {
        if let Ok(base) = AudienceBase::try_from(event_target_value(&ev).as_str()) {
            selection.update(|current| current.base = base);
        }
    };

    view! {
        <div class="j-field-row" style="grid-template-columns:auto 1fr">
            <label class="j-field-label" for="audience-base">
                "Audience"
            </label>
            <select id="audience-base" class="j-field-val" on:change=change_base>
                <For
                    // Each base variant is paired with its caption here, so the
                    // values and visible order cannot drift apart.
                    each=|| {
                        [
                            (AudienceBase::Public, "Public"),
                            (AudienceBase::Subscribers, "Subscribers"),
                            (AudienceBase::Private, "Private (only me)"),
                        ]
                    }
                    key=|(base, _)| *base
                    children=move |(base, label)| {
                        view! {
                            <option
                                value=base.to_string()
                                selected=move || selection.get().base == base
                            >
                                {label}
                            </option>
                        }
                    }
                />
            </select>
        </div>
        <NamedAudienceOptions named=named selection=selection />
    }
}

/// Loading, failure, or successfully loaded named-audience options.
#[component]
fn NamedAudienceOptions(
    named: RwSignal<NamedAudienceState>,
    selection: RwSignal<AudienceSelection>,
) -> impl IntoView {
    view! {
        <Show
            when=move || named.with(|state| matches!(state, NamedAudienceState::Loading))
            fallback=move || {
                view! {
                    <Show
                        when=move || named.with(|state| matches!(state, NamedAudienceState::Failed))
                        fallback=move || {
                            view! { <ReadyNamedAudienceOptions named=named selection=selection /> }
                        }
                    >
                        <p class="error">"Could not load named audiences."</p>
                    </Show>
                }
            }
        >
            <p class="j-loading">"Loading\u{2026}"</p>
        </Show>
    }
}

/// A successful named-audience load, split between genuine empty and rows.
#[component]
fn ReadyNamedAudienceOptions(
    named: RwSignal<NamedAudienceState>,
    selection: RwSignal<AudienceSelection>,
) -> impl IntoView {
    view! {
        <Show
            when=move || {
                named
                    .with(|state| {
                        matches!(
                            state,
                            NamedAudienceState::Ready(audiences)
                            if audiences.is_empty()
                        )
                    })
            }
            fallback=move || {
                view! { <NamedAudienceRows named=named selection=selection /> }
            }
        >
            <p class="j-sub">"No named audiences."</p>
        </Show>
    }
}

/// Checkbox rows for a successfully loaded, non-empty named-audience list.
#[component]
fn NamedAudienceRows(
    named: RwSignal<NamedAudienceState>,
    selection: RwSignal<AudienceSelection>,
) -> impl IntoView {
    let audiences = move || {
        named.with(|state| match state {
            NamedAudienceState::Ready(audiences) => audiences.clone(),
            NamedAudienceState::Loading | NamedAudienceState::Failed => Vec::new(),
        })
    };

    view! {
        <div style="margin-top:8px">
            <span class="j-field-label">"Also share with"</span>
            <For
                each=audiences
                key=|audience| audience.audience_id
                children=move |audience| audience_checkbox(audience, selection)
            />
        </div>
    }
}

/// One named-audience checkbox row for [`AudiencePicker`]. Toggling it
/// adds/removes the audience id in the shared selection. Disabled while the
/// base is `Private`, since Private cannot combine with named audiences.
fn audience_checkbox(
    audience: audiences::Summary,
    selection: RwSignal<AudienceSelection>,
) -> impl IntoView {
    let id = audience.audience_id;
    let input_id = format!("audience-named-{id}");
    let checked = move || selection.get().named.contains(&id);
    let disabled = move || selection.get().base == AudienceBase::Private;
    view! {
        <label style="display:block" for=input_id.clone()>
            <input
                id=input_id.clone()
                type="checkbox"
                prop:checked=checked
                disabled=disabled
                on:change=move |ev| {
                    let on = event_target_checked(&ev);
                    selection
                        .update(|sel| {
                            sel.named.retain(|x| *x != id);
                            if on {
                                sel.named.push(id);
                            }
                        });
                }
            />
            " "
            {String::from(audience.name)}
        </label>
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
    #[prop(default = 6)] rows: u32,
    #[prop(default = "What\u{2019}s on your mind?")] placeholder: &'static str,
    /// Called on every textarea input event (compact mode only).
    #[prop(optional)]
    on_input: Option<Callback<()>>,
) -> impl IntoView {
    let create_action = ServerAction::<Create>::new();
    let state = ComposeState::new();

    let default_audience = Resource::new(|| (), |()| super::get_default_audience_selection());
    // The site-wide default audience resolves asynchronously; the composer must
    // render immediately (no Suspense), so seed the editable `audience` signal
    // once the Resource resolves, over the Public placeholder `ComposeState::new`
    // sets. The author can then edit the selection via `AudiencePicker`.
    on_settled_ok(
        move || default_audience.get(),
        move |default| state.audience.set(default),
    );

    // A successful create hands the result to the parent and empties the composer for
    // the next post.
    on_settled_ok(
        move || create_action.value().get(),
        move |created| {
            on_success.run(created);
            state.reset();
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
    let (submit_disabled, dispatch) = super::submit_gate(
        state.body,
        Signal::derive(move || !state.summary_field.is_valid()),
        Callback::new(move |(body, publish): (PostBody, bool)| {
            let publication = super::publication_from_local(publish, &state.publish_at.get());
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
    let named = load_named_audiences();
    // The one-call form gate also carries the named-audience load decision: a
    // failed or unresolved picker cannot dispatch as though an empty list had
    // loaded. The callback repeats the pure guard so direct invocation cannot
    // bypass the disabled buttons.
    let (submit_disabled, dispatch) = super::submit_gate(
        state.body,
        Signal::derive(move || {
            !slug_field.is_valid()
                || !state.summary_field.is_valid()
                || state.audience.with(|selection| {
                    named.with(|state| state.selection_for_submit(selection).is_none())
                })
        }),
        Callback::new(move |(body, publish): (PostBody, bool)| {
            let publication = super::publication_from_local(publish, &state.publish_at.get());
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
                    named=named
                />
                <MediaSection />
                <div style="margin-top:auto;display:flex;align-items:center;gap:8px">
                    <PostSaveActions
                        publication=LoadedPublication::Draft
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
                .and_then(Result::err)
                .map(|e| view! { <p class="error">{e.to_string()}</p> })
        }}
    }
}

#[component]
pub fn InlineComposer(username: Username, on_publish: WriteSignal<u32>) -> impl IntoView {
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
        if created.published_at.is_some() {
            on_publish.update(|v| *v += 1);
        }
    });

    view! {
        <div class="j-composer">
            <PostCreateForm
                compact=true
                username=username
                on_success=on_success
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
                            {move || {
                                last_result
                                    .get()
                                    .map(|created| {
                                        let message = if created.published_at.is_some() {
                                            "Post published."
                                        } else {
                                            "Draft saved."
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
                                                <a
                                                    data-test="permalink-link"
                                                    href=created.permalink.to_string()
                                                >
                                                    "View post"
                                                </a>
                                            </div>
                                        }
                                    })
                            }}
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

/// First-paint view for [`PostPage`]'s `Suspense`: the projector-seeded content
/// (flash-free) when the server painted this permalink, or a spinner while the
/// reactive fetch runs (client-side navigation, no seed).
fn permalink_first_paint(seed_post: Option<AuthoredPost>) -> AnyView {
    match seed_post {
        Some(seed) => {
            // Just the article — this fallback sits inside the reactive PostPage's
            // own `j-scroll`/`j-page`. `display:contents` keeps the host wrapper out
            // of the layout so it coincides with the projector's permalink page.
            let html = render::permalink_article(&seed.post);
            html.inject_into(leptos::html::div().style("display:contents"))
                .into_any()
        }
        None => view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any(),
    }
}

/// `PostPage`'s resource key: the decoded permalink route plus a refetch tick. The tick
/// is folded into the key (not merely tracked) so an in-place publish — same URL, so the
/// route is unchanged — still changes the key and forces a re-fetch (#592). Named so the
/// fetcher's parameter reads as one thing.
type PermalinkFetchKey = (Option<PermalinkRoute>, u32);

#[component]
pub fn PostPage() -> impl IntoView {
    // Public projector seed (#178/#179): the content the server painted for this
    // permalink. Adopted as the `Suspense` fallback below so first paint shows
    // real content (flash-free) instead of a spinner. The reactive fetch still
    // runs and takes over — restoring the author's edit/delete affordances when
    // the viewer owns the post — so this *enhances* rather than *replaces*.
    let seed_post = match use_context::<Option<PageSeed>>().flatten() {
        Some(PageSeed::Permalink(post)) => Some(post),
        _ => None,
    };

    let params = use_params_map();

    let route = move || {
        let params = params.get();
        // Decode the permalink route params into typed values client-side so
        // `get` takes a typed `Slug`/`Username` (ADR-0063 §4). The pure
        // all-or-nothing decoder is host-tested in `super::parse`.
        super::parse_permalink_route(
            params.get("username").as_deref(),
            params.get("year").as_deref(),
            params.get("month").as_deref(),
            params.get("day").as_deref(),
            params.get("slug").as_deref(),
        )
    };

    // Bump to force a refetch when the post mutates in place (a same-URL publish, where
    // navigation is a no-op — see the publish effect in `PostCard`). Folded into the
    // resource key alongside the route (#592).
    let refetch = RwSignal::new(0u32);
    let post = Resource::new(
        move || (route(), refetch.get()),
        |(route, _): PermalinkFetchKey| async move {
            let Some(route) = route else {
                // A malformed permalink — an unparseable username, an absent/non-numeric/
                // impossible date, or an unparseable slug — names no post that could
                // exist, so 404 client-side without a round-trip. The route's
                // `TildeUsername` segment guarantees the `~`, so a non-`~` server URL
                // (e.g. /media/…) never reaches this page at all (#592).
                return Err(WebError::validation("Invalid permalink"));
            };
            super::get(route.username, route.date, route.slug).await
        },
    );

    // Unpublish navigates client-side to /drafts (a fresh mount that refetches its own
    // list — its resource keys on the publish/delete action versions); publish refetches
    // this page in place via `on_publish` (#592).
    let navigate = use_navigate();
    let on_unpublish = Callback::new(move |()| {
        navigate("/drafts", NavigateOptions::default());
    });
    // Publish-only: refetch this page in place (delete/unpublish must NOT — delete shows
    // its own success and a refetch would 404; unpublish navigates away) (#592).
    let on_publish = Callback::new(move |()| refetch.update(|v| *v += 1));

    view! {
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=move || permalink_first_paint(
                    seed_post.clone(),
                )>
                    {move || Suspend::new(async move {
                        match post.await {
                            Ok(fetched) => {
                                let banner = fetched
                                    .post
                                    .is_draft()
                                    .then_some("Draft - visible only to you".to_string());
                                let tag_context = TagCtx::ForUser(fetched.post.username.clone());
                                // Both bound before the `view!`: the props are borrows
                                // now, so an inline temporary would be dropped inside
                                // the macro expansion (E0716).
                                view! {
                                    <PostCard
                                        post=&fetched.post
                                        banner=banner.as_deref()
                                        tag_context=&tag_context
                                        on_unpublish=on_unpublish
                                        on_publish=on_publish
                                    />
                                }
                                    .into_any()
                            }
                            Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                        }
                    })}
                </Suspense>
            </div>
        </div>
    }
    .into_any()
}

#[component]
pub fn UserTimelinePage() -> impl IntoView {
    let params = use_params_map();
    // Parse the `~username` route segment into `Username` once, at the source; an
    // invalid segment is `None` and every consumer handles the absence.
    let username = Memo::new(move |_| {
        params
            .get()
            .get("username")
            .unwrap_or_default()
            .strip_prefix('~')
            .and_then(|s| s.parse::<Username>().ok())
    });

    let mutate_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| mutate_version.update(|v| *v += 1));

    let initial_page = Resource::new(
        move || (username.get(), mutate_version.get()),
        |(username, _)| async move {
            timeline::list_by_user(
                super::user_query(username)?,
                None,
                Some(PageSize::default()),
            )
            .await
        },
    );

    let state = TimelineState::default();
    // Public projector seed (#178/#179): if the server painted this profile,
    // adopt its posts as the initial state so first paint shows content (no
    // Loading flash). The route guard — which keeps a client-side navigation to a
    // *different* profile from adopting the initial URL's seed — is the host-tested
    // `seeded_page` (#306); the reactive fetch still runs and takes over.
    state.adopt_seed(super::seeded_page(
        use_context::<Option<PageSeed>>().flatten(),
        &ListingRoute::Profile(username.get_untracked()),
    ));

    timeline::wire_timeline_resolve(state, initial_page);

    let on_load_more = Callback::new(move |()| {
        if let Ok(username) = super::user_query(username.get_untracked()) {
            timeline::spawn_load_more(state, move |cursor, limit| {
                timeline::list_by_user(username, cursor, limit)
            });
        }
    });

    let display_username = canonical_username_display(username);

    view! {
        {move || {
            username
                .get()
                .map(|username| {
                    let surface = FeedSurface::User {
                        username: username.clone(),
                    };
                    view! {
                        <FeedDiscovery surface=&surface />
                        <RsdDiscovery username=&username />
                    }
                })
        }}
        <Topbar title=move || format!("Posts by {}", display_username()) sub="User timeline" />
        {move || { username.get().map(|username| view! { <SubscribeButton username=username /> }) }}
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            tag_context=Signal::derive(move || username.get().map(TagCtx::ForUser))
        />
    }
}

#[component]
pub fn EditPostPage() -> impl IntoView {
    let params = use_params_map();
    let update_post_action = ServerAction::<super::Update>::new();
    // The editor edits the same seven fields the composer does and dispatches the
    // same `PostInputs` payload, so it reuses that bundle (#301). The slug field is
    // page-level here — unlike the composer, where only the full shape has one.
    let state = ComposeState::new();
    let slug_field = Field::<Slug>::optional();
    let named = load_named_audiences();
    // The redirect-on-publish effect reacts to the client-only ServerAction
    // dispatch. Whether a settled update redirects at all, and to where, is the
    // host-tested `publish_redirect` (#306), leaving this the bare `Effect`
    // `on_settled_ok` wraps.
    let navigate = use_navigate();
    on_settled_ok(
        move || super::publish_redirect(update_post_action.value().get()),
        move |permalink: RootRelativeUrl| navigate(&permalink, NavigateOptions::default()),
    );

    // A missing or unparseable `post_id` is honest absence, not a real id: derive
    // `Option<PostId>` and let the host-tested `with_post_id` short-circuit `None`
    // to a client-side not-found, rather than minting a sentinel id and paying a
    // round-trip that only ever returns not-found (#487).
    let post_id_param = move || {
        params
            .get()
            .get("post_id")
            .and_then(|v| v.parse::<PostId>().ok())
    };
    let post = Resource::new(post_id_param, |post_id| {
        super::with_post_id(post_id, super::get_preview)
    });
    // Seeded into the editable `audience` picker inside the `Suspense` block below
    // (awaited alongside `post`, not via a standalone Effect, since the page
    // already suspends on `post`). On a fetch error the Public default survives
    // (the `Ok`-only guard mirrors the dissolved post-resolve Effect). The intent
    // comment lives here, outside `view!`, because leptosfmt relocates comments
    // inside the macro.
    let current_audience = Resource::new(post_id_param, |post_id| {
        super::with_post_id(post_id, super::get_audience_selection)
    });

    view! {
        <Topbar title="Edit Post" sub="Long-form" />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok(fetched) => {
                        state.seed_from(&fetched.post);
                        slug_field.value.set(fetched.post.post.slug.to_string());
                        let publication = EditPublicationState::from_loaded(
                            super::loaded_publication(
                                fetched.post.post.published_at,
                                fetched.fetched_at,
                            ),
                            state.publish_at,
                        );
                        if let Ok(selection) = current_audience.await {
                            state.audience.set(selection);
                        }
                        // The slug is not part of the bundle (the compact shape has
                        // none) — see `seed_from`.
                        view! {
                            <EditPostForm
                                state=state
                                slug_field=slug_field
                                post_id=fetched.post.post.post_id
                                publication=publication
                                action=update_post_action
                                named=named
                            />
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        <EditSaveOutcome action=update_post_action />
    }
}

/// The editor's form: body column, publication-aware options, media, and save controls.
/// Split out of [`EditPostPage`] (#301), which keeps only the fetch and its branch.
#[component]
fn EditPostForm(
    state: ComposeState,
    /// Page-level rather than held by [`ComposeState`] — see
    /// [`ComposeState::seed_from`].
    slug_field: Field<Slug>,
    post_id: PostId,
    /// Publication branch and branch-specific signals fixed when the response loaded.
    publication: EditPublicationState,
    /// The named-audience load shared by the picker and the action gate.
    named: RwSignal<NamedAudienceState>,
    action: ServerAction<super::Update>,
) -> impl IntoView {
    // The body/field gate also waits for a real named-audience load. Repeating
    // the pure guard in the callback prevents a direct invocation from
    // dispatching while Loading or Failed.
    let also_blocked = Signal::derive(move || {
        !slug_field.is_valid()
            || !state.summary_field.is_valid()
            || state.audience.with(|selection| {
                named.with(|state| state.selection_for_submit(selection).is_none())
            })
    });
    let loaded_publication = publication.loaded();
    let scheduled = publication.scheduled();
    let (save_disabled, schedule_error, dispatch_update) = super::edit_submit_gate(
        state.body,
        also_blocked,
        publication,
        Callback::new(move |(body, publication): (PostBody, PublicationIntent)| {
            if state.audience.with(|selection| {
                named.with(|state| state.selection_for_submit(selection).is_some())
            }) {
                action.dispatch(super::Update {
                    post_id,
                    post: state.inputs(body, publication, slug_field.parsed()),
                });
            }
        }),
    );
    view! {
        <div class="j-compose-grid">
            <div class="j-edit-form-body">
                <ComposerFields
                    body=state.body
                    format=state.format
                    rows=20
                    field_class="j-edit-form-field j-edit-form-field--body"
                    textarea_class="j-edit-form-textarea"
                    show_seg=false
                />
            </div>
            <aside class="j-compose-aside">
                <ComposeOptions
                    state=state
                    slug_field=slug_field
                    publication=loaded_publication
                    scheduled=scheduled
                    schedule_error=schedule_error
                    named=named
                />
                <MediaSection />
                <div class="j-edit-form-actions">
                    <PostSaveActions
                        publication=loaded_publication
                        disabled=save_disabled
                        on_save=dispatch_update
                    />
                </div>
            </aside>
        </div>
    }
}

/// Shared creation and editor save controls: "Save draft" + "Publish" for a draft,
/// and a lone "Save" for scheduled or live Posts.
///
/// The callers retain their layout wrappers; this component owns only the stable
/// publication branch and button contract. `on_save` is a plain data callback (the
/// `publish` flag), not a view closure — ADR-0083 §3 rules out passing markup as a prop.
#[component]
fn PostSaveActions(
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

/// Draft-only slug override control shared by the full composer and editor.
///
/// The control is deliberately slug-specific: ADR-0065's generic labelled
/// components remain the default for ordinary fields, while this options-aside row
/// keeps its bespoke grid layout and direct `Field<Slug>` binding.
#[component]
fn SlugOverrideInput(slug_field: Field<Slug>) -> impl IntoView {
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
                slug_field.error,
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
fn ComposeOptions(
    state: ComposeState,
    /// Page-level rather than held by [`ComposeState`] — see
    /// [`ComposeState::seed_from`].
    slug_field: Field<Slug>,
    publication: LoadedPublication,
    scheduled: Option<ScheduledEditState>,
    schedule_error: Signal<Option<InvalidSchedule>>,
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
                        <ScheduleControl state=state scheduled=None schedule_error=schedule_error />
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

/// The draft/scheduled publication-time control.
///
/// A scheduled editor writes through [`ScheduledEditState`] so changing display
/// text marks the exact loaded instant as replaced. A draft writes the composer's
/// ordinary optional schedule field.
#[component]
fn ScheduleControl(
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
fn MediaSection() -> impl IntoView {
    view! {
        <div style="margin-top:16px">
            <div class="j-sb-head" style="padding:0 0 10px">
                "Media"
            </div>
            <MediaUpload show_result=true />
        </div>
    }
}

/// The save summary under the editor: the draft-saved block (slug + permalink) when the
/// post stayed unpublished, a "Redirecting…" notice when a publish is about to navigate
/// away, or the error. Nothing at all until the update action has a value.
///
/// Split out of [`EditPostPage`] (#306); this component owns that three-way decision.
#[component]
fn EditSaveOutcome(action: ServerAction<super::Update>) -> impl IntoView {
    view! {
        {move || {
            action
                .value()
                .get()
                .map(|result: Result<SavedPost, WebError>| match result {
                    Ok(updated) if updated.published_at.is_none() => {
                        let slug_value = updated.slug.to_string();
                        let slug_for_attr = slug_value.clone();
                        view! {
                            <div class="j-save-summary">
                                <p class="success">"Draft saved."</p>
                                <p data-test="slug-value" data-slug=slug_for_attr>
                                    "Slug: "
                                    {slug_value}
                                </p>
                                <a data-test="permalink-link" href=updated.permalink.to_string()>
                                    "View post"
                                </a>
                            </div>
                        }
                            .into_any()
                    }
                    Ok(_) => view! { <p>"Redirecting\u{2026}"</p> }.into_any(),
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
    }
}

#[component]
pub fn DraftsPage() -> impl IntoView {
    let publish_action = ServerAction::<Publish>::new();
    let delete_action = ServerAction::<Delete>::new();
    let drafts = Resource::new(
        move || {
            (
                publish_action.version().get(),
                delete_action.version().get(),
            )
        },
        |_| super::list_drafts(None, Some(PageSize::default())),
    );

    view! {
        <Topbar title="Drafts" sub="Unpublished posts" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        view! {
                            <DraftList
                                drafts=drafts.await
                                publish_action=publish_action
                                delete_action=delete_action
                            />
                        }
                    })}
                </Suspense>
                {move || {
                    publish_action
                        .value()
                        .get()
                        .map(|result: Result<SavedPost, WebError>| match result {
                            Ok(published) => {
                                view! {
                                    <p class="success">
                                        "Post published. "
                                        <a href=published.permalink.to_string()>"View permalink"</a>
                                    </p>
                                }
                                    .into_any()
                            }
                            Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                        })
                }}
                {move || {
                    delete_action
                        .value()
                        .get()
                        .map(|result| match result {
                            Ok(()) => view! { <p class="success">"Draft deleted."</p> }.into_any(),
                            Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                        })
                }}
            </div>
        </div>
    }
}

#[component]
pub fn ScheduledPage() -> impl IntoView {
    // Match the create-post gate: the scheduled listing is an authenticated management
    // surface, so wait for the server-confirmed session before calling the row API.
    let session = auth::use_session();

    view! {
        <Topbar title="Scheduled" sub="Posts queued for publication" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match session.reconcile.await {
                            Ok(Some(_)) => {
                                let scheduled = super::list_scheduled(
                                        None,
                                        Some(PageSize::default()),
                                    )
                                    .await;
                                view! { <ScheduledList scheduled=scheduled /> }.into_any()
                            }
                            Ok(None) => {
                                view! {
                                    <div data-test="scheduled-auth-required">
                                        <p>"You must be logged in to manage Scheduled Posts."</p>
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
            </div>
        </div>
    }
}

#[component]
fn ScheduledList(scheduled: Result<Page<UnpublishedPost>, WebError>) -> impl IntoView {
    match scheduled {
        Ok(page) => {
            if page.posts.is_empty() {
                view! {
                    <div data-test="scheduled-empty">
                        <p>"You have no Scheduled Posts."</p>
                        <p>
                            <a
                                data-test="scheduled-compose-link"
                                href="/posts/new"
                                class="j-btn is-primary"
                            >
                                "Compose a Post"
                            </a>
                        </p>
                    </div>
                }
                .into_any()
            } else {
                view! {
                    <ul class="j-draft-list" data-test="scheduled-list">
                        {page.posts.into_iter().map(render_scheduled_row).collect::<Vec<_>>()}
                    </ul>
                }
                .into_any()
            }
        }
        Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
    }
}

fn render_scheduled_row(scheduled: UnpublishedPost) -> impl IntoView {
    let DraftRowDisplay { label, .. } = super::draft_row_display(&scheduled);
    let go_live = scheduled.post.published_at.map_or_else(
        || "Scheduled time unavailable".to_owned(),
        |when| when.to_string(),
    );
    view! {
        <li data-test="scheduled-row">
            <div class="j-draft-row">
                <div class="j-draft-row-content">
                    <strong>
                        <a href=String::from(scheduled.edit_url.clone())>{label}</a>
                    </strong>
                    <span class="j-badge j-badge-scheduled">
                        "Scheduled for " <time data-test="scheduled-go-live">{go_live}</time>
                    </span>
                </div>
                <div class="j-draft-actions">
                    <a
                        class="j-btn"
                        data-test="scheduled-edit-link"
                        href=String::from(scheduled.edit_url)
                    >
                        "Edit"
                    </a>
                </div>
            </div>
        </li>
    }
}

/// The resolved drafts list: the rows, the empty-state line, or the fetch error.
///
/// A **subcomponent** rather than a plain view-returning fn, so the branch on the awaited
/// result stays measured by the thin-component guard instead of hiding in an unmeasured
/// helper (#306). It takes the already-awaited result, so [`DraftsPage`]'s `Suspense` still
/// owns the awaiting.
#[component]
fn DraftList(
    drafts: Result<Page<UnpublishedPost>, WebError>,
    publish_action: ServerAction<Publish>,
    delete_action: ServerAction<Delete>,
) -> impl IntoView {
    match drafts {
        Ok(page) => {
            if page.posts.is_empty() {
                view! { <p>"You have no drafts."</p> }.into_any()
            } else {
                view! {
                    <ul class="j-draft-list">
                        {page
                            .posts
                            .into_iter()
                            .map(|draft| render_draft_row(draft, publish_action, delete_action))
                            .collect::<Vec<_>>()}
                    </ul>
                }
                .into_any()
            }
        }
        Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
    }
}

fn render_draft_row(
    draft: UnpublishedPost,
    publish_action: ServerAction<Publish>,
    delete_action: ServerAction<Delete>,
) -> impl IntoView {
    let post_id = i64::from(draft.post.post_id);
    // Pure title + scheduled-badge-text computation (host-tested in `super::parse`);
    // only the `view!` markup stays here.
    let DraftRowDisplay {
        label,
        scheduled_badge,
    } = super::draft_row_display(&draft);
    let scheduled_badge = scheduled_badge.map(|text| {
        view! { <span class="j-badge j-badge-scheduled">{text}</span> }
    });
    view! {
        <li>
            <div class="j-draft-row">
                <div class="j-draft-row-content">
                    <strong>{label}</strong>
                    " ("
                    {draft.post.slug.to_string()}
                    ") "
                    {scheduled_badge}
                    " "
                    <a href=String::from(draft.post.permalink)>"Permalink"</a>
                </div>
                <div class="j-draft-actions">
                    <a class="j-btn" href=String::from(draft.edit_url)>
                        "Edit"
                    </a>
                    <ActionForm action=publish_action>
                        <input type="hidden" name="post_id" value=post_id />
                        <button type="submit" class="j-btn">
                            "Publish"
                        </button>
                    </ActionForm>
                    <ActionForm action=delete_action>
                        <input type="hidden" name="post_id" value=post_id />
                        <button
                            type="submit"
                            class="j-btn is-danger"
                            onclick="return confirm('Delete this draft?')"
                        >
                            "Delete"
                        </button>
                    </ActionForm>
                </div>
            </div>
        </li>
    }
}

/// Site-wide listing of posts carrying a tag, at `/tags/:tag`.
#[component]
pub fn SiteTagPage() -> impl IntoView {
    let params = use_params_map();
    // Parse the `:tag` route segment into a canonical `Tag` once, at the source
    // (ADR-0063 §4); an unparseable segment is `None`, so the fetch below is
    // skipped and the client 404s — mirroring the `PostPage` slug parse.
    // `Tag::from_str` lowercases, so the heading and the projected render coincide.
    let tag = Memo::new(move |_| params.get().get("tag").and_then(|s| s.parse::<Tag>().ok()));

    let mutate_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| mutate_version.update(|v| *v += 1));

    let initial_page = Resource::new(
        move || (tag.get(), mutate_version.get()),
        |(tag, _)| async move {
            timeline::list_by_tag(super::tag_query(tag)?, None, Some(PageSize::default())).await
        },
    );

    let state = TimelineState::default();
    // Public projector seed (#178/#179): adopt the seeded posts for a matching
    // tag so first paint shows content (the host-tested `seeded_page` guard keeps a
    // client-side nav to a different tag from adopting the initial URL's seed, #306);
    // the reactive fetch still runs.
    state.adopt_seed(super::seeded_page(
        use_context::<Option<PageSeed>>().flatten(),
        &ListingRoute::SiteTag(tag.get_untracked()),
    ));

    timeline::wire_timeline_resolve(state, initial_page);

    let on_load_more = Callback::new(move |()| {
        if let Ok(tag_value) = super::tag_query(tag.get_untracked()) {
            timeline::spawn_load_more(state, move |cursor, limit| {
                timeline::list_by_tag(tag_value, cursor, limit)
            });
        }
    });

    // The canonical tag for the heading (a newtype is not `IntoRender`), or empty
    // for an unparseable segment — the page renders a validation error anyway.
    let read_tag = move || tag.get().map(|t| t.to_string()).unwrap_or_default();

    view! {
        {move || {
            tag.get()
                .map(|tag| {
                    let surface = FeedSurface::SiteTag { tag };
                    view! { <FeedDiscovery surface=&surface /> }
                })
        }}
        <Topbar title=move || format!("#{}", read_tag()) sub="Posts on this instance" />
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            empty_text="No posts with this tag yet."
        />
    }
}

/// Per-user listing of posts carrying a tag, at `/~:username/tags/:tag`.
#[component]
pub fn UserTagPage() -> impl IntoView {
    let params = use_params_map();
    // Parse the `~username` route segment into `Username` once, at the source; an
    // invalid segment is `None` and every consumer handles the absence.
    let username = Memo::new(move |_| {
        params
            .get()
            .get("username")
            .unwrap_or_default()
            .strip_prefix('~')
            .and_then(|s| s.parse::<Username>().ok())
    });
    // Parse the `:tag` route segment into a canonical `Tag` once, at the source
    // (ADR-0063 §4); an unparseable segment is `None`, so the fetch below is
    // skipped and the client 404s — mirroring the `PostPage` slug parse.
    // `Tag::from_str` lowercases, so the heading and the projected render coincide.
    let tag = Memo::new(move |_| params.get().get("tag").and_then(|s| s.parse::<Tag>().ok()));

    let mutate_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| mutate_version.update(|v| *v += 1));

    let initial_page = Resource::new(
        move || (username.get(), tag.get(), mutate_version.get()),
        |(username, tag, _)| async move {
            let (username, tag) = super::user_tag_query(username, tag)?;
            timeline::list_by_user_and_tag(username, tag, None, Some(PageSize::default())).await
        },
    );

    let state = TimelineState::default();
    // Public projector seed (#178/#179): adopt the seeded posts for a matching
    // username+tag so first paint shows content; both halves of the match are the
    // host-tested `seeded_page` (#306), and the reactive fetch still runs.
    state.adopt_seed(super::seeded_page(
        use_context::<Option<PageSeed>>().flatten(),
        &ListingRoute::UserTag(username.get_untracked(), tag.get_untracked()),
    ));

    timeline::wire_timeline_resolve(state, initial_page);

    let on_load_more = Callback::new(move |()| {
        if let Ok((username_value, tag_value)) =
            super::user_tag_query(username.get_untracked(), tag.get_untracked())
        {
            timeline::spawn_load_more(state, move |cursor, limit| {
                timeline::list_by_user_and_tag(username_value, tag_value, cursor, limit)
            });
        }
    });

    let read_username = canonical_username_display(username);
    // The canonical tag for the heading (a newtype is not `IntoRender`), or empty
    // for an unparseable segment — the page renders a validation error anyway.
    let read_tag = move || tag.get().map(|t| t.to_string()).unwrap_or_default();

    view! {
        {move || {
            username
                .get()
                .zip(tag.get())
                .map(|(username, tag)| {
                    let surface = FeedSurface::UserTag {
                        username,
                        tag,
                    };
                    view! { <FeedDiscovery surface=&surface /> }
                })
        }}
        <Topbar
            title=move || format!("#{}", read_tag())
            sub=move || format!("Posts by ~{}", read_username())
        />
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            tag_context=Signal::derive(move || username.get().map(TagCtx::ForUser))
            empty_text="No posts with this tag yet."
        />
    }
}
