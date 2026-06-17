use crate::auth::current_user;
use crate::backup::{backup_warning_visible, current_user_is_operator};
use crate::pages::upload::MediaPanel;
use crate::posts::{CreatePost, CreatePostResult, DeletePost, TimelinePostSummary, UnpublishPost};
use crate::tags::TagSummary;
use leptos::prelude::*;
use leptos_router::hooks::use_location;

/// Linking context for a [`TagList`] rendering.
///
/// `SiteWide` links each chip to `/tags/:slug` only. `ForUser` adds a small
/// "· here" link next to each chip pointing at `/~:username/tags/:slug`, so
/// per-user tag listings stay one click away from any user-rooted page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagContext {
    SiteWide,
    ForUser(String),
}

/// Renders a post's tags as clickable chips for use inside a post-display
/// footer. See [`TagContext`] for the linking behavior.
#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::must_use_candidate)]
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
            view! {
                <span class="j-tag-cell">
                    <a class="j-tag" href=chip_href>
                        "#"
                        {tag.display}
                    </a>
                    {here}
                </span>
            }
        })
        .collect();
    view! { <span class="j-tag-list">{chips}</span> }.into_any()
}

// ─── Icons ────────────────────────────────────────────────────

/// SVG path `d` attribute strings for all Jaunder icons.
pub struct Icons;

impl Icons {
    pub const HOME: &'static str = "M3 10l7-6 7 6v7a1 1 0 0 1-1 1h-4v-5H8v5H4a1 1 0 0 1-1-1z";
    pub const LOCAL: &'static str = "M4 5h12v10H4z M4 9h12";
    pub const FED: &'static str =
        "M10 3a7 7 0 1 0 0 14a7 7 0 0 0 0-14zM3 10h14 M10 3c2 3 2 11 0 14 M10 3c-2 3-2 11 0 14";
    pub const REPLY: &'static str = "M4 4h12v9H7l-3 3z";
    pub const BOOKMARK: &'static str = "M5 3h10v14l-5-3-5 3z";
    pub const BOOST: &'static str =
        "M5 8l4-4 4 4 M4 7v4a3 3 0 0 0 3 3h9 M15 12l-4 4-4-4 M16 13V9a3 3 0 0 0-3-3H4";
    pub const HEART: &'static str =
        "M10 17s-7-4.5-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 17 7c0 5.5-7 10-7 10z";
    pub const SEARCH: &'static str = "M8 3a6 6 0 1 0 0 12a6 6 0 0 0 0-12z M17 17l-4-4";
    pub const PLUS: &'static str = "M10 4v12 M4 10h12";
    pub const COG: &'static str = "M10 6v2 M10 12v2 M6 10H4 M16 10h-2 M6.5 6.5l-1.5-1.5 M14 14l1.5 1.5 M6.5 13.5L5 15 M14 6l1.5-1.5 M10 13a3 3 0 1 0 0-6a3 3 0 0 0 0 6z";
    pub const EDIT: &'static str = "M3 17l4 0 9-9a2.83 2.83 0 0 0-4-4l-9 9 0 4 M12 5l3 3";
    pub const SHIELD: &'static str = "M10 3l6 2v4c0 4-2.4 7.1-6 8-3.6-.9-6-4-6-8V5l6-2z";
    pub const MEDIA: &'static str =
        "M3 5h14v10H3z M7 9a1 1 0 1 0 0-2 1 1 0 0 0 0 2z M5 13l3-3 2 2 3-3 5 5H3z";
}

// ─── 3.1 Icon ─────────────────────────────────────────────────

#[allow(clippy::must_use_candidate)]
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

