use crate::auth::current_user;
use crate::posts::TimelinePostSummary;
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
    pub const COG: &'static str =
        "M10 6v2 M10 12v2 M6 10H4 M16 10h-2 M6.5 6.5l-1.5-1.5 M14 14l1.5 1.5 M6.5 13.5L5 15 M14 6l1.5-1.5 M10 13a3 3 0 1 0 0-6a3 3 0 0 0 0 6z";
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

/// Formats an ISO-8601 timestamp into a short relative display string.
/// Returns a trimmed date portion as a fallback for older posts.
fn format_post_time(published_at: &str) -> String {
    // The timestamp is an RFC-3339 string like "2026-04-22T10:30:00+00:00".
    // For now, return just the date portion (YYYY-MM-DD) as a readable label.
    // A full relative-time implementation can replace this in a later step.
    published_at
        .split('T')
        .next()
        .unwrap_or(published_at)
        .to_owned()
}

#[component]
pub fn PostCard(post: TimelinePostSummary) -> impl IntoView {
    // TimelinePostSummary has no protocol, handle, or stats fields — this is
    // real app data. We render what we have and omit the source indicator and
    // stats footer for now (wired up in a later step).
    let time_label = format_post_time(&post.published_at);
    let has_title = !post.title.is_empty();

    view! {
        <article class="j-post">
            <Avatar name=post.username.clone() size=38 />
            <div style="min-width:0">
                <header class="j-post-head">
                    <span class="j-post-name">{post.username.clone()}</span>
                    <span class="j-post-handle">"@"{post.username.clone()}</span>
                    <span class="j-spacer"></span>
                    <span class="j-post-time">{time_label}</span>
                </header>
                {has_title.then(|| view! { <div class="j-post-title">{post.title.clone()}</div> })}
                <div class="j-post-body" inner_html=post.rendered_html.clone()></div>
                <footer class="j-post-foot">
                    <span class="j-spacer"></span>
                </footer>
            </div>
        </article>
    }
}

// ─── 3.7 InlineComposer ───────────────────────────────────────

/// Compact draft row at the top of the timeline.
/// Static placeholder — live data binding is wired up in a later step.
#[component]
pub fn InlineComposer() -> impl IntoView {
    view! {
        <div class="j-composer">
            <div class="j-composer-row">
                <Avatar name="".to_string() size=36 />
                <div class="j-composer-body">
                    <div></div>
                    <div class="j-composer-toolbar">
                        <span class="j-tag">
                            <Icon path=Icons::PLUS size=12 />
                            "Media"
                        </span>
                        <span class="j-tag is-accent">
                            "Cross-posting " <Icon path="M2 7l5 5 5-5" size=10 />
                        </span>
                        <button class="j-btn is-primary">"Publish"</button>
                    </div>
                </div>
            </div>
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

    // Fetch the current user. Key the resource off the current pathname so it
    // re-fetches after client-side navigations (e.g. login → home, logout → home),
    // keeping the sidebar footer in sync with auth state without a full page reload.
    let location = use_location();
    let user = Resource::new(move || location.pathname.get(), |_| current_user());

    // (key, label, icon_path, href)
    let nav_items: &[(&str, &str, &str, Option<&'static str>)] = &[
        ("home", "Home", Icons::HOME, Some("/")),
        ("local", "Local", Icons::LOCAL, None),
        ("federated", "Federated", Icons::FED, None),
        ("replies", "Replies", Icons::REPLY, None),
        ("bookmarks", "Bookmarks", Icons::BOOKMARK, None),
        ("settings", "Settings", Icons::COG, None),
    ];

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
            <nav class="j-nav">
                {nav_items
                    .iter()
                    .map(|&(key, label, icon_path, href)| {
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
            <div>
                <div class="j-sb-head">
                    <span>"Sources"</span>
                    <span class="j-sb-add">"+"</span>
                </div>
                // Static placeholder sources — replaced with real data in a later step.
                <SidebarSource proto="atproto" name="Bluesky" sub="mara.bsky.social" />
                <SidebarSource proto="activitypub" name="Mastodon" sub="@mara@hachyderm.io" />
                <SidebarSource proto="rss" name="Ivy Chen" sub="weeknotes" />
                <SidebarSource proto="jsonfeed" name="Manton" sub="manton.org" />
            </div>
            <div class="j-sb-foot">
                <Suspense fallback=|| {
                    view! {
                        <a href="/login" class="j-btn is-primary" style="width:100%">
                            "Sign in"
                        </a>
                    }
                }>
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
                            _ => {
                                view! {
                                    <a href="/login" class="j-btn is-primary" style="width:100%">
                                        "Sign in"
                                    </a>
                                }
                                    .into_any()
                            }
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
    use super::avatar_parts;

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
}
