//! Pure, non-reactive HTML rendering for the public discoverability surface.
//!
//! Shared by the server-side projector (`server::projector`) and the CSR client
//! (`web::pages`): both call the SAME function on the SAME data, so the
//! projector's server-painted content and the client's first paint coincide
//! byte-for-byte (flash-free). There is deliberately NO leptos reactivity here
//! — plain string building only, like `common::feed` — so `reactive_graph`
//! never sits on the public request path (the #173 escape). See
//! `docs/adr/0041` and `docs/inbound-data-handling.md` §4.
//!
//! The markup mirrors `web::pages::ui::PostDisplay`'s class structure
//! (`article.j-post` → `j-post-head` / `j-post-title` / `j-post-body` /
//! `j-post-foot`) so the seeded first paint and the reactive client-navigation
//! fallback share styling.

use crate::posts::{PostResponse, TimelinePage, TimelinePostSummary};
use crate::tags::TagSummary;
use crate::ui::{avatar, icon, taglist, topbar};
use common::render::RenderedHtml;
use common::tag::Tag;
use common::username::Username;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// The default theme applied to `<div class="j-root" data-theme=…>`. Lives here
/// (the shell-rendering layer) so the projector's server-painted shell and the
/// reactive `AppShell` share one value; re-exported from `pages` for the client.
pub const DEFAULT_THEME: &str = "studio";

/// The pre-paint auth-detection script (#181, ADR-0044). A tiny inline, blocking
/// `<head>` script: reads the localStorage auth marker (`jaunder_auth`, same key
/// as `auth::marker`) and marks `<html class="authed" data-user=…>` BEFORE first
/// paint, so CSS reserves the authed layout and the SPA boots already knowing.
/// Never external/deferred (a round-trip would guarantee paint-then-swap). The
/// redirect-pref (`jaunder_home_redirect`) read path is present with the safe
/// stay-default — nothing writes the key yet (ADR-0044 D7/D10). Bytes are
/// identical for every visitor → cacheability intact. Kept byte-identical in
/// `csr/index.html` (a `<!-- prettier-ignore -->`-pinned copy, drift-guarded by a
/// unit test) — deliberately minified so the two copies can match verbatim.
pub const PREPAINT_SCRIPT: &str = "<script>(function(){try{\
var m=localStorage.getItem('jaunder_auth');\
if(m){var u=JSON.parse(m).username;\
if(u){var e=document.documentElement;e.classList.add('authed');e.setAttribute('data-user',u);\
if(localStorage.getItem('jaunder_home_redirect')==='app'&&location.pathname==='/'){location.replace('/app');}}}\
}catch(_){}})();</script>";

/// The CSR SPA shell, embedded at compile time. The `cargo xtask build-csr` build
/// never writes `index.html` to `site_root` (#239); the server owns it and serves it — the
/// same way the projector renders its routes from constants. Single source of the
/// shell; copied to no build output.
pub const SPA_SHELL: &str = include_str!("../../../csr/index.html");

/// The initial data a public page is rendered from — serialized into the
/// projector's `#jaunder-seed` blob and adopted by the CSR client on boot.
///
/// Variants carry the route context (`username` / `tag`) the bare
/// [`TimelinePage`] lacks but the heading, title, and permalinks need — the
/// reactive components get it from the route params today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageSeed {
    SiteTimeline(TimelinePage),
    Profile {
        username: Username,
        page: TimelinePage,
    },
    SiteTag {
        tag: Tag,
        page: TimelinePage,
    },
    UserTag {
        username: Username,
        tag: Tag,
        page: TimelinePage,
    },
    Permalink(PostResponse),
}

/// Linking context for a post's footer tag chips — the pure mirror of
/// `pages::ui::TagContext` (which is a re-export of this). `SiteWide` links each
/// chip to `/tags/:slug` only; `ForUser` also renders the "· here" link to
/// `/~:username/tags/:slug`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagCtx {
    SiteWide,
    ForUser(Username),
}

/// Formats an RFC-3339 timestamp as `"YYYY-MM-DD HH:MM"`, falling back to the raw
/// string if it contains no `T` separator. Shared with the reactive components so
/// the projected post time and the reactive re-render coincide.
#[must_use]
pub fn format_post_time(ts: &str) -> String {
    if let Some(t_pos) = ts.find('T') {
        let date = &ts[..t_pos];
        let rest = &ts[t_pos + 1..];
        let time = if rest.len() >= 5 { &rest[..5] } else { rest };
        format!("{date} {time}")
    } else {
        ts.to_owned()
    }
}