/// Derives `(initials, hue)` from a display name.
/// `initials`: first character of each of the first two whitespace-separated words, uppercased.
/// `hue`: sum of all char codes mod 360.
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss)]
#[must_use]
pub fn avatar_parts(name: &str) -> (String, u32) {
    let initials: String = name
        .split_whitespace()
        .take(2)
        .filter_map(|word| word.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .collect();

    let hue: u32 = name.chars().fold(0u32, |acc, c| acc + c as u32) % 360;

    (initials, hue)
}

#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::must_use_candidate)]
#[component]
pub fn Avatar(name: String, #[prop(default = 38)] size: u32) -> impl IntoView {
    let (initials, hue) = avatar_parts(&name);
    #[allow(clippy::cast_precision_loss)]
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::cast_sign_loss)]
    let font_size = (size as f32 * 0.36).round() as u32;
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

#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::must_use_candidate)]
#[component]
pub fn Dot(proto: String) -> impl IntoView {
    let style = format!("background: var(--c-{proto})");
    view! { <span class="j-dot" style=style></span> }
}

// ─── 3.4 Chip ─────────────────────────────────────────────────

#[allow(clippy::must_use_candidate)]
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

#[allow(clippy::must_use_candidate)]
#[component]
pub fn BackupBanner() -> impl IntoView {
    let visible = Resource::new(|| (), |()| backup_warning_visible());

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

#[allow(clippy::must_use_candidate)]
#[component]
pub fn Topbar(
    #[prop(into)] title: Signal<String>,
    #[prop(optional, into)] sub: Option<Signal<String>>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="j-topbar">
            <div>
                <h1>{move || title.get()}</h1>
                {sub
                    .map(|s| {
                        view! { <div class="j-sub">{move || s.get()}</div> }
                    })}
            </div>
            <div class="j-topbar-right">{children.map(|c| c())}</div>
        </div>
    }
}

// ─── 3.6 PostCard ─────────────────────────────────────────────

// ─── ComposerFields ───────────────────────────────────────────

/// Shared body + format fields used by all post editors.
///
/// Renders a `name="body"` textarea and a `name="format"` hidden input.
/// When `show_seg` is true (default), also renders the `.j-seg` format toggle.
#[allow(clippy::must_use_candidate)]
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

/// Formats an RFC-3339 timestamp as `"YYYY-MM-DD HH:MM"`.
/// Falls back to the raw string if the input contains no `T` separator.
pub(crate) fn format_post_time(ts: &str) -> String {
    // RFC-3339: "YYYY-MM-DDTHH:MM:SS+HH:MM" or "YYYY-MM-DDTHH:MM:SSZ"
    // Return "YYYY-MM-DD HH:MM"; fall back to the raw string if malformed.
    if let Some(t_pos) = ts.find('T') {
        let date = &ts[..t_pos];
        let rest = &ts[t_pos + 1..];
        let time = if rest.len() >= 5 { &rest[..5] } else { rest };
        format!("{date} {time}")
    } else {
        ts.to_owned()
    }
}

#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::must_use_candidate)]
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
    let time_label = format_post_time(&post.published_at);
    let is_author = post.is_author;
    let post_tags = post.tags.clone();

    view! {
        <article class="j-post">
            <Avatar name=post.username.clone() size=38 />
            <div style="min-width:0;display:flex;gap:8px;align-items:flex-start">
                <div style="flex:1;min-width:0">
                    <header class="j-post-head">
                        <span class="j-post-name">{post.username.clone()}</span>
                        <span class="j-post-handle">"@"{post.username.clone()}</span>
                        {(!is_author)
                            .then(|| {
                                view! {
                                    <>
                                        <span class="j-spacer"></span>
                                        <span class="j-post-time">{time_label}</span>
                                    </>
                                }
                            })}
                    </header>
                    {post
                        .title
                        .clone()
                        .map(|title| {
                            if post.permalink.is_empty() {
                                view! { <div class="j-post-title">{title}</div> }.into_any()
                            } else {
                                view! {
                                    <div class="j-post-title">
                                        <a href=post.permalink.clone()>{title}</a>
                                    </div>
                                }
                                    .into_any()
                            }
                        })}
                    {banner.map(|b| view! { <p class="draft-banner">{b}</p> })}
                    {post.summary.clone().map(|s| view! { <p class="j-post-summary">{s}</p> })}
                    <div class="j-post-body" inner_html=post.rendered_html.clone()></div>
                    <footer class="j-post-foot">
                        <TagList tags=post_tags context=tag_context />
                        <span class="j-spacer"></span>
                    </footer>
                </div>
                {children.map(|c| c())}
            </div>
        </article>
    }
}

