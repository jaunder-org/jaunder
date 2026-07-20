//! The **posts** vertical's wasm-only UI (ADR-0070): the reactive post widgets —
//! the composer/create form and its shared body fields, the inline composer, the
//! post card/display, the audience picker, and the tag input. Declared
//! `#[cfg(target_arch = "wasm32")] mod component;` in `posts/mod.rs`, so this file
//! is wasm-only by its `mod` declaration and carries no cfg gates of its own; it
//! calls browser APIs directly. The pure, projector-coincident render twins live
//! in the host-tested [`super::render`]; the scheduled-publish datetime helper is
//! reached (transitionally) via [`crate::pages::ui::publish_at_from_local`].

use leptos::prelude::*;

use crate::audiences::{list_my_audiences, AudienceSummary};
use crate::avatar::Avatar;
use crate::forms::Field;
use crate::media::MediaUpload;
use crate::pages::ui::publish_at_from_local;
use crate::posts::{
    default_audience_selection, AudienceSelection, CreatePost, CreatePostResult, DeletePost,
    TimelinePostSummary, UnpublishPost,
};
use crate::render::TagCtx as TagContext;
use crate::tags::TagSummary;
use common::ids::AudienceId;
use common::slug::Slug;
use common::tag::TagLabel;
use common::username::Username;
use common::visibility::AudienceBase;

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
    let time_label = crate::posts::render::format_post_time(post.published_at);
    // Built once and shared by both arms so the authored content column is the SAME
    // pure, viewer-independent render the projector paints (#181, ADR-0044 D4) — no
    // hand-rebuilt markup and no is_author-driven content change that could diverge
    // and reintroduce a flash. The action column is layered on additively.
    let view = crate::posts::render::PostView {
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
            let inner = crate::posts::render::render_post_inner(&view);
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
            let inner_content = crate::posts::render::render_post_content(&view);
            view! {
                <article class="j-post">
                    <Avatar name=post.username.clone() size=38 />
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
fn marker_matches(author: &Username) -> bool {
    crate::auth::marker_storage::get().as_ref() == Some(author)
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

    // Each base variant paired with its UI caption in one place, so the two
    // can't drift out of order.
    let base_options = [
        (AudienceBase::Public, "Public"),
        (AudienceBase::Subscribers, "Subscribers"),
        (AudienceBase::Private, "Private (only me)"),
    ];

    view! {
        <div class="j-field-row" style="grid-template-columns:auto 1fr">
            <label class="j-field-label" for="audience-base">
                "Audience"
            </label>
            <select
                id="audience-base"
                class="j-field-val"
                on:change=move |ev| {
                    if let Ok(base) = AudienceBase::try_from(event_target_value(&ev).as_str()) {
                        selection.update(|sel| sel.base = base);
                    }
                }
            >
                {base_options
                    .into_iter()
                    .map(|(base, label)| {
                        view! {
                            <option
                                value=base.as_str()
                                selected=move || selection.get().base == base
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
    // `AudienceSummary.audience_id` is a bare `i64` (the reactive-store carve-out); wrap it
    // into the `AudienceId` that `AudienceSelection.named` holds.
    let id = AudienceId::from(audience.audience_id);
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
            {audience.name}
        </label>
    }
    // cov:ignore-stop
} // cov:ignore

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
        base: AudienceBase::Public,
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
                publish_at: publish_at_from_local(&publish_at.get()),
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
                publish_at: publish_at_from_local(&publish_at.get()),
                tags: Some(tags.get().into_iter().map(|t| t.display).collect()),
                summary: Some(summary.get()),
                audience: Some(audience.get()),
            });
        };
        view! {
            <div class="j-composer-row">
                {username.map(|u| view! { <Avatar name=u size=36 /> })}
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
                    <MediaUpload show_result=true />
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
                publish_at: publish_at_from_local(&publish_at.get()),
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
                        <MediaUpload show_result=true />
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
