use crate::audiences::{list_my_audiences, AudienceSummary};
// `current_user` is the sidebar's background reconcile (#181), used only in the
// wasm-only correction Effect.
use crate::auth::current_user;
use crate::backup::{backup_warning_visible, current_user_is_operator};
use crate::forms::Field;
use crate::pages::upload::MediaPanel;
use crate::posts::{
    default_audience_selection, AudienceSelection, CreatePost, CreatePostResult, DeletePost,
    TimelinePostSummary, UnpublishPost,
};
use crate::tags::TagSummary;
use common::slug::Slug;
use common::tag::TagLabel;
use common::username::Username;
use leptos::prelude::*;
use leptos_router::hooks::use_location;

/// Linking context for a post's footer tag chips — re-exported from the pure
/// `render` layer (`SiteWide` / `ForUser`) so the reactive components and the
/// projector share one type. See [`crate::render::TagCtx`]. Anonymous posts get
/// their chips from the pure [`crate::render::render_tag_list`] (byte-coincident
/// with the projector, injected via `inner_html`); the authored post view — which
/// the projector never renders — uses the reactive [`TagList`] below.
pub use crate::render::TagCtx as TagContext;

/// Renders a post's tags as clickable chips for the reactive authored post view
/// (kept markup-equivalent to [`crate::render::render_tag_list`], the anonymous /
/// projector path). See [`TagContext`] for the linking behavior.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos #[component] props are stored by the framework and must be owned; \
              the borrow clippy suggests isn't expressible in a component signature"
)]
#[component]
pub fn TagList(tags: Vec<TagSummary>, context: TagContext) -> impl IntoView {
    if tags.is_empty() {
        return ().into_any();
    }
    let chips: Vec<_> = tags
        .into_iter()
        .map(|tag| {
            let slug = tag.slug.clone();
            let here = match &context {
                TagContext::ForUser(username) => {
                    let here_href = format!("/~{username}/tags/{slug}");
                    Some(view! {
                        <a class="j-tag-here" href=here_href title="On this blog">
                            "\u{00b7} here"
                        </a>
                    })
                }
                TagContext::SiteWide => None,
            };
            let chip_href = format!("/tags/{slug}");
            // TagLabel isn't IntoRender/IntoAttributeValue — stringify for the view.
            view! {
                <span class="j-tag-cell">
                    <a class="j-tag" href=chip_href>
                        "#"
                        {tag.display.to_string()}
                    </a>
                    {here}
                </span>
            }
        })
        .collect();
    view! { <span class="j-tag-list">{chips}</span> }.into_any()
}

// ─── Icons ────────────────────────────────────────────────────

/// SVG path `d` strings — re-exported from the pure `render` layer so the
/// reactive [`Icon`] component and the projector's [`crate::render::render_icon`]
/// share one source of truth.
pub use crate::render::Icons;

// ─── 3.1 Icon ─────────────────────────────────────────────────

#[component]
pub fn Icon(path: &'static str, #[prop(default = 16)] size: u32) -> impl IntoView {
    view! {
        <svg
            class="j-icon"
            width=size
            height=size
            viewBox="0 0 20 20"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
        >
            <path d=path />
        </svg>
    }
}

// ─── 3.2 Avatar ───────────────────────────────────────────────

#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos #[component] props are stored by the framework and must be owned; \
              the borrow clippy suggests isn't expressible in a component signature"
)]
#[component]
pub fn Avatar(name: String, #[prop(default = 38)] size: u32) -> impl IntoView {
    let (initials, hue) = crate::render::avatar_parts(&name);
    // Integer equivalent of `(size as f32 * 0.36).round()`; must match
    // `render::render_avatar` so SSR and reactive output coincide.
    let font_size = (size * 36 + 50) / 100;
    let style = format!(
        "width:{size}px;height:{size}px;background:oklch(0.58 0.07 {hue});font-size:{font_size}px"
    );
    view! {
        <div class="j-av" style=style>
            {initials}
        </div>
    }
}

// ─── 3.3 Dot ──────────────────────────────────────────────────

#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos #[component] props are stored by the framework and must be owned; \
              the borrow clippy suggests isn't expressible in a component signature"
)]
#[component]
pub fn Dot(proto: String) -> impl IntoView {
    let style = format!("background: var(--c-{proto})");
    view! { <span class="j-dot" style=style></span> }
}

// ─── 3.4 Chip ─────────────────────────────────────────────────

#[component]
pub fn Chip(
    label: String,
    #[prop(optional)] proto: Option<String>,
    #[prop(optional)] count: Option<u32>,
    #[prop(default = false)] active: bool,
) -> impl IntoView {
    let class = if active { "j-chip is-active" } else { "j-chip" };
    view! {
        <span class=class>
            {proto.map(|p| view! { <Dot proto=p /> })} <span>{label}</span>
            {count.map(|n| view! { <span class="j-n">{n}</span> })}
        </span>
    }
}

// ─── 3.5 Topbar ───────────────────────────────────────────────

