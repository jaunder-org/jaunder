use crate::auth::current_user;
use crate::posts::{CreatePost, DeletePost, TimelinePostSummary, UnpublishPost};
use leptos::prelude::*;
use leptos_router::hooks::use_location;

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
}

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

/// Derives `(initials, hue)` from a display name.
/// `initials`: first character of each of the first two whitespace-separated words, uppercased.
/// `hue`: sum of all char codes mod 360.
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

#[component]
pub fn Avatar(name: String, #[prop(default = 38)] size: u32) -> impl IntoView {
    let (initials, hue) = avatar_parts(&name);
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
pub fn Topbar(
    title: String,
    #[prop(optional)] sub: Option<String>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    view! {
        <div class="j-topbar">
            <div>
                <h1>{title}</h1>
                {sub.map(|s| view! { <div class="j-sub">{s}</div> })}
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

#[component]
pub fn PostDisplay(
    post: TimelinePostSummary,
    banner: Option<String>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let time_label = format_post_time(&post.published_at);
    let is_author = post.is_author;

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
                    <div class="j-post-body" inner_html=post.rendered_html.clone()></div>
                    <footer class="j-post-foot">
                        <span class="j-spacer"></span>
                    </footer>
                </div>
                {children.map(|c| c())}
            </div>
        </article>
    }
}

#[component]
pub fn PostCard(
    post: TimelinePostSummary,
    banner: Option<String>,
    #[prop(optional)] on_mutate: Option<Callback<()>>,
    #[prop(optional)] on_unpublish: Option<Callback<()>>,
) -> impl IntoView {
    let is_author = post.is_author;
    let post_id = post.post_id;
    let time_label = format_post_time(&post.published_at);
    let permalink = post.permalink.clone();
    let edit_url = format!("/posts/{}/edit", post_id);
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
                    class="j-btn"
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
        <PostDisplay post=post banner=banner>
            {action_col}
        </PostDisplay>
    }
}

// ─── 3.7 InlineComposer ───────────────────────────────────────

#[component]
pub fn InlineComposer(username: String, on_publish: WriteSignal<u32>) -> impl IntoView {
    let create_action = ServerAction::<CreatePost>::new();
    let body = RwSignal::new(String::new());
    let format = RwSignal::new("markdown".to_string());
    let flash: RwSignal<Option<(String, String)>> = RwSignal::new(None);

    // After any successful action: clear body, set flash, optionally notify parent.
    #[cfg(target_arch = "wasm32")]
    {
        use leptos_dom::helpers::set_timeout;
        use std::time::Duration;
        Effect::new(move |_| {
            if let Some(Ok(ref created)) = create_action.value().get() {
                body.set(String::new());
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
            }
        });
    }

    // Suppress unused-variable warnings in SSR builds.
    #[cfg(not(target_arch = "wasm32"))]
    let _ = on_publish;

    view! {
        <div class="j-composer">
            <ActionForm action=create_action>
                <div class="j-composer-row">
                    <Avatar name=username.clone() size=36 />
                    <div class="j-composer-body">
                        <ComposerFields
                            body=body
                            format=format
                            rows=6
                            placeholder="What's on your mind?"
                            textarea_class=""
                            on_input=Callback::new(move |_| flash.set(None))
                        />
                        <input type="hidden" name="slug_override" value="" />
                        <div class="j-composer-toolbar">
                            <button
                                class="j-btn"
                                type="submit"
                                name="publish"
                                value="false"
                                disabled=move || body.get().trim().is_empty()
                            >
                                "Save draft"
                            </button>
                            <button
                                class="j-btn"
                                type="submit"
                                name="publish"
                                value="true"
                                disabled=move || body.get().trim().is_empty()
                            >
                                "Publish"
                            </button>
                        </div>
                    </div>
                </div>
            </ActionForm>
            {move || {
                if let Some(e) = create_action.value().get().and_then(|r| r.err()) {
                    return view! { <p class="error">{e.to_string()}</p> }.into_any();
                }
                if let Some((url, msg)) = flash.get() {
                    return view! {
                        <p class="success">
                            <a href=url>{msg}</a>
                        </p>
                    }
                        .into_any();
                }
                ().into_any()
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
    let user = Resource::new(move || location.pathname.get(), |_| current_user());

    // (key, label, icon_path, href, auth_required)
    const NAV_ITEMS: &[(&str, &str, &str, Option<&'static str>, bool)] = &[
        ("home", "Home", Icons::HOME, Some("/"), false),
        ("local", "Local", Icons::LOCAL, None, true),
        ("federated", "Federated", Icons::FED, None, true),
        ("replies", "Replies", Icons::REPLY, None, true),
        ("bookmarks", "Bookmarks", Icons::BOOKMARK, None, true),
        ("drafts", "Drafts", Icons::EDIT, Some("/drafts"), true),
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

// ─── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{avatar_parts, format_post_time};

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
}