#[allow(clippy::must_use_candidate)]
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
    let is_author = post.is_author;
    let post_id = post.post_id;
    let time_label = format_post_time(&post.published_at);
    let permalink = post.permalink.clone();
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

    let action_col = is_author.then(move || {
        view! {
            <div class="j-post-acts">
                <a class="j-post-plink" href=permalink>
                    {time_label}
                </a>
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
                        let confirmed = {
                            #[cfg(target_arch = "wasm32")]
                            {
                                web_sys::window()
                                    .and_then(|w| {
                                        w.confirm_with_message("Delete this post?").ok()
                                    })
                                    .unwrap_or(false)
                            }
                            #[cfg(not(target_arch = "wasm32"))] { false }
                        };
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

// ─── 3.7 PostCreateForm ───────────────────────────────────────

#[allow(clippy::too_many_lines)]
#[allow(clippy::must_use_candidate)]
#[component]
pub fn PostCreateForm(
    compact: bool,
    #[prop(optional)] username: Option<String>,
    #[prop(into)] on_success: Callback<CreatePostResult>,
    #[prop(default = 6)] rows: usize,
    #[prop(default = "What\u{2019}s on your mind?")] placeholder: &'static str,
    /// Called on every textarea input event (compact mode only).
    #[prop(optional)]
    on_input: Option<Callback<()>>,
) -> impl IntoView {
    let create_action = ServerAction::<CreatePost>::new();
    let body = RwSignal::new(String::new());
    let format = RwSignal::new("markdown".to_string());
    let summary = RwSignal::new(String::new());
    let tags: RwSignal<Vec<TagSummary>> = RwSignal::new(Vec::new());

    #[cfg(target_arch = "wasm32")]
    {
        Effect::new(move |_| {
            if let Some(Ok(ref created)) = create_action.value().get() {
                let created = created.clone();
                on_success.run(created);
                body.set(String::new());
                summary.set(String::new());
                tags.set(Vec::new());
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = on_success;

    #[allow(clippy::cast_possible_truncation)]
    let rows_u32 = rows as u32;

    if compact {
        let dispatch_save = move |_| {
            create_action.dispatch(CreatePost {
                body: body.get(),
                format: format.get(),
                slug_override: None,
                publish: false,
                tags: Some(tags.get().into_iter().map(|t| t.display).collect()),
                summary: Some(summary.get()),
            });
        };
        let dispatch_publish = move |_| {
            create_action.dispatch(CreatePost {
                body: body.get(),
                format: format.get(),
                slug_override: None,
                publish: true,
                tags: Some(tags.get().into_iter().map(|t| t.display).collect()),
                summary: Some(summary.get()),
            });
        };
        view! {
            <div class="j-composer-row">
                <Avatar name=username.unwrap_or_default() size=36 />
                <div class="j-composer-body">
                    <ComposerFields
                        body=body
                        format=format
                        rows=rows_u32
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
        let slug_override = RwSignal::new(String::new());
        let dispatch_create = move |publish: bool| {
            let slug = slug_override.get();
            let slug_override = common::text::non_empty(&slug).map(str::to_owned);
            create_action.dispatch(CreatePost {
                body: body.get(),
                format: format.get(),
                slug_override,
                publish,
                tags: Some(tags.get().into_iter().map(|t| t.display).collect()),
                summary: Some(summary.get()),
            });
        };
        view! {
            <div class="j-compose-grid">
                <div class="j-compose-body">
                    <ComposerFields
                        body=body
                        format=format
                        rows=rows_u32
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
                                prop:value=slug_override
                                on:input=move |ev| slug_override.set(event_target_value(&ev))
                            />
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
                            on:click=move |_| dispatch_create(false)
                        >
                            "Save draft"
                        </button>
                        <button
                            class="j-btn is-primary"
                            type="button"
                            name="publish"
                            value="true"
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

#[allow(clippy::must_use_candidate)]
#[component]
pub fn InlineComposer(username: String, on_publish: WriteSignal<u32>) -> impl IntoView {
    let flash: RwSignal<Option<(String, String)>> = RwSignal::new(None);

    #[cfg(not(target_arch = "wasm32"))]
    let _ = on_publish;

    let on_success = Callback::new(move |created: CreatePostResult| {
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
        #[cfg(target_arch = "wasm32")]
        {
            use leptos_dom::helpers::set_timeout;
            use std::time::Duration;
            set_timeout(move || flash.set(None), Duration::from_secs(30));
            if created.published_at.is_some() {
                on_publish.update(|v| *v += 1);
            }
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
#[allow(clippy::too_many_lines)]
#[allow(clippy::must_use_candidate)]
#[component]
pub fn Sidebar(#[prop(optional)] active: Option<String>) -> impl IntoView {
    let active_key = active.unwrap_or_default();

    let location = use_location();
    let user = Resource::new(move || location.pathname.get(), |_| current_user());
    let operator = Resource::new(
        move || location.pathname.get(),
        |_| current_user_is_operator(),
    );

    // (key, label, icon_path, href, auth_required)
    #[allow(clippy::items_after_statements)]
    const NAV_ITEMS: &[(&str, &str, &str, Option<&'static str>, bool)] = &[
        ("home", "Home", Icons::HOME, Some("/"), false),
        ("local", "Local", Icons::LOCAL, None, true),
        ("federated", "Federated", Icons::FED, None, true),
        ("replies", "Replies", Icons::REPLY, None, true),
        ("bookmarks", "Bookmarks", Icons::BOOKMARK, None, true),
        ("drafts", "Drafts", Icons::EDIT, Some("/drafts"), true),
        ("media", "Media", Icons::MEDIA, Some("/media"), true),
        ("settings", "Settings", Icons::COG, None, true),
    ];

    // Items shown when unauthenticated: has a real href and no auth required.
    let public_nav: Vec<_> = NAV_ITEMS
        .iter()
        .filter(|&&(_, _, _, href, auth_required)| href.is_some() && !auth_required)
        .copied()
        .collect();

    // Clone active_key for the fallback closure; the original moves into the Suspend closure.
    let active_key_fallback = active_key.clone();

    view! {
        <aside class="j-sidebar">
            <a class="j-brand" href="/" style="text-decoration:none;color:inherit">
                <div class="j-brand-mark">"j"</div>
                <div class="j-brand-text">"Jaunder"</div>
            </a>
            <div class="j-search">
                <Icon path=Icons::SEARCH size=14 />
                <span>"Search"</span>
                <span class="j-kbd">"⌘K"</span>
            </div>
            // Nav: filtered by auth state after Suspense resolves.
            <Suspense fallback=move || {
                view! {
                    <nav class="j-nav">
                        {public_nav
                            .iter()
                            .map(|&(key, label, icon_path, href, _)| {
                                let is_active = key == active_key_fallback.as_str();
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
                    </nav>
                }
            }>
                {move || {
                    let active_key = active_key.clone();
                    Suspend::new(async move {
                        let is_authed = matches!(user.await, Ok(Some(_)));
                        let is_operator = matches!(operator.await, Ok(true));
                        view! {
                            <nav class="j-nav">
                                {NAV_ITEMS
                                    .iter()
                                    .filter(|&&(_, _, _, href, auth_required)| {
                                        href.is_some() && (!auth_required || is_authed)
                                    })
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
                        }
                    })
                }}
            </Suspense>
            <div>
                <div class="j-sb-head">
                    <span>"Sources"</span>
                    <span class="j-sb-add">"+"</span>
                </div>
                <SidebarSource proto="atproto" name="Bluesky" sub="mara.bsky.social" />
                <SidebarSource proto="activitypub" name="Mastodon" sub="@mara@hachyderm.io" />
                <SidebarSource proto="rss" name="Ivy Chen" sub="weeknotes" />
                <SidebarSource proto="jsonfeed" name="Manton" sub="manton.org" />
            </div>
            // Footer: avatar+sign-out when authed; nothing when not.
            <div class="j-sb-foot">
                <Suspense fallback=|| ()>
                    {move || Suspend::new(async move {
                        match user.await {
                            Ok(Some(username)) => {
                                view! {
                                    <Avatar name=username.clone() size=28 />
                                    <div style="font-size:13px;flex:1;min-width:0">
                                        <div style="font-weight:500">{username}</div>
                                    </div>
                                    <a href="/logout" style="font-size:11px;color:var(--muted)">
                                        "Sign out"
                                    </a>
                                }
                                    .into_any()
                            }
                            _ => ().into_any(),
                        }
                    })}
                </Suspense>
            </div>
        </aside>
    }
}

// ─── Pure helpers for TagInput ────────────────────────────────

/// Returns `true` when `s` is a valid tag slug: non-empty, first char
/// `[a-z0-9]`, remaining chars `[a-z0-9-]`.  The input must already be
/// lowercased (call [`normalize_tag_token`] first).
///
/// Mirrors [`common::tag::Tag::from_str`] so client and server agree on
/// validity without importing `common` into the WASM bundle.
#[must_use]
pub fn is_valid_tag_slug(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        None => false,
        Some(c) if !c.is_ascii_lowercase() && !c.is_ascii_digit() => false,
        _ => chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
    }
}

/// Trims whitespace from `raw` and lowercases the result.
#[must_use]
pub fn normalize_tag_token(raw: &str) -> String {
    raw.trim().to_lowercase()
}

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
#[allow(clippy::too_many_lines)]
#[allow(clippy::must_use_candidate)]
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
    #[cfg(target_arch = "wasm32")]
    let debounce_tick = RwSignal::new(0u64);

    let on_input = move |ev: leptos::ev::Event| {
        let val = event_target_value(&ev);
        input_text.set(val.clone());
        error.set(None);
        selected_idx.set(None);

        #[cfg(target_arch = "wasm32")]
        {
            use leptos::task::spawn_local;
            use leptos_dom::helpers::set_timeout;
            use std::time::Duration;

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
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = val;
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
                let text = normalize_tag_token(&input_text.get());
                if text.is_empty() {
                    return;
                }
                ev.prevent_default();
                if is_valid_tag_slug(&text) {
                    let slug = text.clone();
                    tags.update(|t| {
                        if !t.iter().any(|x| x.slug == slug) {
                            t.push(TagSummary {
                                slug: slug.clone(),
                                display: slug,
                            });
                        }
                    });
                    input_text.set(String::new());
                    error.set(None);
                    suggestions.set(Vec::new());
                    suggestions_open.set(false);
                    selected_idx.set(None);
                } else {
                    error.set(Some(format!("Invalid tag \"{text}\"")));
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
                        let display = tag.display.clone();
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
                                {tag.display}
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

// ─── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{avatar_parts, format_post_time, is_valid_tag_slug, normalize_tag_token};

    #[test]
    fn avatar_parts_single_word() {
        let (initials, _hue) = avatar_parts("Mara");
        assert_eq!(initials, "M");
    }

    #[test]
    fn avatar_parts_two_words() {
        let (initials, _hue) = avatar_parts("Mara Ek");
        assert_eq!(initials, "ME");
    }

    #[test]
    fn avatar_parts_more_than_two_words_uses_first_two() {
        let (initials, _hue) = avatar_parts("Mara Jane Ek");
        assert_eq!(initials, "MJ");
    }

    #[test]
    fn avatar_parts_empty_name() {
        let (initials, hue) = avatar_parts("");
        assert_eq!(initials, "");
        assert_eq!(hue, 0);
    }

    #[test]
    fn avatar_parts_hue_is_in_range() {
        let (_initials, hue) = avatar_parts("Some User");
        assert!(hue < 360);
    }

    #[test]
    fn avatar_parts_hue_is_deterministic() {
        let (_, h1) = avatar_parts("Mara Ek");
        let (_, h2) = avatar_parts("Mara Ek");
        assert_eq!(h1, h2);
    }

    #[test]
    fn avatar_parts_hue_differs_for_different_names() {
        let (_, h1) = avatar_parts("Alice");
        let (_, h2) = avatar_parts("Bob");
        assert_ne!(h1, h2);
    }

    #[test]
    fn format_post_time_includes_time_portion() {
        assert_eq!(
            format_post_time("2026-04-23T10:30:00+00:00"),
            "2026-04-23 10:30"
        );
    }

    #[test]
    fn format_post_time_handles_date_only_input() {
        // Input with no 'T' separator — return as-is.
        assert_eq!(format_post_time("2026-04-23"), "2026-04-23");
    }

    #[test]
    fn format_post_time_handles_negative_offset() {
        assert_eq!(
            format_post_time("2026-04-23T15:45:00-05:00"),
            "2026-04-23 15:45"
        );
    }

    #[test]
    fn format_post_time_handles_utc_z_suffix() {
        assert_eq!(format_post_time("2026-04-23T10:30:00Z"), "2026-04-23 10:30");
    }

    // ─── is_valid_tag_slug ────────────────────────────────────

    #[test]
    fn tag_slug_accepts_lowercase_alpha() {
        assert!(is_valid_tag_slug("rust"));
    }

    #[test]
    fn tag_slug_accepts_leading_digit() {
        assert!(is_valid_tag_slug("42things"));
    }

    #[test]
    fn tag_slug_accepts_hyphens_in_body() {
        assert!(is_valid_tag_slug("hello-world"));
    }

    #[test]
    fn tag_slug_accepts_single_char() {
        assert!(is_valid_tag_slug("a"));
        assert!(is_valid_tag_slug("0"));
    }

    #[test]
    fn tag_slug_rejects_empty() {
        assert!(!is_valid_tag_slug(""));
    }

    #[test]
    fn tag_slug_rejects_leading_hyphen() {
        assert!(!is_valid_tag_slug("-hello"));
    }

    #[test]
    fn tag_slug_rejects_uppercase() {
        assert!(!is_valid_tag_slug("Rust"));
        assert!(!is_valid_tag_slug("RUST"));
    }

    #[test]
    fn tag_slug_rejects_spaces() {
        assert!(!is_valid_tag_slug("hello world"));
    }

    #[test]
    fn tag_slug_rejects_special_chars() {
        assert!(!is_valid_tag_slug("tag@site"));
        assert!(!is_valid_tag_slug("tag_name"));
    }

    // ─── normalize_tag_token ──────────────────────────────────

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(normalize_tag_token("  rust  "), "rust");
    }

    #[test]
    fn normalize_lowercases() {
        assert_eq!(normalize_tag_token("Rust"), "rust");
        assert_eq!(normalize_tag_token("HELLO-WORLD"), "hello-world");
    }

    #[test]
    fn normalize_empty_stays_empty() {
        assert_eq!(normalize_tag_token(""), "");
        assert_eq!(normalize_tag_token("   "), "");
    }

    #[test]
    fn normalize_then_validate_roundtrip() {
        let normalized = normalize_tag_token("  Hello-World  ");
        assert!(is_valid_tag_slug(&normalized));
        assert_eq!(normalized, "hello-world");
    }
}