#[component]
pub fn BackupBanner() -> impl IntoView {
    let visible = crate::server_resource(|| (), |()| backup_warning_visible());

    view! {
        <Suspense fallback=|| ()>
            {move || Suspend::new(async move {
                match visible.await {
                    Ok(true) => {
                        view! {
                            <div class="j-backup-banner" role="alert">
                                <span>"Backups are not configured. Your data is at risk."</span>
                                <div>
                                    <a href="/admin/backups">"Configure Backups"</a>
                                    <a href="/admin/site">"Site Settings"</a>
                                </div>
                            </div>
                        }
                            .into_any()
                    }
                    _ => ().into_any(),
                }
            })}
        </Suspense>
    }
}

pub use crate::ui::Topbar;

// ─── 3.6 PostCard ─────────────────────────────────────────────

// ─── ComposerFields ───────────────────────────────────────────

/// Shared body + format fields used by all post editors.
///
/// Renders a `name="body"` textarea and a `name="format"` hidden input.
/// When `show_seg` is true (default), also renders the `.j-seg` format toggle.
#[component]
pub fn ComposerFields(
    body: RwSignal<String>,
    format: RwSignal<String>,
    #[prop(default = "Write something\u{2026}")] placeholder: &'static str,
    #[prop(default = 16u32)] rows: u32,
    #[prop(default = "j-edit-form-textarea")] textarea_class: &'static str,
    /// When false, the `.j-seg` format toggle is not rendered (caller places it elsewhere).
    #[prop(default = true)]
    show_seg: bool,
    /// Optional callback fired on every body input event (e.g. to clear a flash message).
    #[prop(optional)]
    on_input: Option<Callback<()>>,
) -> impl IntoView {
    view! {
        <textarea
            name="body"
            class=textarea_class
            rows=rows
            placeholder=placeholder
            prop:value=body
            on:input=move |ev| {
                body.set(event_target_value(&ev));
                if let Some(cb) = on_input {
                    cb.run(());
                }
            }
        ></textarea>
        {show_seg
            .then(move || {
                view! {
                    <div class="j-seg">
                        <button
                            type="button"
                            class=move || {
                                if format.get() == "markdown" {
                                    "j-btn is-selected"
                                } else {
                                    "j-btn"
                                }
                            }
                            on:click=move |_| format.set("markdown".to_string())
                        >
                            "Markdown"
                        </button>
                        <button
                            type="button"
                            class=move || {
                                if format.get() == "org" { "j-btn is-selected" } else { "j-btn" }
                            }
                            on:click=move |_| format.set("org".to_string())
                        >
                            "Org"
                        </button>
                    </div>
                }
            })}
        <input type="hidden" name="format" prop:value=move || format.get() />
    }
}

/// Converts a `datetime-local` input value — a naive local wall-clock such as
/// "2026-07-01T13:30" — into a UTC RFC3339 instant string for the server.
/// Returns `None` for an empty/whitespace input (i.e. publish-now).
///
/// The browser's `Date` does the local→UTC conversion so it honors the
/// author's timezone and DST. Form dispatch is client-only, so the non-wasm
/// build only needs this to compile (the stub is never executed there).
// Deliberate manual keep: this genuine helper (not a Leptos view) benefits from
// `#[must_use]`; the crate-wide `must_use_candidate = "allow"` (Cargo.toml, #94)
// means clippy no longer flags it, so we assert it by hand.
#[must_use]
pub(crate) fn local_datetime_to_utc_rfc3339(local: &str) -> Option<String> {
    let trimmed = local.trim();
    if trimmed.is_empty() {
        return None;
    }
    // `new Date("YYYY-MM-DDTHH:MM")` (time present, no offset) is parsed as
    // local time per ECMAScript; `toISOString()` re-renders it in UTC.
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(trimmed));
    if date.get_time().is_nan() {
        return None;
    }
    date.to_iso_string().as_string()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos #[component] props are stored by the framework and must be owned; \
              the borrow clippy suggests isn't expressible in a component signature"
)]
#[component]
pub fn PostDisplay(
    post: TimelinePostSummary,
    banner: Option<String>,
    /// Linking context for the tag chips in the footer; defaults to
    /// site-wide.
    #[prop(default = TagContext::SiteWide)]
    tag_context: TagContext,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let time_label = crate::render::format_post_time(&post.published_at);
    // Built once and shared by both arms so the authored content column is the SAME
    // pure, viewer-independent render the projector paints (#181, ADR-0044 D4) — no
    // hand-rebuilt markup and no is_author-driven content change that could diverge
    // and reintroduce a flash. The action column is layered on additively.
    let view = crate::render::PostView {
        username: &post.username,
        title: post.title.as_deref(),
        banner: banner.as_deref(),
        summary: post.summary.as_deref(),
        rendered_html: &post.rendered_html,
        time: &time_label,
        permalink: &post.permalink,
        tags: &post.tags,
        tag_ctx: &tag_context,
    };
    match children {
        // Anonymous / no-action layout: the WHOLE article inner is produced by the
        // pure `render` layer — the SAME code the public projector server-renders
        // (#179) — and injected via `inner_html`, so a seeded first paint and this
        // reactive re-render are byte-identical (flash-free). "Share the pure fn,
        // not the component" (ADR-0041 §4). The projector only ever renders this
        // anonymous view, so this is the only path that must coincide.
        None => {
            let inner = crate::render::render_post_inner(&view);
            view! { <article class="j-post" inner_html=inner></article> }.into_any()
        }
        // Authored layout (own posts, with the action column). The content column is
        // the SAME `render_post_content` the anonymous arm wraps, injected via
        // `inner_html` so it coincides with the projector's paint (#181); only the
        // reactive action column (`children`, carrying edit/delete handlers that
        // `inner_html` can't) overlays it as a sibling. This replaces the previously
        // hand-rebuilt reactive header/title/body markup, which had diverged from
        // the projector — the divergence that kept the authored path from coinciding.
        Some(children) => {
            let inner_content = crate::render::render_post_content(&view);
            view! {
                <article class="j-post">
                    <Avatar name=post.username.to_string() size=38 />
                    <div style="min-width:0;display:flex;gap:8px;align-items:flex-start">
                        <div style="flex:1;min-width:0" inner_html=inner_content></div>
                        {children()}
                    </div>
                </article>
            }
            .into_any()
        }
    }
}