/// Escape text for safe interpolation into HTML element or attribute content.
pub(crate) fn escape_html<S: AsRef<str>>(input: S) -> String {
    let input = input.as_ref();
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// The document `<head>` inner HTML: per-page title + description + Open Graph.
/// This is the SEO/discoverability payload — the whole reason the public
/// surface stays server-rendered.
#[must_use]
pub fn render_head(seed: &PageSeed) -> String {
    let (title, description) = match seed {
        PageSeed::Permalink(post) => (
            post.title
                .clone()
                .map_or_else(|| format!("Post by {}", post.username), String::from),
            post.summary.clone().unwrap_or_default(),
        ),
        PageSeed::Profile { username, .. } => (format!("Posts by {username}"), String::new()),
        PageSeed::SiteTimeline(_) => ("Jaunder".to_string(), String::new()),
        PageSeed::SiteTag { tag, .. } => (format!("#{tag}"), String::new()),
        PageSeed::UserTag { username, tag, .. } => (format!("#{tag} by {username}"), String::new()),
    };
    let title = escape_html(&title);
    let description = escape_html(&description);
    let mut head = format!(
        concat!(
            "<meta charset=\"utf-8\" />",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />",
            "<link rel=\"stylesheet\" href=\"/style/jaunder.css\" />",
            "<link rel=\"stylesheet\" href=\"/style/jaunder-themes.css\" />",
            "<title>{title}</title>",
            "<meta name=\"description\" content=\"{description}\" />",
            "<meta property=\"og:title\" content=\"{title}\" />",
            "<meta property=\"og:description\" content=\"{description}\" />",
        ),
        title = title,
        description = description,
    );
    head.push_str(&render_discovery(seed));
    head
}

/// Feed + RSD autodiscovery `<link>`s for the seed's surface, the pure mirror of
/// the reactive `FeedDiscovery`/`RsdDiscovery` components (`web::feed_discovery`)
/// so the projector's `<head>` carries the same discovery metadata the reactive
/// SSR render did (feed readers + `AtomPub` editors follow these). Each page emits
/// exactly what its reactive counterpart does: the RSS/Atom/JSON feed links for
/// its surface, and — only on the user-profile page — the RSD `EditURI` link. The
/// permalink page renders none. Post-boot the reactive components re-add
/// identical links; the duplicates are invisible.
fn render_discovery(seed: &PageSeed) -> String {
    use common::feed::{canonicalize, FeedFormat, FeedSurface};

    let mut out = String::new();

    let surface = match seed {
        PageSeed::SiteTimeline(_) => Some(FeedSurface::Site),
        PageSeed::SiteTag { tag, .. } => Some(FeedSurface::SiteTag { tag: tag.clone() }),
        PageSeed::Profile { username, .. } => Some(FeedSurface::User {
            username: username.clone(),
        }),
        PageSeed::UserTag { username, tag, .. } => Some(FeedSurface::UserTag {
            username: username.clone(),
            tag: tag.clone(),
        }),
        // The reactive permalink page renders no discovery links.
        PageSeed::Permalink(_) => None,
    };

    if let Some(surface) = surface {
        let label = feed_label(&surface);
        for (format, suffix, mime) in [
            (FeedFormat::Rss, "RSS", "application/rss+xml"),
            (FeedFormat::Atom, "Atom", "application/atom+xml"),
            (FeedFormat::Json, "JSON Feed", "application/feed+json"),
        ] {
            let _ = write!(
                out,
                "<link rel=\"alternate\" type=\"{mime}\" title=\"{title}\" href=\"{href}\" />",
                title = escape_html(format!("{label} ({suffix})")),
                href = escape_html(canonicalize(&surface, format)),
            );
        }
    }

    // Only the reactive user-profile page hoists the RSD link (the user-tag page
    // does not), so mirror that exactly.
    if let PageSeed::Profile { username, .. } = seed {
        let _ = write!(
            out,
            "<link rel=\"EditURI\" type=\"application/rsd+xml\" title=\"AtomPub (RSD)\" href=\"{href}\" />",
            href = escape_html(format!("/~{username}/rsd.xml")),
        );
    }

    out
}

/// Human-readable feed title per surface — the pure mirror of the reactive
/// `web::feed_discovery::surface_label`.
fn feed_label(surface: &common::feed::FeedSurface) -> String {
    use common::feed::FeedSurface;
    match surface {
        FeedSurface::Site => "Site feed".to_string(),
        FeedSurface::SiteTag { tag } => format!("#{tag} feed"),
        FeedSurface::User { username } => format!("@{username} feed"),
        FeedSurface::UserTag { username, tag } => format!("@{username} #{tag} feed"),
    }
}

/// The full anonymous `#app` shell the projector serves: the exact `j-root`
/// layout the reactive `App`/`AppShell` produces for an anonymous viewer (the
/// sidebar, the main region, and the per-route `<main>` content), so removing
/// `#app` and mounting the CSR app on boot causes no reflow. The authed extras
/// (footer avatar, authed nav, action columns) layer on top reactively once
/// `current_user` resolves (that is #181, and needs no coincidence).
/// `BackupBanner` renders nothing for an anonymous viewer, so it is omitted here.
#[must_use]
pub fn render_shell(seed: &PageSeed) -> String {
    format!(
        concat!(
            "<div class=\"j-root\" data-theme=\"{theme}\"><div class=\"j-shell\">",
            "<aside class=\"j-sidebar\">{sidebar}</aside>",
            "<div class=\"j-main-region\"><main class=\"j-main\">{body}</main></div></div></div>",
        ),
        theme = DEFAULT_THEME,
        sidebar = render_sidebar(""),
        body = render_body(seed),
    )
}

/// The `<main class="j-main">` inner content for a route — mirrors each reactive
/// page's markup (Topbar + wrappers + posts + load-more) so the seeded first paint
/// coincides. Split from [`render_shell`] so the permalink Suspense fallback can
/// reuse just [`permalink_article`].
pub(crate) fn render_body(seed: &PageSeed) -> String {
    match seed {
        // Permalink: no Topbar; a single article inside `j-scroll`/`j-page`.
        PageSeed::Permalink(post) => format!(
            "<div class=\"j-scroll\"><div class=\"j-page\">{}</div></div>",
            permalink_article(post),
        ),
        // Home (anonymous "Local" mode): Topbar + hero + a bare `j-scroll` of posts.
        PageSeed::SiteTimeline(page) => {
            let topbar = topbar::render(
                "jaunder.local",
                Some("Read-only \u{00b7} posts originating on this instance"),
                "<a href=\"/login\" class=\"j-btn\">Sign in</a>\
                 <a href=\"/register\" class=\"j-btn is-primary\">Register</a>",
            );
            let scroll = if page.posts.is_empty() {
                "<p>No posts yet.</p>".to_string()
            } else {
                format!(
                    "{}{}",
                    render_posts(&page.posts, &TagCtx::SiteWide),
                    render_load_more(page.has_more),
                )
            };
            format!(
                "{topbar}{hero}<div class=\"j-scroll\">{scroll}</div>",
                hero = render_hero(),
            )
        }
        PageSeed::Profile { username, page } => render_timeline_page(
            &topbar::render(&format!("Posts by {username}"), Some("User timeline"), ""),
            &page.posts,
            page.has_more,
            &TagCtx::ForUser(username.clone()),
            "No posts yet.",
        ),
        PageSeed::SiteTag { tag, page } => render_timeline_page(
            &topbar::render(&format!("#{tag}"), Some("Posts on this instance"), ""),
            &page.posts,
            page.has_more,
            &TagCtx::SiteWide,
            "No posts with this tag yet.",
        ),
        PageSeed::UserTag {
            username,
            tag,
            page,
        } => render_timeline_page(
            &topbar::render(
                &format!("#{tag}"),
                Some(&format!("Posts by ~{username}")),
                "",
            ),
            &page.posts,
            page.has_more,
            &TagCtx::ForUser(username.clone()),
            "No posts with this tag yet.",
        ),
    }
}

/// One permalink post as an `<article>`. Shared by the projector's permalink page
/// and the reactive `PostPage`'s Suspense fallback so they coincide.
#[must_use]
pub(crate) fn permalink_article(post: &PostResponse) -> String {
    let ctx = TagCtx::ForUser(post.username.clone());
    render_post_article(&PostView {
        username: &post.username,
        title: post.title.as_deref(),
        banner: None,
        summary: post.summary.as_deref(),
        rendered_html: &post.rendered_html,
        time: &format_post_time(post.published_at.as_deref().unwrap_or(&post.created_at)),
        permalink: post.permalink.as_deref().unwrap_or_default(),
        tags: &post.tags,
        tag_ctx: &ctx,
    })
}

/// The home page hero block (constant copy), mirroring `home.rs`.
#[must_use]
fn render_hero() -> String {
    "<div class=\"j-hero\"><h1>One timeline. Every protocol.</h1><p>Jaunder is a self-hosted \
     social client that reads from ActivityPub, AT Protocol, RSS, Atom, and JSON Feed \u{2014} and \
     publishes back out to the ones you choose. Below: what\u{2019}s been posted from this \
     instance.</p></div>"
        .to_string()
}

/// The non-functional "Load more" button the projector paints so the reactive
/// button (which replaces it on boot) doesn't reflow. Rendered only when there is
/// a next page, matching the reactive `has_more` guard.
#[must_use]
fn render_load_more(has_more: bool) -> String {
    if has_more {
        "<button>Load more</button>".to_string()
    } else {
        String::new()
    }
}

/// Concatenated `<article>`s for a list of timeline posts (anonymous, so no action
/// column), each coincident with the reactive `PostCard`/`PostDisplay` output.
#[must_use]
fn render_posts(posts: &[TimelinePostSummary], tag_ctx: &TagCtx) -> String {
    let mut out = String::new();
    for post in posts {
        out.push_str(&render_post_article(&PostView {
            username: &post.username,
            title: post.title.as_deref(),
            banner: None,
            summary: post.summary.as_deref(),
            rendered_html: &post.rendered_html,
            time: &format_post_time(&post.published_at),
            permalink: &post.permalink,
            tags: &post.tags,
            tag_ctx,
        }));
    }
    out
}

/// The fields needed to render one post, borrowed from a `PostResponse` or a
/// `TimelinePostSummary`. `time` is already formatted (see [`format_post_time`]).
pub(crate) struct PostView<'a> {
    pub username: &'a Username,
    pub title: Option<&'a str>,
    pub banner: Option<&'a str>,
    pub summary: Option<&'a str>,
    pub rendered_html: &'a RenderedHtml,
    pub time: &'a str,
    pub permalink: &'a str,
    pub tags: &'a [TagSummary],
    pub tag_ctx: &'a TagCtx,
}

