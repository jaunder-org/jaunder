//! The **posts** vertical's wasm-only UI (ADR-0070): the reactive post widgets —
//! the composer/create form and its shared body fields, the inline composer, the
//! post card/display, the audience picker, and the tag input. Declared
//! `#[cfg(target_arch = "wasm32")] mod component;` in `posts/mod.rs`, so this file
//! is wasm-only by its `mod` declaration and carries no cfg gates of its own; it
//! calls browser APIs directly. The pure, projector-coincident render twins live
//! in the host-tested [`super::render`]; the scheduled-publish datetime conversion
//! is the host-tested [`common::time::utc_instant_from_local`].

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::audiences::{list_my_audiences, AudienceSummary};
use crate::auth::current_user;
use crate::avatar::Avatar;
use crate::error::WebError;
use crate::feed_discovery::{FeedDiscovery, RsdDiscovery};
use crate::forms::Field;
use crate::media::MediaUpload;
use crate::posts::{
    default_audience_selection, draft_row_display, get_post, get_post_preview, list_drafts,
    list_posts_by_tag, list_user_posts, list_user_posts_by_tag, parse_permalink_params,
    post_audience_selection, CreatePost, CreatePostArgs, CreatePostResult, DeletePost,
    DraftRowDisplay, DraftSummary, ListPostsByTag, ListUserPosts, ListUserPostsByTag, PostResponse,
    PublishPost, PublishPostResult, TimelinePostSummary, UnpublishPost, UpdatePost, UpdatePostArgs,
    UpdatePostResult,
};
use crate::render::TagCtx as TagContext;
use crate::subscriptions::{is_subscribed_to, SubscribeTo, UnsubscribeFrom};
use crate::tags::TagSummary;
use crate::topbar::Topbar;
use common::feed::FeedSurface;
use common::ids::{AudienceId, PostId};
use common::pagination::PageSize;
use common::post_summary::PostSummary;
use common::render::PostFormat;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::time::utc_instant_from_local;
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::{AudienceBase, AudienceSelection};