/// `true` when the auth marker's username equals `author` (#181, ADR-0044): the
/// client-side signal that the viewer owns this post, so its action column shows
/// even though the anonymous seed data has `is_author = false`. `false` on the
/// host build (no marker) — the affordance is wasm-only chrome.
fn marker_matches(author: &str) -> bool {
    crate::auth::marker::read().as_deref() == Some(author)
}

#[component]
pub fn PostCard(
    post: TimelinePostSummary,
    banner: Option<String>,
    /// Linking context for the footer tag chips; defaults to site-wide.
    #[prop(default = TagContext::SiteWide)]
    tag_context: TagContext,
    #[prop(optional)] on_mutate: Option<Callback<()>>,
    #[prop(optional)] on_unpublish: Option<Callback<()>>,
) -> impl IntoView {
    // The seed/anonymous data has `is_author = false` (the projector paints
    // anonymous-only), so on the Local timeline the owner's own posts would show no
    // action column. Decide it client-side from the auth marker (#181, ADR-0044 D4)
    // so the affordance appears synchronously at mount. The server still authorizes
    // the actual edit/delete by session — the marker only gates visibility.
    let is_author = post.is_author || marker_matches(&post.username);
    let post_id = post.post_id;
    let edit_url = format!("/posts/{post_id}/edit");
    let delete_action = ServerAction::<DeletePost>::new();
    let unpublish_action = ServerAction::<UnpublishPost>::new();
    let deleted = RwSignal::new(false);

    Effect::new_isomorphic(move |_| {
        if let Some(Ok(())) = delete_action.value().get() {
            deleted.set(true);
            if let Some(cb) = on_mutate {
                cb.run(());
            }
        }
    });
    Effect::new_isomorphic(move |_| {
        if let Some(Ok(())) = unpublish_action.value().get() {
            let cb = on_unpublish.or(on_mutate);
            if let Some(cb) = cb {
                cb.run(());
            }
        }
    });

    // Additive action column (#181, ADR-0044 D4): edit/unpublish/delete only. The
    // timestamp deliberately stays in the (coincident) content-column header rather
    // than moving here, so the owner's own post doesn't diverge from the anon paint.
    let action_col = is_author.then(move || {
        view! {
            <div class="j-post-acts">
                <a class="j-btn" href=edit_url>
                    "Edit"
                </a>
                <button
                    type="button"
                    class="j-btn"
                    on:click=move |_| {
                        unpublish_action.dispatch(UnpublishPost { post_id });
                    }
                >
                    "Unpublish"
                </button>
                <button
                    type="button"
                    class="j-btn is-danger"
                    on:click=move |_| {
                        let confirmed = web_sys::window()
                            .and_then(|w| { w.confirm_with_message("Delete this post?").ok() })
                            .unwrap_or(false);
                        if confirmed {
                            delete_action.dispatch(DeletePost { post_id });
                        }
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

// ─── 3.6b AudiencePicker ──────────────────────────────────────

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
    // SSR-resolved Resources serialize their value to the client and are not
    // re-fetched on hydration; if `list_my_audiences` lost the disposal race and
    // resolved to `Err` during SSR, the client would reuse that `Err` and the
    // multiselect would stay empty. So resolve it client-only: SSR renders no
    // checkboxes, and a wasm-only Effect seeds them after hydration
    // (web-style-guide.md §9, mirroring `home.rs`).
    let named = crate::server_resource(|| (), |()| list_my_audiences());
    let named_audiences = RwSignal::new(Vec::<AudienceSummary>::new());
    Effect::new(move |_| {
        if let Some(Ok(list)) = named.get() {
            named_audiences.set(list);
        }
    });

    let base_options = ["public", "subscribers", "private"];
    let base_labels = ["Public", "Subscribers", "Private (only me)"];

    view! {
        <div class="j-field-row" style="grid-template-columns:auto 1fr">
            <label class="j-field-label" for="audience-base">
                "Audience"
            </label>
            <select
                id="audience-base"
                class="j-field-val"
                on:change=move |ev| {
                    selection
                        .update(|sel| {
                            sel.base = event_target_value(&ev);
                        });
                }
            >
                {base_options
                    .iter()
                    .zip(base_labels)
                    .map(|(value, label)| {
                        let value = (*value).to_string();
                        view! {
                            <option
                                value=value.clone()
                                selected=move || selection.get().base == value
                            >
                                {label}
                            </option>
                        }
                    })
                    .collect_view()}
            </select>
        </div>
        {move || {
            let audiences = named_audiences.get();
            if audiences.is_empty() {
                ().into_any()
            } else {
                let rows = audiences
                    .into_iter()
                    .map(|a| audience_checkbox(a, selection))
                    .collect_view();
                view! {
                    <div style="margin-top:8px">
                        <span class="j-field-label">"Also share with"</span>
                        {rows}
                    </div>
                }
                    .into_any()
            }
        }}
    }
}

/// One named-audience checkbox row for [`AudiencePicker`]. Toggling it
/// adds/removes the audience id in the shared selection. Disabled while the
/// base is `Private`, since Private cannot combine with named audiences.
// cov:ignore-start
fn audience_checkbox(
    audience: AudienceSummary,
    selection: RwSignal<AudienceSelection>,
) -> impl IntoView {
    let id = audience.audience_id;
    let input_id = format!("audience-named-{id}");
    let checked = move || selection.get().named.contains(&id);
    let disabled = move || selection.get().base == "private";
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
            {audience.name}
        </label>
    }
    // cov:ignore-stop
} // cov:ignore

// ─── 3.7 PostCreateForm ───────────────────────────────────────

#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
#[component]
pub fn PostCreateForm(
    compact: bool,
    #[prop(optional)] username: Option<Username>,
    #[prop(into)] on_success: Callback<CreatePostResult>,
    #[prop(default = 6)] rows: u32,
    #[prop(default = "What\u{2019}s on your mind?")] placeholder: &'static str,
    /// Called on every textarea input event (compact mode only).
    #[prop(optional)]
    on_input: Option<Callback<()>>,
) -> impl IntoView {
    let create_action = ServerAction::<CreatePost>::new();
    let body = RwSignal::new(String::new());
    let format = RwSignal::new("markdown".to_string());
    let summary = RwSignal::new(String::new());
    // Optional scheduled-publish time (naive local wall-clock from a
    // `datetime-local` control); empty = publish now / draft. Only the
    // non-compact form renders the control; the compact composer leaves it
    // empty (publish-now).
    let publish_at = RwSignal::new(String::new());
    let tags: RwSignal<Vec<TagSummary>> = RwSignal::new(Vec::new());
    // A new post starts at the site-wide default audience; default to Public
    // until that resolves.
    let audience = RwSignal::new(AudienceSelection {
        base: "public".to_string(),
        named: Vec::new(),
    });
    let default_audience = crate::server_resource(|| (), |()| default_audience_selection());
    // Client-only: copying the resolved Resource into `audience` must not run
    // during SSR, where the future can resolve after the per-request reactive
    // owner is disposed (web-style-guide.md §9). SSR renders the Public
    // default; the real default is seeded on the client after hydration.
    Effect::new(move |_| {
        if let Some(Ok(default)) = default_audience.get() {
            audience.set(default);
        }
    });

    Effect::new(move |_| {
        if let Some(Ok(ref created)) = create_action.value().get() {
            let created = created.clone();
            on_success.run(created);
            body.set(String::new());
            summary.set(String::new());
            publish_at.set(String::new());
            tags.set(Vec::new());
        }
    });

    if compact {
        let dispatch_save = move |_| {
            create_action.dispatch(CreatePost {
                body: body.get().into(),
                format: format.get(),
                slug_override: None,
                publish: false,
                publish_at: local_datetime_to_utc_rfc3339(&publish_at.get()),
                tags: Some(tags.get().into_iter().map(|t| t.display).collect()),
                summary: Some(summary.get()),
                audience: Some(audience.get()),
            });
        };
        let dispatch_publish = move |_| {
            create_action.dispatch(CreatePost {
                body: body.get().into(),
                format: format.get(),
                slug_override: None,
                publish: true,
                publish_at: local_datetime_to_utc_rfc3339(&publish_at.get()),
                tags: Some(tags.get().into_iter().map(|t| t.display).collect()),
                summary: Some(summary.get()),
                audience: Some(audience.get()),
            });
        };
        view! {
            <div class="j-composer-row">
                <Avatar name=username.map(String::from).unwrap_or_default() size=36 />
                <div class="j-composer-body">
                    <ComposerFields
                        body=body
                        format=format
                        rows=rows
                        placeholder=placeholder
                        textarea_class=""
                        show_seg=false
                        on_input=on_input.unwrap_or_else(|| Callback::new(move |()| {}))
                    />
                    <MediaPanel />
                    <div style="margin-top:10px">
                        <label class="j-field-label">"Summary"</label>
                        <textarea
                            id="compose-summary"
                            class="j-field-val"
                            rows=3
                            placeholder="Optional summary or excerpt"
                            prop:value=summary
                            on:input=move |ev| {
                                summary.set(event_target_value(&ev));
                            }
                        />
                    </div>
                    <TagInput tags=tags />
                    <div class="j-composer-toolbar">
                        <div class="j-seg">
                            <button
                                type="button"
                                class=move || {
                                    if format.get() == "markdown" {
                                        "j-btn is-selected"
                                    } else {
                                        "j-btn"
                                    }
                                }
                                on:click=move |_| format.set("markdown".to_string())
                            >
                                "Markdown"
                            </button>
                            <button
                                type="button"
                                class=move || {
                                    if format.get() == "org" {
                                        "j-btn is-selected"
                                    } else {
                                        "j-btn"
                                    }
                                }
                                on:click=move |_| format.set("org".to_string())
                            >
                                "Org"
                            </button>
                        </div>
                        <span class="j-spacer"></span>
                        <button
                            class="j-btn"
                            type="button"
                            name="publish"
                            value="false"
                            disabled=move || body.get().trim().is_empty()
                            on:click=dispatch_save
                        >
                            "Save draft"
                        </button>
                        <button
                            class="j-btn is-primary"
                            type="button"
                            name="publish"
                            value="true"
                            disabled=move || body.get().trim().is_empty()
                            on:click=dispatch_publish
                        >
                            "Publish"
                        </button>
                    </div>
                </div>
            </div>
            {move || {
                create_action
                    .value()
                    .get()
                    .and_then(Result::err)
                    .map(|e| view! { <p class="error">{e.to_string()}</p> })
            }}
        }
        .into_any()
    } else {
        let slug_field = Field::<Slug>::optional();
        let dispatch_create = move |publish: bool| {
            create_action.dispatch(CreatePost {
                body: body.get().into(),
                format: format.get(),
                slug_override: slug_field.parsed(),
                publish,
                publish_at: local_datetime_to_utc_rfc3339(&publish_at.get()),
                tags: Some(tags.get().into_iter().map(|t| t.display).collect()),
                summary: Some(summary.get()),
                audience: Some(audience.get()),
            });
        };
        view! {
            <div class="j-compose-grid">
                <div class="j-compose-body">
                    <ComposerFields
                        body=body
                        format=format
                        rows=rows
                        placeholder=placeholder
                        show_seg=false
                    />
                </div>
                <aside class="j-compose-aside">
                    <div>
                        <div class="j-sb-head" style="padding:0 0 10px">
                            "Options"
                        </div>
                        <div class="j-field-row" style="grid-template-columns:auto 1fr">
                            <label class="j-field-label" for="compose-slug">
                                "Slug"
                            </label>
                            <input
                                id="compose-slug"
                                type="text"
                                name="slug_override"
                                placeholder="auto"
                                class="j-field-val"
                                prop:value=slug_field.value
                                on:input=move |ev| {
                                    let v = event_target_value(&ev);
                                    slug_field.value.set(v.clone());
                                    slug_field.error.set(slug_field.error_for(&v));
                                }
                                on:blur=move |_| slug_field.touch()
                            />
                            {move || {
                                slug_field
                                    .is_touched()
                                    .then(|| slug_field.error.get())
                                    .flatten()
                                    .map(|msg| view! { <p class="error">{msg}</p> })
                            }}
                        </div>
                        <div style="margin-top:10px">
                            <label class="j-field-label" for="compose-summary">
                                "Summary"
                            </label>
                            <textarea
                                id="compose-summary"
                                name="summary"
                                placeholder="Optional summary or excerpt"
                                class="j-field-val"
                                rows=3
                                prop:value=summary
                                on:input=move |ev| {
                                    summary.set(event_target_value(&ev));
                                }
                            />
                        </div>
                        <div style="margin-top:10px">
                            <TagInput tags=tags />
                        </div>
                        <div style="margin-top:10px">
                            <AudiencePicker selection=audience />
                        </div>
                        // Optional schedule: a future time schedules the post;
                        // a past time backdates it; empty publishes immediately.
                        <div style="margin-top:10px">
                            <label class="j-field-label" for="compose-publish-at">
                                "Publish at (optional)"
                            </label>
                            <input
                                id="compose-publish-at"
                                type="datetime-local"
                                class="j-field-val"
                                prop:value=publish_at
                                on:input=move |ev| publish_at.set(event_target_value(&ev))
                            />
                        </div>
                        <div class="j-seg" style="margin-top:10px">
                            <button
                                type="button"
                                class=move || {
                                    if format.get() == "markdown" {
                                        "j-btn is-selected"
                                    } else {
                                        "j-btn"
                                    }
                                }
                                on:click=move |_| format.set("markdown".to_string())
                            >
                                "Markdown"
                            </button>
                            <button
                                type="button"
                                class=move || {
                                    if format.get() == "org" {
                                        "j-btn is-selected"
                                    } else {
                                        "j-btn"
                                    }
                                }
                                on:click=move |_| format.set("org".to_string())
                            >
                                "Org"
                            </button>
                        </div>
                    </div>
                    <div style="margin-top:16px">
                        <div class="j-sb-head" style="padding:0 0 10px">
                            "Media"
                        </div>
                        <MediaPanel />
                    </div>
                    <div style="margin-top:auto;display:flex;align-items:center;gap:8px">
                        <button
                            class="j-btn"
                            type="button"
                            name="publish"
                            value="false"
                            prop:disabled=move || !slug_field.is_valid()
                            on:click=move |_| dispatch_create(false)
                        >
                            "Save draft"
                        </button>
                        <button
                            class="j-btn is-primary"
                            type="button"
                            name="publish"
                            value="true"
                            prop:disabled=move || !slug_field.is_valid()
                            on:click=move |_| dispatch_create(true)
                        >
                            "Publish"
                        </button>
                    </div>
                </aside>
            </div>
            {move || {
                create_action
                    .value()
                    .get()
                    .and_then(Result::err)
                    .map(|e| view! { <p class="error">{e.to_string()}</p> })
            }}
        }
        .into_any()
    }
}

// ─── 3.8 InlineComposer ───────────────────────────────────────

#[component]
pub fn InlineComposer(username: Username, on_publish: WriteSignal<u32>) -> impl IntoView {
    let flash: RwSignal<Option<(String, String)>> = RwSignal::new(None);

    let on_success = Callback::new(move |created: CreatePostResult| {
        use leptos_dom::helpers::set_timeout;
        use std::time::Duration;
        let url = created
            .permalink
            .clone()
            .unwrap_or_else(|| created.preview_url.clone());
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

// ─── 3.8 Sidebar ──────────────────────────────────────────────

/// A single nav item in the sidebar.
#[component]
fn SidebarNavItem(
    label: &'static str,
    icon_path: &'static str,
    active: bool,
    href: Option<&'static str>,
) -> impl IntoView {
    let class = if active {
        "j-nav-item is-active"
    } else {
        "j-nav-item"
    };
    let inner = view! {
        <Icon path=icon_path size=16 />
        <span>{label}</span>
    };
    match href {
        Some(href) => view! {
            <a class=class href=href>
                {inner}
            </a>
        }
        .into_any(),
        None => view! { <div class=class>{inner}</div> }.into_any(),
    }
}

/// A static source row in the sidebar sources section.
#[component]
fn SidebarSource(proto: &'static str, name: &'static str, sub: &'static str) -> impl IntoView {
    let dot_style = format!("width:8px;height:8px;border-radius:4px;background:var(--c-{proto})");
    view! {
        <div class="j-source">
            <span class="j-dot" style=dot_style></span>
            <div style="flex:1;min-width:0">
                <div class="j-source-name">{name}</div>
                <div class="j-source-sub">{sub}</div>
            </div>
        </div>
    }
}

/// The left navigation sidebar. Reads theme and current-user from context.
/// `active`: the key of the currently active nav item (e.g. `"home"`).
#[component]
pub fn Sidebar(#[prop(optional)] active: Option<String>) -> impl IntoView {
    let active_key = active.unwrap_or_default();

    let location = use_location();
    let operator = crate::server_resource(
        move || location.pathname.get(),
        |_| current_user_is_operator(),
    );

    // Synchronous boot source (#181, ADR-0044): the auth marker decides authed vs.
    // anon at mount, so there is NO async <Suspense> swap on first paint. The
    // anonymous sidebar is the pure `render::render_sidebar` (the SAME code the
    // projector server-renders) injected via `inner_html`, so a seeded first paint
    // and the reactive re-render coincide (flash-free). `display:contents` keeps
    // the host wrapper out of the aside's layout.
    let owner = RwSignal::new(marker_username_on_boot());

    // Background reconcile / correctness backstop (D3): confirm the marker against
    // the real session and correct a stale one without gating first paint — a dead
    // session clears the marker (toward anon, the safe direction); a live session
    // with a missing marker sets it. wasm-only: the marker lives in localStorage.
    let reconcile = crate::server_resource(move || location.pathname.get(), |_| current_user());
    Effect::new(move |_| {
        if let Some(res) = reconcile.get() {
            match res {
                Ok(Some(u)) => {
                    let u = u.to_string();
                    crate::auth::marker::set(&u);
                    if owner.get_untracked().as_deref() != Some(u.as_str()) {
                        owner.set(Some(u));
                    }
                }
                Ok(None) => {
                    crate::auth::marker::clear();
                    if owner.get_untracked().is_some() {
                        owner.set(None);
                    }
                }
                Err(_) => {}
            }
        }
    });

    let anon_html = crate::render::render_sidebar(&active_key);
    view! {
        <aside class="j-sidebar">
            {move || match owner.get() {
                None => {
                    view! { <div style="display:contents" inner_html=anon_html.clone()></div> }
                        .into_any()
                }
                Some(username) => {
                    authed_sidebar(&active_key, &username, matches!(operator.get(), Some(Ok(true))))
                        .into_any()
                }
            }}
        </aside>
    }
}

/// Boot-time marker read: `Some(username)` in the browser when the auth marker is
/// set, `None` on the host build (the sidebar only ever renders in wasm). Lets the
/// sidebar pick authed vs. anon synchronously at mount (#181), no async gate.
fn marker_username_on_boot() -> Option<String> {
    crate::auth::marker::read()
}

/// The authenticated sidebar chrome (brand, search, nav + operator admin links,
/// sources, footer avatar). Shared by the marker-seeded initial render and the
/// reconciled render (#181) so both are byte-for-byte the same authed markup —
/// only its inputs change from awaited values to these params.
// cov:ignore-start
fn authed_sidebar(active_key: &str, username: &str, is_operator: bool) -> impl IntoView {
    let active_key = active_key.to_string();
    let username = username.to_string();
    view! {
        <div style="display:contents">
            <a class="j-brand" href="/" style="text-decoration:none;color:inherit">
                <div class="j-brand-mark">"j"</div>
                <div class="j-brand-text">"Jaunder"</div>
            </a>
            <div class="j-search">
                <Icon path=Icons::SEARCH size=14 />
                <span>"Search"</span>
                <span class="j-kbd">"⌘K"</span>
            </div>
            <nav class="j-nav">
                {crate::render::NAV_ITEMS
                    .iter()
                    .filter(|&&(_, _, _, href, _)| href.is_some())
                    .map(|&(key, label, icon_path, href, _)| {
                        let is_active = key == active_key.as_str();
                        view! {
                            <SidebarNavItem
                                label=label
                                icon_path=icon_path
                                active=is_active
                                href=href
                            />
                        }
                    })
                    .collect::<Vec<_>>()}
                {if is_operator {
                    view! {
                        <SidebarNavItem
                            label="Configure Backups"
                            icon_path=Icons::SHIELD
                            active=active_key == "admin-backups"
                            href=Some("/admin/backups")
                        />
                        <SidebarNavItem
                            label="Site Settings"
                            icon_path=Icons::SHIELD
                            active=active_key == "admin-site"
                            href=Some("/admin/site")
                        />
                    }
                        .into_any()
                } else {
                    ().into_any()
                }}
            </nav>
            <div>
                <div class="j-sb-head">
                    <span>"Sources"</span>
                    <span class="j-sb-add">"+"</span>
                </div>
                {crate::render::SIDEBAR_SOURCES
                    .iter()
                    .map(|&(proto, name, sub)| {
                        view! { <SidebarSource proto=proto name=name sub=sub /> }
                    })
                    .collect::<Vec<_>>()}
            </div>
            <div class="j-sb-foot">
                <Avatar name=username.clone() size=28 />
                <div style="font-size:13px;flex:1;min-width:0">
                    <div style="font-weight:500">{username}</div>
                </div>
                <a href="/logout" style="font-size:11px;color:var(--muted)">
                    "Sign out"
                </a>
            </div>
        </div>
    }
    // cov:ignore-stop
} // cov:ignore

// ─── 3.9 TagInput ─────────────────────────────────────────────

/// Chip-based tag input with debounced autocomplete.
///
/// Renders each tag in `tags` as a removable chip and emits one
/// `<input type="hidden" name=name value=display>` per chip so an enclosing
/// form receives a `Vec<String>`.
///
/// Key bindings: `Enter`/`Tab` commit a chip from the text field; `Backspace`
/// on an empty field removes the last chip; `ArrowUp`/`ArrowDown` navigate
/// the autocomplete dropdown; `Escape` closes it.
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
#[component]
pub fn TagInput(
    tags: RwSignal<Vec<TagSummary>>,
    #[prop(default = "tags")] name: &'static str,
) -> impl IntoView {
    let input_text = RwSignal::new(String::new());
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let suggestions: RwSignal<Vec<TagSummary>> = RwSignal::new(Vec::new());
    let suggestions_open = RwSignal::new(false);
    let selected_idx: RwSignal<Option<usize>> = RwSignal::new(None);
    // Tick counter for debounce: increment on each keystroke; the timeout
    // callback only fires if the tick hasn't changed.
    let debounce_tick = RwSignal::new(0u64);

    let on_input = move |ev: leptos::ev::Event| {
        use leptos::task::spawn_local;
        use leptos_dom::helpers::set_timeout;
        use std::time::Duration;

        let val = event_target_value(&ev);
        input_text.set(val.clone());
        error.set(None);
        selected_idx.set(None);

        let prefix = val.trim().to_lowercase();
        if prefix.is_empty() {
            suggestions.set(Vec::new());
            suggestions_open.set(false);
            return;
        }

        let tick = debounce_tick.get_untracked() + 1;
        debounce_tick.set(tick);

        set_timeout(
            move || {
                if debounce_tick.get_untracked() != tick {
                    return;
                }
                spawn_local(async move {
                    if let Ok(results) = crate::tags::list_tags(Some(prefix), Some(10)).await {
                        if debounce_tick.get_untracked() == tick {
                            let open = !results.is_empty();
                            suggestions.set(results);
                            suggestions_open.set(open);
                        }
                    }
                });
            },
            Duration::from_millis(150),
        );
    };

    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        let key = ev.key();
        match key.as_str() {
            "Enter" | "Tab" => {
                // If a suggestion is keyboard-selected, commit it.
                if let Some(i) = selected_idx.get() {
                    if let Some(tag) = suggestions.get().get(i).cloned() {
                        ev.prevent_default();
                        tags.update(|t| {
                            if !t.iter().any(|x| x.slug == tag.slug) {
                                t.push(tag.clone());
                            }
                        });
                        input_text.set(String::new());
                        error.set(None);
                        suggestions.set(Vec::new());
                        suggestions_open.set(false);
                        selected_idx.set(None);
                        return;
                    }
                }
                // Commit the typed text; Tab passes through if the field is empty.
                if input_text.get().trim().is_empty() {
                    return;
                }
                ev.prevent_default();
                // Validate the raw input via `TagLabel::from_str` (the single
                // validity source, shared with the server) — trims and validates
                // without lowercasing, so the author's casing is preserved
                // (Decision 4). Dedup is on the canonical slug.
                match input_text.get().parse::<TagLabel>() {
                    Ok(label) => {
                        let slug = label.slug();
                        tags.update(|t| {
                            if !t.iter().any(|x| x.slug == slug) {
                                t.push(TagSummary {
                                    slug,
                                    display: label,
                                });
                            }
                        });
                        input_text.set(String::new());
                        error.set(None);
                        suggestions.set(Vec::new());
                        suggestions_open.set(false);
                        selected_idx.set(None);
                    }
                    Err(e) => error.set(Some(e.to_string())),
                }
            }
            "Backspace" if input_text.get().is_empty() => {
                tags.update(|t| {
                    t.pop();
                });
            }
            "ArrowDown" => {
                ev.prevent_default();
                let len = suggestions.get().len();
                if len > 0 {
                    selected_idx.update(|i| {
                        *i = Some(i.map_or(0, |n| (n + 1).min(len - 1)));
                    });
                }
            }
            "ArrowUp" => {
                ev.prevent_default();
                selected_idx.update(|i| {
                    *i = i.and_then(|n| n.checked_sub(1));
                });
            }
            "Escape" => {
                suggestions.set(Vec::new());
                suggestions_open.set(false);
                selected_idx.set(None);
            }
            _ => {}
        }
    };

    view! {
        <div class="j-tag-input">
            {move || {
                tags.get()
                    .into_iter()
                    .map(|tag| {
                        let slug = tag.slug.clone();
                        let display = tag.display.to_string();
                        view! {
                            <span class="j-tag-chip">
                                <input type="hidden" name=name value=display.clone() />
                                <span class="j-tag-chip-label">"#" {display}</span>
                                <button
                                    type="button"
                                    class="j-tag-chip-remove"
                                    aria-label="Remove tag"
                                    on:click=move |_| {
                                        tags.update(|t| t.retain(|x| x.slug != slug));
                                    }
                                >
                                    "\u{00d7}"
                                </button>
                            </span>
                        }
                    })
                    .collect::<Vec<_>>()
            }}
            <input
                type="text"
                class="j-tag-text"
                placeholder="Add tag\u{2026}"
                prop:value=input_text
                on:input=on_input
                on:keydown=on_keydown
                autocomplete="off"
            />
            {move || {
                if !suggestions_open.get() {
                    return ().into_any();
                }
                let items = suggestions
                    .get()
                    .into_iter()
                    .enumerate()
                    .map(|(idx, tag)| {
                        let is_active = selected_idx.get() == Some(idx);
                        let slug = tag.slug.clone();
                        let display = tag.display.clone();
                        view! {
                            <li
                                class=if is_active {
                                    "j-tag-suggest-item is-active"
                                } else {
                                    "j-tag-suggest-item"
                                }
                                on:click=move |_| {
                                    let slug = slug.clone();
                                    let display = display.clone();
                                    tags.update(|t| {
                                        if !t.iter().any(|x| x.slug == slug) {
                                            t.push(TagSummary {
                                                slug: slug.clone(),
                                                display: display.clone(),
                                            });
                                        }
                                    });
                                    input_text.set(String::new());
                                    error.set(None);
                                    suggestions.set(Vec::new());
                                    suggestions_open.set(false);
                                    selected_idx.set(None);
                                }
                            >
                                "#"
                                {tag.display.to_string()}
                            </li>
                        }
                    })
                    .collect::<Vec<_>>();
                view! { <ul class="j-tag-suggest">{items}</ul> }.into_any()
            }}
        </div>
        {move || error.get().map(|e| view! { <p class="j-tag-error">{e}</p> })}
    }
}