/// One post as a full `<article class="j-post">…</article>` — the projector's
/// coincident unit. The reactive `PostDisplay` renders the SAME inner HTML (via
/// [`render_post_inner`] / [`render_post_content`]) so a seeded first paint and
/// the reactive re-render are byte-identical.
#[must_use]
pub(crate) fn render_post_article(view: &PostView) -> String {
    format!(
        "<article class=\"j-post\">{}</article>",
        render_post_inner(view)
    )
}

/// The inner HTML of `<article class="j-post">` for the **anonymous** layout:
/// avatar + the content column, with no author-action slot. Mirrors
/// `PostDisplay`'s children when no `children` are passed.
#[must_use]
pub(crate) fn render_post_inner(view: &PostView) -> String {
    format!(
        concat!(
            "{avatar}",
            "<div style=\"min-width:0;display:flex;gap:8px;align-items:flex-start\">",
            "<div style=\"flex:1;min-width:0\">{content}</div>",
            "</div>",
        ),
        avatar = avatar::render(view.username, 38),
        content = render_post_content(view),
    )
}

/// The inner HTML of the post's content column (`<div style="flex:1;min-width:0">`):
/// header, title, optional draft banner, summary, body, footer. Shared by the
/// anonymous [`render_post_inner`] and the reactive author layout, which slots
/// this into the same content `<div>` via `inner_html` and overlays the reactive
/// action column as a sibling. It is deliberately **viewer-independent** (#181,
/// ADR-0044 D4): the owner's own-post content column must be byte-identical to the
/// projector's anonymous paint, so the timestamp always stays in the header and
/// the action column is purely additive — never a content change.
#[must_use]
pub(crate) fn render_post_content(view: &PostView) -> String {
    let user = escape_html(view.username);
    let header_time = format!(
        "<span class=\"j-spacer\"></span><span class=\"j-post-time\">{}</span>",
        escape_html(view.time)
    );
    let title_html = view.title.map_or_else(String::new, |t| {
        let t = escape_html(t);
        if view.permalink.is_empty() {
            format!("<div class=\"j-post-title\">{t}</div>")
        } else {
            format!(
                "<div class=\"j-post-title\"><a href=\"{}\">{t}</a></div>",
                escape_html(view.permalink)
            )
        }
    });
    let banner_html = view.banner.map_or_else(String::new, |b| {
        format!("<p class=\"draft-banner\">{}</p>", escape_html(b))
    });
    let summary_html = view.summary.map_or_else(String::new, |s| {
        format!("<p class=\"j-post-summary\">{}</p>", escape_html(s))
    });
    format!(
        concat!(
            "<header class=\"j-post-head\">",
            "<span class=\"j-post-name\">{user}</span>",
            "<span class=\"j-post-handle\">@{user}</span>",
            "{header_time}",
            "</header>",
            "{title_html}{banner_html}{summary_html}",
            "<div class=\"j-post-body\">{body}</div>",
            "<footer class=\"j-post-foot\">{tags}<span class=\"j-spacer\"></span></footer>",
        ),
        user = user,
        header_time = header_time,
        title_html = title_html,
        banner_html = banner_html,
        summary_html = summary_html,
        body = view.rendered_html,
        tags = taglist::render(view.tags, view.tag_ctx),
    )
}