/// Shared body + format fields used by all post editors.
///
/// Renders a `name="body"` textarea. When `show_seg` is true (default), also
/// renders the `.j-seg` format toggle.
#[component]
pub fn ComposerFields(
    body: RwSignal<String>,
    format: RwSignal<PostFormat>,
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
                                if format.get() == PostFormat::Markdown {
                                    "j-btn is-selected"
                                } else {
                                    "j-btn"
                                }
                            }
                            on:click=move |_| format.set(PostFormat::Markdown)
                        >
                            "Markdown"
                        </button>
                        <button
                            type="button"
                            class=move || {
                                if format.get() == PostFormat::Org {
                                    "j-btn is-selected"
                                } else {
                                    "j-btn"
                                }
                            }
                            on:click=move |_| format.set(PostFormat::Org)
                        >
                            "Org"
                        </button>
                    </div>
                }
            })}
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
        permalink: post.permalink.as_deref().unwrap_or_default(),
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
}

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
    let format = RwSignal::new(PostFormat::Markdown);
    // Optional summary: a parent-owned validated field (ADR-0065 direct-bind), so an
    // invalid excerpt disables submit and shows an error rather than erroring on POST.
    let summary_field = Field::<PostSummary>::optional();
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
            summary_field.reset();
            publish_at.set(String::new());
            tags.set(Vec::new());
        }
    });

    if compact {
        let dispatch_save = move |_| {
            create_action.dispatch(CreatePost {
                args: CreatePostArgs {
                    body: body.get().into(),
                    format: format.get(),
                    slug_override: None,
                    publish: false,
                    publish_at: utc_instant_from_local(&publish_at.get()),
                    tags: Some(tags.get().into_iter().map(|t| t.display).collect()),
                    summary: summary_field.parsed(),
                    audience: Some(audience.get()),
                },
            });
        };
        let dispatch_publish = move |_| {
            create_action.dispatch(CreatePost {
                args: CreatePostArgs {
                    body: body.get().into(),
                    format: format.get(),
                    slug_override: None,
                    publish: true,
                    publish_at: utc_instant_from_local(&publish_at.get()),
                    tags: Some(tags.get().into_iter().map(|t| t.display).collect()),
                    summary: summary_field.parsed(),
                    audience: Some(audience.get()),
                },
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
                            prop:value=summary_field.value
                            on:input=move |ev| {
                                let v = event_target_value(&ev);
                                summary_field.value.set(v.clone());
                                summary_field.error.set(summary_field.error_for(&v));
                            }
                            on:blur=move |_| summary_field.touch()
                        />
                        {move || {
                            summary_field
                                .is_touched()
                                .then(|| summary_field.error.get())
                                .flatten()
                                .map(|msg| view! { <p class="error">{msg}</p> })
                        }}
                    </div>
                    <TagInput tags=tags />
                    <div class="j-composer-toolbar">
                        <div class="j-seg">
                            <button
                                type="button"
                                class=move || {
                                    if format.get() == PostFormat::Markdown {
                                        "j-btn is-selected"
                                    } else {
                                        "j-btn"
                                    }
                                }
                                on:click=move |_| format.set(PostFormat::Markdown)
                            >
                                "Markdown"
                            </button>
                            <button
                                type="button"
                                class=move || {
                                    if format.get() == PostFormat::Org {
                                        "j-btn is-selected"
                                    } else {
                                        "j-btn"
                                    }
                                }
                                on:click=move |_| format.set(PostFormat::Org)
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
                            disabled=move || {
                                body.get().trim().is_empty() || !summary_field.is_valid()
                            }
                            on:click=dispatch_save
                        >
                            "Save draft"
                        </button>
                        <button
                            class="j-btn is-primary"
                            type="button"
                            name="publish"
                            value="true"
                            disabled=move || {
                                body.get().trim().is_empty() || !summary_field.is_valid()
                            }
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
                args: CreatePostArgs {
                    body: body.get().into(),
                    format: format.get(),
                    slug_override: slug_field.parsed(),
                    publish,
                    publish_at: utc_instant_from_local(&publish_at.get()),
                    tags: Some(tags.get().into_iter().map(|t| t.display).collect()),
                    summary: summary_field.parsed(),
                    audience: Some(audience.get()),
                },
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
                                prop:value=summary_field.value
                                on:input=move |ev| {
                                    let v = event_target_value(&ev);
                                    summary_field.value.set(v.clone());
                                    summary_field.error.set(summary_field.error_for(&v));
                                }
                                on:blur=move |_| summary_field.touch()
                            />
                            {move || {
                                summary_field
                                    .is_touched()
                                    .then(|| summary_field.error.get())
                                    .flatten()
                                    .map(|msg| view! { <p class="error">{msg}</p> })
                            }}
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
                                    if format.get() == PostFormat::Markdown {
                                        "j-btn is-selected"
                                    } else {
                                        "j-btn"
                                    }
                                }
                                on:click=move |_| format.set(PostFormat::Markdown)
                            >
                                "Markdown"
                            </button>
                            <button
                                type="button"
                                class=move || {
                                    if format.get() == PostFormat::Org {
                                        "j-btn is-selected"
                                    } else {
                                        "j-btn"
                                    }
                                }
                                on:click=move |_| format.set(PostFormat::Org)
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
                            prop:disabled=move || {
                                !slug_field.is_valid() || !summary_field.is_valid()
                            }
                            on:click=move |_| dispatch_create(false)
                        >
                            "Save draft"
                        </button>
                        <button
                            class="j-btn is-primary"
                            type="button"
                            name="publish"
                            value="true"
                            prop:disabled=move || {
                                !slug_field.is_valid() || !summary_field.is_valid()
                            }
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
            .unwrap_or_else(|| created.preview_url.clone())
            .to_string();
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

// ---------------------------------------------------------------------------
// Routed page components (moved from `pages/posts.rs`, #323).
// ---------------------------------------------------------------------------

#[component]
pub fn CreatePostPage() -> impl IntoView {
    let current_user = crate::server_resource(|| (), |()| current_user());
    let last_result: RwSignal<Option<CreatePostResult>> = RwSignal::new(None);

    view! {
        <Topbar title="New post" sub="Long-form" />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match current_user.await {
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
                                                    data-test="preview-link"
                                                    href=created.preview_url.to_string()
                                                >
                                                    "Preview draft"
                                                </a>
                                                {created
                                                    .permalink
                                                    .as_ref()
                                                    .map(|href| {
                                                        view! {
                                                            <a data-test="permalink-link" href=href.to_string()>
                                                                "View permalink"
                                                            </a>
                                                        }
                                                    })}
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
fn permalink_first_paint(seed_post: Option<PostResponse>) -> AnyView {
    match seed_post {
        Some(post) => {
            // Just the article — this fallback sits inside the reactive PostPage's
            // own `j-scroll`/`j-page`. `display:contents` keeps the host wrapper out
            // of the layout so it coincides with the projector's permalink page.
            let html = crate::posts::render::permalink_article(&post);
            view! { <div style="display:contents" inner_html=html></div> }.into_any()
        }
        None => view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any(),
    }
}

#[component]
pub fn PostPage() -> impl IntoView {
    // Public projector seed (#178/#179): the content the server painted for this
    // permalink. Adopted as the `Suspense` fallback below so first paint shows
    // real content (flash-free) instead of a spinner. The reactive fetch still
    // runs and takes over — restoring the author's edit/delete affordances when
    // the viewer owns the post — so this *enhances* rather than *replaces*.
    let seed_post = match use_context::<Option<crate::render::PageSeed>>().flatten() {
        Some(crate::render::PageSeed::Permalink(post)) => Some(post),
        _ => None,
    };

    let params = use_params_map();

    let post_data = move || {
        let params = params.get();
        // Decode the permalink route params into typed values client-side so
        // `get_post` takes a typed `Slug`/`Username` (ADR-0063 §4). The pure
        // decoder is host-tested in `super::parse`.
        parse_permalink_params(
            params.get("username").as_deref(),
            params.get("year").as_deref(),
            params.get("month").as_deref(),
            params.get("day").as_deref(),
            params.get("slug").as_deref(),
        )
    };

    let post = crate::server_resource(
        post_data,
        |(username, year, month, day, slug): (Option<Username>, i32, u32, u32, Option<Slug>)| async move {
            let Some(username) = username else {
                // This is not a post permalink segment (it didn't start with '~').
                // It may be a server-handled URL (e.g. /media/…) that the SPA
                // router matched here because it has the same number of segments.
                // Reload the page so the server can handle it properly.
                if let Some(window) = web_sys::window() {
                    if let Ok(href) = window.location().href() {
                        let _ = window.location().replace(&href);
                    }
                }
                return Err(WebError::validation("Invalid permalink"));
            };
            // A '~'-prefixed permalink with an unparseable slug is a malformed
            // permalink (not a server URL): 404 client-side without calling the
            // server, matching the pre-typing behavior where get_post rejected it.
            let Some(slug) = slug else {
                return Err(WebError::validation("Invalid permalink"));
            };
            get_post(username, year, month, day, slug).await
        },
    );

    let on_unpublish = Callback::new(move |()| {
        if let Some(window) = web_sys::window() {
            let _ = window.location().replace("/drafts");
        }
    });

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
                                    .is_draft
                                    .then_some("Draft - visible only to you".to_string());
                                let summary = TimelinePostSummary {
                                    post_id: fetched.post_id,
                                    username: fetched.username.clone(),
                                    title: fetched.title.clone(),
                                    summary: fetched.summary.clone(),
                                    slug: fetched.slug.clone(),
                                    rendered_html: fetched.rendered_html.clone(),
                                    created_at: fetched.created_at,
                                    published_at: fetched
                                        .published_at
                                        .unwrap_or(fetched.created_at),
                                    permalink: fetched.permalink.clone(),
                                    is_author: fetched.is_author,
                                    is_draft: fetched.is_draft,
                                    tags: fetched.tags.clone(),
                                };
                                let username_for_tags = fetched.username.clone();
                                view! {
                                    <PostCard
                                        post=summary
                                        banner=banner
                                        tag_context=TagContext::ForUser(username_for_tags)
                                        on_unpublish=on_unpublish
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

/// Subscribe / Unsubscribe control shown on a user's profile (timeline) page.
///
/// Hidden when the viewer is logged out or is viewing their own profile.
/// Otherwise renders Subscribe when not subscribed and Unsubscribe when
/// subscribed, querying state via `is_subscribed_to`.
#[component]
fn SubscribeButton(username: Username) -> impl IntoView {
    let subscribe = ServerAction::<SubscribeTo>::new();
    let unsubscribe = ServerAction::<UnsubscribeFrom>::new();

    // Re-query after either action mutates the subscription.
    let username_for_state = username.clone();
    let state = crate::server_resource(
        move || (subscribe.version().get(), unsubscribe.version().get()),
        move |_| {
            let username = username_for_state.clone();
            async move {
                let viewer = current_user().await.ok().flatten();
                let subscribed = is_subscribed_to(username.clone()).await.unwrap_or(false);
                (viewer, subscribed)
            }
        },
    );

    let profile_username = username;

    view! {
        <Suspense fallback=|| ()>
            {move || {
                let username = profile_username.clone();
                Suspend::new(async move {
                    let (viewer, subscribed) = state.await;
                    let show = match &viewer {
                        Some(name) => *name != username,
                        None => false,
                    };
                    if !show {
                        return ().into_any();
                    }
                    if subscribed {
                        view! {
                            <ActionForm action=unsubscribe>
                                <input
                                    type="hidden"
                                    name="author_username"
                                    value=username.to_string()
                                />
                                <button type="submit" class="j-btn">
                                    "Unsubscribe"
                                </button>
                            </ActionForm>
                        }
                            .into_any()
                    } else {
                        view! {
                            <ActionForm action=subscribe>
                                <input
                                    type="hidden"
                                    name="author_username"
                                    value=username.to_string()
                                />
                                <button type="submit" class="j-btn is-primary">
                                    "Subscribe"
                                </button>
                            </ActionForm>
                        }
                            .into_any()
                    }
                })
            }}
        </Suspense>
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
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

    let initial_page = crate::server_resource(
        move || (username.get(), mutate_version.get()),
        |(username, _)| async move {
            let username = username.ok_or_else(|| WebError::validation("Invalid username"))?;
            list_user_posts(username, None, None, Some(PageSize::default())).await
        },
    );

    let timeline = RwSignal::new(Vec::<TimelinePostSummary>::new());
    let next_cursor_created_at = RwSignal::new(None::<UtcInstant>);
    let next_cursor_post_id = RwSignal::new(None::<PostId>);
    let has_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let initial_loaded = RwSignal::new(false);

    // Public projector seed (#178/#179): if the server painted this profile,
    // adopt its posts as the initial state so first paint shows content (no
    // Loading flash). Guarded on the username so a client-side navigation to a
    // *different* profile ignores the initial URL's seed; the reactive fetch
    // still runs and takes over.
    if let Some(crate::render::PageSeed::Profile {
        username: seed_user,
        page,
    }) = use_context::<Option<crate::render::PageSeed>>().flatten()
    {
        if username.get_untracked().as_ref() == Some(&seed_user) {
            next_cursor_created_at.set(page.next_cursor_created_at);
            next_cursor_post_id.set(page.next_cursor_post_id);
            has_more.set(page.has_more);
            timeline.set(page.posts);
            initial_loaded.set(true);
        }
    }

    let load_more_action = ServerAction::<ListUserPosts>::new();

    // Client-only: `Effect::new_isomorphic` would race with SSR reactive-owner
    // disposal because the Resource future can resolve on a tokio worker after
    // the per-request owner is gone, panicking on signal access. SSR renders
    // the loading placeholder; signals seed on the client after hydration.
    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            match result {
                Ok(page) => {
                    timeline.set(page.posts);
                    next_cursor_created_at.set(page.next_cursor_created_at);
                    next_cursor_post_id.set(page.next_cursor_post_id);
                    has_more.set(page.has_more);
                    error.set(None);
                    initial_loaded.set(true);
                }
                Err(err) => {
                    error.set(Some(err.to_string()));
                    timeline.set(Vec::new());
                    has_more.set(false);
                    initial_loaded.set(true);
                }
            }
        }
    });

    // ServerAction dispatches happen only on the client, so this effect's body
    // never fires server-side; using `Effect::new` matches that reality.
    Effect::new(move |_| {
        if let Some(result) = load_more_action.value().get() {
            match result {
                Ok(page) => {
                    timeline.update(|rows| rows.extend(page.posts));
                    next_cursor_created_at.set(page.next_cursor_created_at);
                    next_cursor_post_id.set(page.next_cursor_post_id);
                    has_more.set(page.has_more);
                    error.set(None);
                }
                Err(err) => error.set(Some(err.to_string())),
            }
        }
    });

    let on_load_more = move |_| {
        let Some(username) = username.get_untracked() else {
            return;
        };
        if !has_more.get_untracked() {
            return;
        }
        load_more_action.dispatch(ListUserPosts {
            username,
            cursor_created_at: next_cursor_created_at.get_untracked(),
            cursor_post_id: next_cursor_post_id.get_untracked(),
            limit: Some(PageSize::default()),
        });
    };

    // The heading shows the canonical (parsed, lowercased) username, or empty for an
    // invalid segment — the page renders a validation error in that case anyway.
    let display_username = move || username.get().map(String::from).unwrap_or_default();
    let read_error = move || error.get();
    let read_initial_loaded = move || initial_loaded.get();
    let read_timeline = move || timeline.get();
    let read_has_more = move || has_more.get();
    let read_pending = move || load_more_action.pending().get();

    view! {
        {move || {
            username
                .get()
                .map(|username| {
                    view! {
                        <FeedDiscovery surface=FeedSurface::User {
                            username: username.clone(),
                        } />
                        <RsdDiscovery username=username />
                    }
                })
        }}
        <Topbar title=move || format!("Posts by {}", display_username()) sub="User timeline" />
        <div class="j-scroll">
            <div class="j-page">
                {move || {
                    username.get().map(|username| view! { <SubscribeButton username=username /> })
                }}
                {move || {
                    if let Some(err) = read_error() {
                        return view! { <p class="error">{err}</p> }.into_any();
                    }
                    if !read_initial_loaded() {
                        return view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any();
                    }
                    let rows = read_timeline();
                    if rows.is_empty() {
                        return view! { <p>"No posts yet."</p> }.into_any();
                    }
                    view! {
                        <div>
                            {rows
                                .into_iter()
                                .map(|post| {
                                    let username_for_tags = post.username.clone();
                                    view! {
                                        <PostCard
                                            post=post
                                            banner=None
                                            tag_context=TagContext::ForUser(username_for_tags)
                                            on_mutate=on_mutate
                                        />
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                        {move || {
                            read_has_more()
                                .then(|| {
                                    view! {
                                        <button on:click=on_load_more disabled=read_pending>
                                            {move || {
                                                if read_pending() { "Loading\u{2026}" } else { "Load more" }
                                            }}
                                        </button>
                                    }
                                })
                        }}
                    }
                        .into_any()
                }}
            </div>
        </div>
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
#[component]
pub fn EditPostPage() -> impl IntoView {
    let params = use_params_map();
    let update_post_action = ServerAction::<UpdatePost>::new();
    let body = RwSignal::new(String::new());
    let format = RwSignal::new(PostFormat::Markdown);
    let slug_field = Field::<Slug>::optional();
    // Optional summary: parent-owned validated field (ADR-0065 direct-bind), so an
    // over-cap entry disables save and an empty field submits `None` (omit → clear).
    let summary_field = Field::<PostSummary>::optional();
    // Optional scheduled-publish time for an unpublished/draft post (naive
    // local wall-clock from a `datetime-local` control); empty publishes now.
    let publish_at = RwSignal::new(String::new());
    let post_tags: RwSignal<Vec<TagSummary>> = RwSignal::new(Vec::new());
    // Pre-selected with the post's current targeting (defaults to Public until
    // it resolves).
    let audience = RwSignal::new(AudienceSelection {
        base: AudienceBase::Public,
        named: Vec::new(),
    });
    // ServerAction dispatches happen only on the client; this redirect-on-publish
    // effect only ever fires there. `Effect::new_isomorphic` would needlessly
    // schedule on the server.
    Effect::new(move |_| {
        if let Some(Ok(ref updated)) = update_post_action.value().get() {
            if updated.published_at.is_some() {
                if let Some(ref permalink) = updated.permalink {
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().replace(permalink.as_ref());
                    }
                }
            }
        }
    });

    // A missing or unparseable `post_id` is honest absence, not a real id: derive
    // `Option<PostId>` and short-circuit `None` to a client-side not-found in each
    // fetcher, rather than minting a sentinel id and paying a round-trip that only
    // ever returns not-found (#487).
    let post_id_param = move || {
        params
            .get()
            .get("post_id")
            .and_then(|v| v.parse::<PostId>().ok())
    };
    let post = crate::server_resource(post_id_param, |maybe_id| async move {
        match maybe_id {
            Some(id) => get_post_preview(id).await,
            None => Err(WebError::not_found("Post")),
        }
    });
    let current_audience = crate::server_resource(post_id_param, |maybe_id| async move {
        match maybe_id {
            Some(id) => post_audience_selection(id).await,
            None => Err(WebError::not_found("Post")),
        }
    });
    // Client-only: copying the resolved Resource into `audience` must not run
    // during SSR, where the future can resolve after the per-request reactive
    // owner is disposed (web-style-guide.md §9). The picker is seeded with the
    // post's current targeting on the client after hydration.
    Effect::new(move |_| {
        if let Some(Ok(selection)) = current_audience.get() {
            audience.set(selection);
        }
    });

    view! {
        <Topbar title="Edit Post" sub="Long-form" />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok(fetched) => {
                        body.set(String::from(fetched.body.clone()));
                        format.set(fetched.format);
                        slug_field.value.set(fetched.slug.to_string());
                        summary_field
                            .value
                            .set(fetched.summary.as_deref().unwrap_or_default().to_owned());
                        post_tags.set(fetched.tags.clone());
                        let post_id = fetched.post_id;
                        let is_published = fetched.published_at.is_some();
                        let dispatch_update = move |publish: bool| {
                            update_post_action
                                .dispatch(UpdatePost {
                                    args: UpdatePostArgs {
                                        post_id,
                                        body: body.get().into(),
                                        format: format.get(),
                                        slug_override: slug_field.parsed(),
                                        publish,
                                        publish_at: utc_instant_from_local(&publish_at.get()),
                                        tags: Some(
                                            post_tags.get().into_iter().map(|t| t.display).collect(),
                                        ),
                                        summary: summary_field.parsed(),
                                        audience: Some(audience.get()),
                                    },
                                });
                        };
                        view! {
                            <div class="j-edit-form-grid">
                                <div class="j-edit-form-body">
                                    <ComposerFields
                                        body=body
                                        format=format
                                        rows=20
                                        show_seg=false
                                    />
                                </div>
                                <aside class="j-edit-form-aside">
                                    <div>
                                        <div class="j-sb-head" style="padding:0 0 10px">
                                            "Options"
                                        </div>
                                        {(!is_published)
                                            .then(|| {
                                                view! {
                                                    <div
                                                        class="j-field-row"
                                                        style="grid-template-columns:auto 1fr"
                                                    >
                                                        <label class="j-field-label" for="edit-slug">
                                                            "Slug"
                                                        </label>
                                                        <input
                                                            id="edit-slug"
                                                            type="text"
                                                            name="slug_override"
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
                                                    // Optional schedule for a draft: a future
                                                    // time schedules it; empty publishes now.
                                                    <div style="margin-top:10px">
                                                        <label class="j-field-label" for="edit-publish-at">
                                                            "Publish at (optional)"
                                                        </label>
                                                        <input
                                                            id="edit-publish-at"
                                                            type="datetime-local"
                                                            class="j-field-val"
                                                            prop:value=publish_at
                                                            on:input=move |ev| {
                                                                publish_at.set(event_target_value(&ev));
                                                            }
                                                        />
                                                    </div>
                                                }
                                            })}
                                        <div style="margin-top:10px">
                                            <label class="j-field-label" for="edit-summary">
                                                "Summary"
                                            </label>
                                            <textarea
                                                id="edit-summary"
                                                name="summary"
                                                placeholder="Optional summary or excerpt"
                                                class="j-field-val"
                                                rows=3
                                                prop:value=summary_field.value
                                                on:input=move |ev| {
                                                    let v = event_target_value(&ev);
                                                    summary_field.value.set(v.clone());
                                                    summary_field.error.set(summary_field.error_for(&v));
                                                }
                                                on:blur=move |_| summary_field.touch()
                                            />
                                            {move || {
                                                summary_field
                                                    .is_touched()
                                                    .then(|| summary_field.error.get())
                                                    .flatten()
                                                    .map(|msg| view! { <p class="error">{msg}</p> })
                                            }}
                                        </div>
                                        <div style="margin-top:10px">
                                            <TagInput tags=post_tags />
                                        </div>
                                        <div style="margin-top:10px">
                                            <AudiencePicker selection=audience />
                                        </div>
                                        <div class="j-seg" style="margin-top:10px">
                                            <button
                                                type="button"
                                                class=move || {
                                                    if format.get() == PostFormat::Markdown {
                                                        "j-btn is-selected"
                                                    } else {
                                                        "j-btn"
                                                    }
                                                }
                                                on:click=move |_| { format.set(PostFormat::Markdown) }
                                            >
                                                "Markdown"
                                            </button>
                                            <button
                                                type="button"
                                                class=move || {
                                                    if format.get() == PostFormat::Org {
                                                        "j-btn is-selected"
                                                    } else {
                                                        "j-btn"
                                                    }
                                                }
                                                on:click=move |_| format.set(PostFormat::Org)
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
                                    <div class="j-edit-form-actions">
                                        {if is_published {
                                            view! {
                                                <button
                                                    class="j-btn is-primary"
                                                    type="button"
                                                    name="publish"
                                                    value="true"
                                                    prop:disabled=move || {
                                                        !slug_field.is_valid() || !summary_field.is_valid()
                                                    }
                                                    on:click=move |_| dispatch_update(true)
                                                >
                                                    "Save"
                                                </button>
                                            }
                                                .into_any()
                                        } else {
                                            view! {
                                                <button
                                                    class="j-btn"
                                                    type="button"
                                                    name="publish"
                                                    value="false"
                                                    prop:disabled=move || {
                                                        !slug_field.is_valid() || !summary_field.is_valid()
                                                    }
                                                    on:click=move |_| dispatch_update(false)
                                                >
                                                    "Save draft"
                                                </button>
                                                <button
                                                    class="j-btn is-primary"
                                                    type="button"
                                                    name="publish"
                                                    value="true"
                                                    prop:disabled=move || {
                                                        !slug_field.is_valid() || !summary_field.is_valid()
                                                    }
                                                    on:click=move |_| dispatch_update(true)
                                                >
                                                    "Publish"
                                                </button>
                                            }
                                                .into_any()
                                        }}
                                    </div>
                                </aside>
                            </div>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {move || {
            update_post_action
                .value()
                .get()
                .map(|result: Result<UpdatePostResult, WebError>| match result {
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
                                <a data-test="preview-link" href=updated.preview_url.to_string()>
                                    "Preview draft"
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
    let publish_action = ServerAction::<PublishPost>::new();
    let delete_action = ServerAction::<DeletePost>::new();
    let drafts = crate::server_resource(
        move || {
            (
                publish_action.version().get(),
                delete_action.version().get(),
            )
        },
        |_| list_drafts(None, None, Some(PageSize::default())),
    );

    view! {
        <Topbar title="Drafts" sub="Unpublished posts" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match drafts.await {
                            Ok(list) => {
                                if list.is_empty() {
                                    return view! { <p>"You have no drafts."</p> }.into_any();
                                }
                                view! {
                                    <ul class="j-draft-list">
                                        {list
                                            .into_iter()
                                            .map(|draft| render_draft_row(
                                                draft,
                                                publish_action,
                                                delete_action,
                                            ))
                                            .collect::<Vec<_>>()}
                                    </ul>
                                }
                                    .into_any()
                            }
                            Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                        }
                    })}
                </Suspense>
                {move || {
                    publish_action
                        .value()
                        .get()
                        .map(|result: Result<PublishPostResult, WebError>| match result {
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

fn render_draft_row(
    draft: DraftSummary,
    publish_action: ServerAction<PublishPost>,
    delete_action: ServerAction<DeletePost>,
) -> impl IntoView {
    let post_id = i64::from(draft.post_id);
    // Pure title + scheduled-badge-text computation (host-tested in `super::parse`);
    // only the `view!` markup stays here.
    let DraftRowDisplay {
        label,
        scheduled_badge,
    } = draft_row_display(&draft);
    let scheduled_badge = scheduled_badge.map(|text| {
        view! { <span class="j-badge j-badge-scheduled">{text}</span> }
    });
    view! {
        <li>
            <div class="j-draft-row">
                <div class="j-draft-row-content">
                    <strong>{label}</strong>
                    " ("
                    {draft.slug.to_string()}
                    ") "
                    {scheduled_badge}
                    " "
                    <a href=String::from(draft.preview_url)>"Preview"</a>
                    " "
                    <a href=String::from(draft.permalink)>"Permalink"</a>
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
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
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

    let initial_page = crate::server_resource(
        move || (tag.get(), mutate_version.get()),
        |(tag, _)| async move {
            let Some(tag) = tag else {
                return Err(WebError::validation("Invalid tag"));
            };
            list_posts_by_tag(tag, None, None, Some(PageSize::default())).await
        },
    );

    let timeline = RwSignal::new(Vec::<TimelinePostSummary>::new());
    let next_cursor_created_at = RwSignal::new(None::<UtcInstant>);
    let next_cursor_post_id = RwSignal::new(None::<PostId>);
    let has_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let initial_loaded = RwSignal::new(false);

    // Public projector seed (#178/#179): adopt the seeded posts for a matching
    // tag so first paint shows content (guarded so a client-side nav to a
    // different tag ignores the initial URL's seed); the reactive fetch still runs.
    if let Some(crate::render::PageSeed::SiteTag {
        tag: seed_tag,
        page,
    }) = use_context::<Option<crate::render::PageSeed>>().flatten()
    {
        if tag.get_untracked().as_ref() == Some(&seed_tag) {
            next_cursor_created_at.set(page.next_cursor_created_at);
            next_cursor_post_id.set(page.next_cursor_post_id);
            has_more.set(page.has_more);
            timeline.set(page.posts);
            initial_loaded.set(true);
        }
    }

    let load_more_action = ServerAction::<ListPostsByTag>::new();

    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            match result {
                Ok(page) => {
                    timeline.set(page.posts);
                    next_cursor_created_at.set(page.next_cursor_created_at);
                    next_cursor_post_id.set(page.next_cursor_post_id);
                    has_more.set(page.has_more);
                    error.set(None);
                    initial_loaded.set(true);
                }
                Err(err) => {
                    error.set(Some(err.to_string()));
                    timeline.set(Vec::new());
                    has_more.set(false);
                    initial_loaded.set(true);
                }
            }
        }
    });

    Effect::new(move |_| {
        if let Some(result) = load_more_action.value().get() {
            match result {
                Ok(page) => {
                    timeline.update(|rows| rows.extend(page.posts));
                    next_cursor_created_at.set(page.next_cursor_created_at);
                    next_cursor_post_id.set(page.next_cursor_post_id);
                    has_more.set(page.has_more);
                    error.set(None);
                }
                Err(err) => error.set(Some(err.to_string())),
            }
        }
    });

    let on_load_more = move |_| {
        let Some(tag_value) = tag.get_untracked() else {
            return;
        };
        if !has_more.get_untracked() {
            return;
        }
        load_more_action.dispatch(ListPostsByTag {
            tag: tag_value,
            cursor_created_at: next_cursor_created_at.get_untracked(),
            cursor_post_id: next_cursor_post_id.get_untracked(),
            limit: Some(PageSize::default()),
        });
    };

    // The canonical tag for the heading (a newtype is not `IntoRender`), or empty
    // for an unparseable segment — the page renders a validation error anyway.
    let read_tag = move || tag.get().map(|t| t.to_string()).unwrap_or_default();
    let read_error = move || error.get();
    let read_initial_loaded = move || initial_loaded.get();
    let read_timeline = move || timeline.get();
    let read_has_more = move || has_more.get();
    let read_pending = move || load_more_action.pending().get();

    view! {
        {move || {
            view! {
                {tag
                    .get()
                    .map(|tag| view! { <FeedDiscovery surface=FeedSurface::SiteTag { tag } /> })}
            }
        }}
        <Topbar
            title=Signal::derive(move || format!("#{}", read_tag()))
            sub="Posts on this instance"
        />
        <div class="j-scroll">
            <div class="j-page">
                {move || {
                    if let Some(err) = read_error() {
                        return view! { <p class="error">{err}</p> }.into_any();
                    }
                    if !read_initial_loaded() {
                        return view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any();
                    }
                    let rows = read_timeline();
                    if rows.is_empty() {
                        return view! { <p>"No posts with this tag yet."</p> }.into_any();
                    }
                    view! {
                        <div>
                            {rows
                                .into_iter()
                                .map(|post| {
                                    view! { <PostCard post=post banner=None on_mutate=on_mutate /> }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                        {move || {
                            read_has_more()
                                .then(|| {
                                    view! {
                                        <button on:click=on_load_more disabled=read_pending>
                                            {move || {
                                                if read_pending() { "Loading\u{2026}" } else { "Load more" }
                                            }}
                                        </button>
                                    }
                                })
                        }}
                    }
                        .into_any()
                }}
            </div>
        </div>
    }
}

/// Per-user listing of posts carrying a tag, at `/~:username/tags/:tag`.
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
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

    let initial_page = crate::server_resource(
        move || (username.get(), tag.get(), mutate_version.get()),
        |(username, tag, _)| async move {
            let username = username.ok_or_else(|| WebError::validation("Invalid username"))?;
            let Some(tag) = tag else {
                return Err(WebError::validation("Invalid tag"));
            };
            list_user_posts_by_tag(username, tag, None, None, Some(PageSize::default())).await
        },
    );

    let timeline = RwSignal::new(Vec::<TimelinePostSummary>::new());
    let next_cursor_created_at = RwSignal::new(None::<UtcInstant>);
    let next_cursor_post_id = RwSignal::new(None::<PostId>);
    let has_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let initial_loaded = RwSignal::new(false);

    // Public projector seed (#178/#179): adopt the seeded posts for a matching
    // username+tag so first paint shows content; the reactive fetch still runs.
    if let Some(crate::render::PageSeed::UserTag {
        username: seed_user,
        tag: seed_tag,
        page,
    }) = use_context::<Option<crate::render::PageSeed>>().flatten()
    {
        if username.get_untracked().as_ref() == Some(&seed_user)
            && tag.get_untracked().as_ref() == Some(&seed_tag)
        {
            next_cursor_created_at.set(page.next_cursor_created_at);
            next_cursor_post_id.set(page.next_cursor_post_id);
            has_more.set(page.has_more);
            timeline.set(page.posts);
            initial_loaded.set(true);
        }
    }

    let load_more_action = ServerAction::<ListUserPostsByTag>::new();

    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            match result {
                Ok(page) => {
                    timeline.set(page.posts);
                    next_cursor_created_at.set(page.next_cursor_created_at);
                    next_cursor_post_id.set(page.next_cursor_post_id);
                    has_more.set(page.has_more);
                    error.set(None);
                    initial_loaded.set(true);
                }
                Err(err) => {
                    error.set(Some(err.to_string()));
                    timeline.set(Vec::new());
                    has_more.set(false);
                    initial_loaded.set(true);
                }
            }
        }
    });

    Effect::new(move |_| {
        if let Some(result) = load_more_action.value().get() {
            match result {
                Ok(page) => {
                    timeline.update(|rows| rows.extend(page.posts));
                    next_cursor_created_at.set(page.next_cursor_created_at);
                    next_cursor_post_id.set(page.next_cursor_post_id);
                    has_more.set(page.has_more);
                    error.set(None);
                }
                Err(err) => error.set(Some(err.to_string())),
            }
        }
    });

    let on_load_more = move |_| {
        let Some(username_value) = username.get_untracked() else {
            return;
        };
        let Some(tag_value) = tag.get_untracked() else {
            return;
        };
        if !has_more.get_untracked() {
            return;
        }
        load_more_action.dispatch(ListUserPostsByTag {
            username: username_value,
            tag: tag_value,
            cursor_created_at: next_cursor_created_at.get_untracked(),
            cursor_post_id: next_cursor_post_id.get_untracked(),
            limit: Some(PageSize::default()),
        });
    };

    // Canonical (parsed, lowercased) username for the heading, or empty for an
    // invalid segment — the page renders a validation error in that case anyway.
    let read_username = move || username.get().map(String::from).unwrap_or_default();
    // The canonical tag for the heading (a newtype is not `IntoRender`), or empty
    // for an unparseable segment — the page renders a validation error anyway.
    let read_tag = move || tag.get().map(|t| t.to_string()).unwrap_or_default();
    let read_error = move || error.get();
    let read_initial_loaded = move || initial_loaded.get();
    let read_timeline = move || timeline.get();
    let read_has_more = move || has_more.get();
    let read_pending = move || load_more_action.pending().get();

    view! {
        {move || {
            view! {
                {username
                    .get()
                    .zip(tag.get())
                    .map(|(username, tag)| {
                        view! {
                            <FeedDiscovery surface=FeedSurface::UserTag {
                                username,
                                tag,
                            } />
                        }
                    })}
            }
        }}
        <Topbar
            title=Signal::derive(move || format!("#{}", read_tag()))
            sub=Signal::derive(move || format!("Posts by ~{}", read_username()))
        />
        <div class="j-scroll">
            <div class="j-page">
                {move || {
                    if let Some(err) = read_error() {
                        return view! { <p class="error">{err}</p> }.into_any();
                    }
                    if !read_initial_loaded() {
                        return view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any();
                    }
                    let rows = read_timeline();
                    if rows.is_empty() {
                        return view! { <p>"No posts with this tag yet."</p> }.into_any();
                    }
                    view! {
                        <div>
                            {rows
                                .into_iter()
                                .map(|post| {
                                    let username_for_tags = post.username.clone();
                                    view! {
                                        <PostCard
                                            post=post
                                            banner=None
                                            tag_context=TagContext::ForUser(username_for_tags)
                                            on_mutate=on_mutate
                                        />
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                        {move || {
                            read_has_more()
                                .then(|| {
                                    view! {
                                        <button on:click=on_load_more disabled=read_pending>
                                            {move || {
                                                if read_pending() { "Loading\u{2026}" } else { "Load more" }
                                            }}
                                        </button>
                                    }
                                })
                        }}
                    }
                        .into_any()
                }}
            </div>
        </div>
    }
}