/// A profile / tag timeline page's `<main>` content: the given `topbar`, then
/// `j-scroll` > `j-page` holding either the empty placeholder or a `<div>` of
/// posts followed by the load-more button — mirroring `UserTimelinePage` /
/// `SiteTagPage` / `UserTagPage` (the anonymous `SubscribeButton` renders nothing).
#[must_use]
fn render_timeline_page(
    topbar: &str,
    posts: &[TimelinePostSummary],
    has_more: bool,
    tag_ctx: &TagCtx,
    empty_text: &str,
) -> String {
    let inner = if posts.is_empty() {
        format!("<p>{}</p>", escape_html(empty_text))
    } else {
        format!(
            "<div>{}</div>{}",
            render_posts(posts, tag_ctx),
            render_load_more(has_more),
        )
    };
    format!("{topbar}<div class=\"j-scroll\"><div class=\"j-page\">{inner}</div></div>")
}

/// SVG path `d` attribute strings for all Jaunder icons. Shared by the reactive
/// [`crate::ui::Icon`] component and the pure [`crate::ui::icon::render`].
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
    pub const REFRESH: &'static str = "M15.5 8A6 6 0 1 0 16 11.5 M15.5 4v4h-4";
}

/// Sidebar nav items: `(key, label, icon_path, href, auth_required)`. Shared by
/// [`render_sidebar`] (anonymous → the `href.is_some() && !auth_required` subset)
/// and the reactive authed sidebar in `pages::ui::Sidebar`.
pub const NAV_ITEMS: &[(&str, &str, &str, Option<&'static str>, bool)] = &[
    ("home", "Home", Icons::HOME, Some("/"), false),
    // The authed-only cockpit (#181, ADR-0044 D6): the owner's personalized feed at
    // /app. `auth_required = true` keeps it out of the cacheable anonymous sidebar
    // (`render_sidebar` filters `href.is_some() && !auth_required`) — it appears
    // only in the authed sidebar, so the projector's anonymous paint is unchanged.
    ("app", "Feed", Icons::HOME, Some("/app"), true),
    ("local", "Local", Icons::LOCAL, None, true),
    ("federated", "Federated", Icons::FED, None, true),
    ("replies", "Replies", Icons::REPLY, None, true),
    ("bookmarks", "Bookmarks", Icons::BOOKMARK, None, true),
    ("drafts", "Drafts", Icons::EDIT, Some("/drafts"), true),
    ("media", "Media", Icons::MEDIA, Some("/media"), true),
    (
        "audiences",
        "Audiences",
        Icons::BOOKMARK,
        Some("/audiences"),
        true,
    ),
    ("settings", "Settings", Icons::COG, None, true),
];

/// The static demo "Sources" rows in the sidebar: `(proto, name, sub)`.
pub const SIDEBAR_SOURCES: &[(&str, &str, &str)] = &[
    ("atproto", "Bluesky", "mara.bsky.social"),
    ("activitypub", "Mastodon", "@mara@hachyderm.io"),
    ("rss", "Ivy Chen", "weeknotes"),
    ("jsonfeed", "Manton", "manton.org"),
];

/// The inner HTML of the **anonymous** `<aside class="j-sidebar">`: brand, search,
/// the public nav (items with an href and no auth requirement — just "Home"),
/// the sources section, and an empty footer. The reactive `pages::ui::Sidebar`
/// injects this verbatim via `inner_html` for the anonymous viewer, so a seeded
/// first paint and the reactive re-render coincide; authed users get the reactive
/// build (extra nav, footer avatar) layered on top (#181).
#[must_use]
pub fn render_sidebar(active_key: &str) -> String {
    let mut out = String::from(
        "<a class=\"j-brand\" href=\"/\" style=\"text-decoration:none;color:inherit\">\
         <div class=\"j-brand-mark\">j</div><div class=\"j-brand-text\">Jaunder</div></a>",
    );
    let _ = write!(
        out,
        "<div class=\"j-search\">{}<span>Search</span><span class=\"j-kbd\">\u{2318}K</span></div>",
        icon::render(Icons::SEARCH, 14),
    );
    out.push_str("<nav class=\"j-nav\">");
    for &(key, label, icon_path, href, auth_required) in NAV_ITEMS {
        let Some(href) = href else { continue };
        if auth_required {
            continue;
        }
        let active = if key == active_key { " is-active" } else { "" };
        let _ = write!(
            out,
            "<a class=\"j-nav-item{active}\" href=\"{href}\">{icon}<span>{label}</span></a>",
            icon = icon::render(icon_path, 16),
        );
    }
    out.push_str("</nav><div><div class=\"j-sb-head\"><span>Sources</span><span class=\"j-sb-add\">+</span></div>");
    for &(proto, name, sub) in SIDEBAR_SOURCES {
        let _ = write!(
            out,
            "<div class=\"j-source\"><span class=\"j-dot\" style=\"width:8px;height:8px;border-radius:4px;background:var(--c-{proto})\"></span>\
             <div style=\"flex:1;min-width:0\"><div class=\"j-source-name\">{name}</div><div class=\"j-source-sub\">{sub}</div></div></div>",
        );
    }
    out.push_str("</div><div class=\"j-sb-foot\"></div>");
    out
}

/// Formats a byte count as a human-readable size (`B` / `KB` / `MB` / `GB`, one
/// decimal). Shared display formatter, host-tested here.
#[expect(
    clippy::cast_precision_loss,
    reason = "byte counts < 2^52 convert to f64 exactly; larger values only affect a \
              human-readable one-decimal display, so any loss is immaterial"
)]
pub fn format_bytes(bytes: i64) -> String {
    const KB: i64 = 1_024;
    const MB: i64 = 1_024 * KB;
    const GB: i64 = 1_024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::ids::PostId;

    #[test]
    fn format_bytes_displays_bytes_below_kb() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_displays_kb_range() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn format_bytes_displays_mb_range() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 2), "2.0 MB");
    }

    #[test]
    fn format_bytes_displays_gb_range() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn default_theme_is_nonempty() {
        assert!(!DEFAULT_THEME.is_empty());
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

    #[test]
    fn prepaint_script_is_inline_blocking_and_reads_the_marker() {
        let s = PREPAINT_SCRIPT;
        assert!(s.starts_with("<script>") && s.ends_with("</script>"), "{s}");
        // No async/defer/src — a network round-trip would defeat pre-paint.
        assert!(
            !s.contains("src=") && !s.contains("defer") && !s.contains("async"),
            "{s}"
        );
        // Reads the same key + field the marker module writes.
        assert!(s.contains("jaunder_auth"), "{s}");
        assert!(s.contains(".username"), "{s}");
        assert!(s.contains("classList") && s.contains("authed"), "{s}");
    }

    #[test]
    fn author_content_column_coincides_with_the_anonymous_paint() {
        // #181, ADR-0044 D4/D8: the authed own-post PostDisplay injects
        // render_post_content into the same content <div> the projector's anonymous
        // render_post_inner wraps. render_post_content is viewer-independent, so the
        // authed re-render cannot diverge from the paint — no localized flash.
        let ctx = TagCtx::ForUser("alice".parse::<Username>().unwrap());
        let author: Username = "alice".parse().unwrap();
        let body = RenderedHtml::from_trusted("<p>b</p>");
        let view = PostView {
            username: &author,
            title: Some("T"),
            banner: None,
            summary: None,
            rendered_html: &body,
            time: "2026-01-01 00:00",
            permalink: "/~alice/x",
            tags: &[],
            tag_ctx: &ctx,
        };
        let content = render_post_content(&view);
        // The timestamp stays in the header — the specific divergence #181 fixed. The
        // projector painted it anonymously, so the authed content column must too.
        assert!(
            content.contains("<span class=\"j-post-time\">2026-01-01 00:00</span>"),
            "content column must keep the header time to coincide: {content}"
        );
        // The anonymous inner the projector paints embeds that exact content column
        // verbatim — the authed Some arm injects the identical string.
        assert!(
            render_post_inner(&view).contains(&content),
            "anonymous inner must embed the identical content column: {content}"
        );
    }

    #[test]
    fn index_html_shell_contains_the_prepaint_script() {
        // The projector's SPA-shell fallback IS csr/index.html; it must carry the
        // identical pre-paint script (a prettier-ignored, minified copy) so
        // authed-only / shell-fallback pages pre-paint too.
        let index = include_str!("../../../csr/index.html");
        assert!(
            index.contains(PREPAINT_SCRIPT),
            "csr/index.html must embed render::PREPAINT_SCRIPT verbatim (drift guard)"
        );
    }

    #[test]
    fn csr_index_html_boots_wasm_with_an_explicit_url() {
        // Fast unit smoke (#234): the SPA shell must pass an explicit wasm URL to
        // init(), not the arg-less init() that falls back to wasm-bindgen's
        // `jaunder_bg.wasm` default. This runs in `check`; `cargo xtask audit-wasm`
        // is what ties this URL to the file the build actually emits.
        let index = include_str!("../../../csr/index.html");
        assert!(
            index.contains(r#"init("/pkg/jaunder.wasm")"#),
            "csr/index.html must boot via an explicit init(\"/pkg/jaunder.wasm\") (drift guard #234)"
        );
    }

    fn sample_post() -> PostResponse {
        PostResponse {
            post_id: PostId::from(7),
            username: "alice".parse::<Username>().unwrap(),
            title: Some("Hello & <World>".into()),
            slug: "hello".parse().unwrap(),
            body: "raw".into(),
            format: "markdown".into(),
            rendered_html: RenderedHtml::from_trusted("<p>Hi <em>there</em></p>"),
            created_at: "2026-01-02T03:04:05Z".into(),
            published_at: Some("2026-01-02T03:04:05Z".into()),
            is_draft: false,
            is_author: false,
            permalink: Some("/~alice/2026/01/02/hello".into()),
            tags: vec![TagSummary {
                slug: "rust".parse().unwrap(),
                display: "Rust".parse().unwrap(),
            }],
            summary: None,
        }
    }

    fn sample_summary() -> TimelinePostSummary {
        TimelinePostSummary {
            post_id: PostId::from(1),
            username: "bob".parse::<Username>().unwrap(),
            title: Some("First".into()),
            summary: Some("An excerpt".into()),
            slug: "first".parse().unwrap(),
            rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
            created_at: "2026-01-01T00:00:00Z".into(),
            published_at: "2026-01-01T00:00:00Z".into(),
            permalink: "/~bob/2026/01/01/first".into(),
            is_author: false,
            tags: vec![],
        }
    }

    #[test]
    fn escape_replaces_markup_metacharacters() {
        assert_eq!(escape_html("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#39;");
    }

    #[test]
    fn permalink_body_escapes_title_but_injects_rendered_html_raw() {
        let html = render_body(&PageSeed::Permalink(sample_post()));
        assert!(
            html.contains("Hello &amp; &lt;World&gt;"),
            "title must be escaped: {html}"
        );
        assert!(
            html.contains("<p>Hi <em>there</em></p>"),
            "rendered_html must be injected raw: {html}"
        );
        assert!(
            html.contains("<article class=\"j-post\""),
            "expected article: {html}"
        );
        assert!(
            html.contains("/~alice/2026/01/02/hello"),
            "expected permalink: {html}"
        );
        assert!(html.contains("Rust"), "expected tag display: {html}");
    }

    #[test]
    fn permalink_head_sets_escaped_title_and_og() {
        let head = render_head(&PageSeed::Permalink(sample_post()));
        assert!(
            head.contains("<title>Hello &amp; &lt;World&gt;</title>"),
            "{head}"
        );
        assert!(head.contains("<meta property=\"og:title\""), "{head}");
    }

    #[test]
    fn timeline_renders_each_post_and_heading() {
        let page = TimelinePage {
            posts: vec![sample_summary()],
            next_cursor_created_at: None,
            next_cursor_post_id: None,
            has_more: false,
        };
        let html = render_body(&PageSeed::Profile {
            username: "bob".parse::<Username>().unwrap(),
            page,
        });
        assert!(html.contains("Posts by bob"), "expected heading: {html}");
        assert!(html.contains("First"), "expected post title: {html}");
        assert!(html.contains("<p>body</p>"), "expected body: {html}");
    }

    #[test]
    fn empty_timeline_renders_placeholder() {
        let page = TimelinePage {
            posts: vec![],
            next_cursor_created_at: None,
            next_cursor_post_id: None,
            has_more: false,
        };
        let html = render_body(&PageSeed::SiteTimeline(page));
        assert!(html.contains("No posts yet."), "{html}");
    }

    fn one_post_page() -> TimelinePage {
        TimelinePage {
            posts: vec![sample_summary()],
            next_cursor_created_at: None,
            next_cursor_post_id: None,
            has_more: false,
        }
    }

    #[test]
    fn head_titles_cover_every_page_kind() {
        let cases = [
            (
                PageSeed::SiteTimeline(one_post_page()),
                "<title>Jaunder</title>",
            ),
            (
                PageSeed::Profile {
                    username: "bob".parse::<Username>().unwrap(),
                    page: one_post_page(),
                },
                "<title>Posts by bob</title>",
            ),
            (
                PageSeed::SiteTag {
                    tag: "rust".parse().unwrap(),
                    page: one_post_page(),
                },
                "<title>#rust</title>",
            ),
            (
                PageSeed::UserTag {
                    username: "bob".parse::<Username>().unwrap(),
                    tag: "rust".parse().unwrap(),
                    page: one_post_page(),
                },
                "<title>#rust by bob</title>",
            ),
        ];
        for (seed, expected_title) in cases {
            let head = render_head(&seed);
            assert!(head.contains(expected_title), "{head}");
        }
    }

    #[test]
    fn body_covers_tag_page_headings() {
        let site = render_body(&PageSeed::SiteTag {
            tag: "rust".parse().unwrap(),
            page: one_post_page(),
        });
        // Tag pages render the Topbar (h1 + sub), then j-scroll > j-page > posts.
        assert!(site.contains("<h1>#rust</h1>"), "{site}");
        assert!(site.contains("Posts on this instance"), "{site}");
        assert!(
            site.contains("<div class=\"j-scroll\"><div class=\"j-page\">"),
            "{site}"
        );
        assert!(site.contains("First"), "expected post rendered: {site}");

        let user = render_body(&PageSeed::UserTag {
            username: "bob".parse::<Username>().unwrap(),
            tag: "rust".parse().unwrap(),
            page: one_post_page(),
        });
        assert!(user.contains("<h1>#rust</h1>"), "{user}");
        assert!(user.contains("Posts by ~bob"), "{user}");
    }

    #[test]
    fn shell_wraps_body_in_j_root_with_sidebar_and_main() {
        let html = render_shell(&PageSeed::SiteTimeline(one_post_page()));
        assert!(
            html.starts_with(
                "<div class=\"j-root\" data-theme=\"studio\"><div class=\"j-shell\">\
                 <aside class=\"j-sidebar\">"
            ),
            "{html}"
        );
        // Sidebar inner is present, then the main region.
        assert!(html.contains("j-brand-text"), "{html}");
        assert!(
            html.contains("</aside><div class=\"j-main-region\"><main class=\"j-main\">"),
            "{html}"
        );
        assert!(html.ends_with("</main></div></div></div>"), "{html}");
    }

    #[test]
    fn home_local_body_has_topbar_hero_signin_and_posts() {
        let html = render_body(&PageSeed::SiteTimeline(one_post_page()));
        assert!(html.contains("<h1>jaunder.local</h1>"), "{html}");
        assert!(
            html.contains("<a href=\"/login\" class=\"j-btn\">Sign in</a>"),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"/register\" class=\"j-btn is-primary\">Register</a>"),
            "{html}"
        );
        assert!(html.contains("<div class=\"j-hero\">"), "{html}");
        // Posts sit directly in j-scroll for the home page (no inner j-page wrapper).
        assert!(
            html.contains("<div class=\"j-scroll\"><article class=\"j-post\">"),
            "{html}"
        );
    }

    #[test]
    fn load_more_button_rendered_only_when_has_more() {
        let mut page = one_post_page();
        page.has_more = true;
        let with = render_body(&PageSeed::SiteTimeline(page));
        assert!(with.contains("<button>Load more</button>"), "{with}");

        let without = render_body(&PageSeed::SiteTimeline(one_post_page()));
        assert!(!without.contains("Load more"), "{without}");
    }

    #[test]
    fn timeline_page_empty_states_differ_by_route() {
        let empty = TimelinePage {
            posts: vec![],
            next_cursor_created_at: None,
            next_cursor_post_id: None,
            has_more: false,
        };
        let profile = render_body(&PageSeed::Profile {
            username: "bob".parse::<Username>().unwrap(),
            page: empty.clone(),
        });
        assert!(profile.contains("<p>No posts yet.</p>"), "{profile}");
        let tag = render_body(&PageSeed::SiteTag {
            tag: "rust".parse().unwrap(),
            page: empty,
        });
        assert!(tag.contains("<p>No posts with this tag yet.</p>"), "{tag}");
    }

    #[test]
    fn permalink_body_has_no_topbar_and_wraps_article_in_page() {
        let html = render_body(&PageSeed::Permalink(sample_post()));
        assert!(
            !html.contains("j-topbar"),
            "permalink has no topbar: {html}"
        );
        assert!(
            html.starts_with(
                "<div class=\"j-scroll\"><div class=\"j-page\"><article class=\"j-post\">"
            ),
            "{html}"
        );
    }

    #[test]
    fn body_permalink_falls_back_to_created_at_when_unpublished() {
        let mut post = sample_post();
        post.published_at = None;
        post.permalink = None;
        let html = render_body(&PageSeed::Permalink(post));
        // The time is formatted (mirroring the reactive `format_post_time`), not raw.
        assert!(html.contains("2026-01-02 03:04"), "{html}");
    }

    #[test]
    fn post_content_always_shows_the_header_time() {
        // Viewer-independent (#181, ADR-0044 D4): the timestamp stays in the header
        // for every viewer, so the owner's own-post content column coincides with
        // the projector's anonymous paint (the action column is purely additive).
        let ctx = TagCtx::SiteWide;
        let author: Username = "bob".parse().unwrap();
        let body = RenderedHtml::from_trusted("<p>b</p>");
        let view = PostView {
            username: &author,
            title: None,
            banner: None,
            summary: None,
            rendered_html: &body,
            time: "2026-01-01 00:00",
            permalink: "",
            tags: &[],
            tag_ctx: &ctx,
        };
        assert!(render_post_content(&view)
            .contains("<span class=\"j-post-time\">2026-01-01 00:00</span>"));
    }

    #[test]
    fn post_content_renders_draft_banner_when_present() {
        let ctx = TagCtx::SiteWide;
        let author: Username = "bob".parse().unwrap();
        let body = RenderedHtml::from_trusted("<p>b</p>");
        let view = PostView {
            username: &author,
            title: None,
            banner: Some("Draft - visible only to you"),
            summary: Some("An excerpt"),
            rendered_html: &body,
            time: "2026-01-01 00:00",
            permalink: "",
            tags: &[],
            tag_ctx: &ctx,
        };
        let html = render_post_content(&view);
        assert!(
            html.contains("<p class=\"draft-banner\">Draft - visible only to you</p>"),
            "{html}"
        );
        assert!(
            html.contains("<p class=\"j-post-summary\">An excerpt</p>"),
            "{html}"
        );
    }

    #[test]
    fn post_article_wraps_inner_in_j_post_article() {
        let ctx = TagCtx::SiteWide;
        let author: Username = "bob".parse().unwrap();
        let body = RenderedHtml::from_trusted("<p>b</p>");
        let view = PostView {
            username: &author,
            title: Some("T"),
            banner: None,
            summary: None,
            rendered_html: &body,
            time: "2026-01-01 00:00",
            permalink: "/~bob/x",
            tags: &[],
            tag_ctx: &ctx,
        };
        let html = render_post_article(&view);
        assert!(html.starts_with("<article class=\"j-post\">"), "{html}");
        assert!(html.ends_with("</article>"), "{html}");
        // Title links to the permalink, mirroring `PostDisplay`.
        assert!(
            html.contains("<div class=\"j-post-title\"><a href=\"/~bob/x\">T</a></div>"),
            "{html}"
        );
    }

    #[test]
    fn page_seed_round_trips_through_json() {
        let seed = PageSeed::Permalink(sample_post());
        let json = serde_json::to_string(&seed).unwrap();
        let back: PageSeed = serde_json::from_str(&json).unwrap();
        assert_eq!(seed, back);
    }

    #[test]
    fn sidebar_renders_brand_public_nav_sources_and_empty_foot() {
        let html = render_sidebar("home");
        assert!(
            html.contains("<div class=\"j-brand-text\">Jaunder</div>"),
            "{html}"
        );
        // Public nav = Home only; active class applied for the matching key.
        assert!(
            html.contains("<a class=\"j-nav-item is-active\" href=\"/\">"),
            "{html}"
        );
        assert!(html.contains("<span>Home</span>"), "{html}");
        // Auth-required items must NOT appear for the anonymous sidebar.
        assert!(!html.contains(">Drafts<"), "{html}");
        assert!(!html.contains(">Settings<"), "{html}");
        // Sources section + empty footer.
        assert!(
            html.contains("<div class=\"j-source-name\">Bluesky</div>"),
            "{html}"
        );
        assert!(html.ends_with("<div class=\"j-sb-foot\"></div>"), "{html}");
    }

    #[test]
    fn sidebar_active_class_absent_for_non_home_route() {
        let html = render_sidebar("tags");
        assert!(
            html.contains("<a class=\"j-nav-item\" href=\"/\">"),
            "{html}"
        );
    }
}
