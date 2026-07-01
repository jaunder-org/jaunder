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
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

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
        username: String,
        page: TimelinePage,
    },
    SiteTag {
        tag: String,
        page: TimelinePage,
    },
    UserTag {
        username: String,
        tag: String,
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
    ForUser(String),
}

/// Derives `(initials, hue)` from a display name. `initials`: first character of
/// each of the first two whitespace-separated words, uppercased. `hue`: sum of
/// all char codes mod 360. Shared by the reactive `Avatar` component and the
/// pure [`render_avatar`] so a seeded avatar and its reactive re-render coincide.
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

/// One avatar chip as `<div class="j-av" …>`, byte-identical to the reactive
/// `pages::ui::Avatar` component's output for the same `(name, size)`.
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss)]
#[must_use]
pub fn render_avatar(name: &str, size: u32) -> String {
    let (initials, hue) = avatar_parts(name);
    let font_size = (size as f32 * 0.36).round() as u32;
    format!(
        "<div class=\"j-av\" style=\"width:{size}px;height:{size}px;background:oklch(0.58 0.07 {hue});font-size:{font_size}px\">{initials}</div>",
        initials = escape_html(&initials),
    )
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
fn escape_html(input: &str) -> String {
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
                .unwrap_or_else(|| format!("Post by {}", post.username)),
            post.summary.clone().unwrap_or_default(),
        ),
        PageSeed::Profile { username, .. } => (format!("Posts by {username}"), String::new()),
        PageSeed::SiteTimeline(_) => ("Jaunder".to_string(), String::new()),
        PageSeed::SiteTag { tag, .. } => (format!("#{tag}"), String::new()),
        PageSeed::UserTag { username, tag, .. } => (format!("#{tag} by {username}"), String::new()),
    };
    let title = escape_html(&title);
    let description = escape_html(&description);
    format!(
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
    )
}

/// The `<div id="app">` inner HTML: the semantic, crawlable page content.
#[must_use]
pub fn render_body(seed: &PageSeed) -> String {
    match seed {
        PageSeed::Permalink(post) => {
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
                is_author: false,
            })
        }
        PageSeed::SiteTimeline(page) => timeline("Jaunder", &page.posts, &TagCtx::SiteWide),
        PageSeed::Profile { username, page } => timeline(
            &format!("Posts by {username}"),
            &page.posts,
            &TagCtx::ForUser(username.clone()),
        ),
        PageSeed::SiteTag { tag, page } => {
            timeline(&format!("#{tag}"), &page.posts, &TagCtx::SiteWide)
        }
        PageSeed::UserTag {
            username,
            tag,
            page,
        } => timeline(
            &format!("#{tag} by {username}"),
            &page.posts,
            &TagCtx::ForUser(username.clone()),
        ),
    }
}

/// The fields needed to render one post, borrowed from a `PostResponse` or a
/// `TimelinePostSummary`. `time` is already formatted (see [`format_post_time`]).
pub(crate) struct PostView<'a> {
    pub username: &'a str,
    pub title: Option<&'a str>,
    pub banner: Option<&'a str>,
    pub summary: Option<&'a str>,
    pub rendered_html: &'a str,
    pub time: &'a str,
    pub permalink: &'a str,
    pub tags: &'a [TagSummary],
    pub tag_ctx: &'a TagCtx,
    pub is_author: bool,
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
        avatar = render_avatar(view.username, 38),
        content = render_post_content(view),
    )
}

/// The inner HTML of the post's content column (`<div style="flex:1;min-width:0">`):
/// header, title, optional draft banner, summary, body, footer. Shared by the
/// anonymous [`render_post_inner`] and the reactive author layout, which slots
/// this into the same content `<div>` via `inner_html` and overlays the reactive
/// action column as a sibling. When `is_author`, the header time is omitted (it
/// moves to the action column), exactly as the reactive `PostDisplay` does.
#[must_use]
pub(crate) fn render_post_content(view: &PostView) -> String {
    let user = escape_html(view.username);
    let header_time = if view.is_author {
        String::new()
    } else {
        format!(
            "<span class=\"j-spacer\"></span><span class=\"j-post-time\">{}</span>",
            escape_html(view.time)
        )
    };
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
        tags = render_tag_list(view.tags, view.tag_ctx),
    )
}

/// A list of post summaries as a timeline page (heading + `<article>`s). The
/// full-shell assembly (#180) replaces the bare heading with the reactive
/// `Topbar`; for now this keeps the existing content-only structure.
fn timeline(heading: &str, posts: &[TimelinePostSummary], tag_ctx: &TagCtx) -> String {
    let mut out = format!(
        "<h1 class=\"j-timeline-title\">{}</h1>",
        escape_html(heading)
    );
    if posts.is_empty() {
        out.push_str("<p class=\"j-empty\">No posts yet.</p>");
        return out;
    }
    out.push_str("<div class=\"j-timeline\">");
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
            is_author: false,
        }));
    }
    out.push_str("</div>");
    out
}

/// The footer tag chips: a `<span class="j-tag-list">` of `<span class="j-tag-cell">`
/// chips, each a `#display` link to `/tags/:slug`, plus the "· here" link under
/// [`TagCtx::ForUser`]. The reactive post markup injects this via `inner_html`, so
/// there is one source of truth for the chip markup (it replaced the old reactive
/// `TagList` component).
#[must_use]
pub(crate) fn render_tag_list(tags: &[TagSummary], ctx: &TagCtx) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let mut out = String::from("<span class=\"j-tag-list\">");
    for tag in tags {
        let slug = escape_html(&tag.slug);
        let _ = write!(
            out,
            "<span class=\"j-tag-cell\"><a class=\"j-tag\" href=\"/tags/{slug}\">#{display}</a>",
            display = escape_html(&tag.display),
        );
        if let TagCtx::ForUser(username) = ctx {
            let _ = write!(
                out,
                "<a class=\"j-tag-here\" href=\"/~{user}/tags/{slug}\" title=\"On this blog\">\u{00b7} here</a>",
                user = escape_html(username),
            );
        }
        out.push_str("</span>");
    }
    out.push_str("</span>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_post() -> PostResponse {
        PostResponse {
            post_id: 7,
            username: "alice".into(),
            title: Some("Hello & <World>".into()),
            slug: "hello".into(),
            body: "raw".into(),
            format: "markdown".into(),
            rendered_html: "<p>Hi <em>there</em></p>".into(),
            created_at: "2026-01-02T03:04:05Z".into(),
            published_at: Some("2026-01-02T03:04:05Z".into()),
            is_draft: false,
            is_author: false,
            permalink: Some("/~alice/2026/01/02/hello".into()),
            tags: vec![TagSummary {
                slug: "rust".into(),
                display: "Rust".into(),
            }],
            summary: None,
        }
    }

    fn sample_summary() -> TimelinePostSummary {
        TimelinePostSummary {
            post_id: 1,
            username: "bob".into(),
            title: Some("First".into()),
            summary: Some("An excerpt".into()),
            slug: "first".into(),
            rendered_html: "<p>body</p>".into(),
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
            username: "bob".into(),
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
                    username: "bob".into(),
                    page: one_post_page(),
                },
                "<title>Posts by bob</title>",
            ),
            (
                PageSeed::SiteTag {
                    tag: "rust".into(),
                    page: one_post_page(),
                },
                "<title>#rust</title>",
            ),
            (
                PageSeed::UserTag {
                    username: "bob".into(),
                    tag: "rust".into(),
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
            tag: "rust".into(),
            page: one_post_page(),
        });
        assert!(site.contains("#rust"), "{site}");
        assert!(site.contains("First"), "expected post rendered: {site}");

        let user = render_body(&PageSeed::UserTag {
            username: "bob".into(),
            tag: "rust".into(),
            page: one_post_page(),
        });
        assert!(user.contains("#rust by bob"), "{user}");
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
    fn avatar_matches_reactive_component_markup() {
        // Must stay byte-identical to `pages::ui::Avatar` for size 38.
        let (initials, hue) = avatar_parts("Mara Ek");
        assert_eq!(initials, "ME");
        let html = render_avatar("Mara Ek", 38);
        assert_eq!(
            html,
            format!(
                "<div class=\"j-av\" style=\"width:38px;height:38px;background:oklch(0.58 0.07 {hue});font-size:14px\">ME</div>"
            )
        );
    }

    #[test]
    fn tag_list_site_wide_has_hash_chip_and_no_here_link() {
        let tags = [TagSummary {
            slug: "rust".into(),
            display: "Rust".into(),
        }];
        let html = render_tag_list(&tags, &TagCtx::SiteWide);
        assert_eq!(
            html,
            "<span class=\"j-tag-list\"><span class=\"j-tag-cell\">\
             <a class=\"j-tag\" href=\"/tags/rust\">#Rust</a></span></span>"
        );
    }

    #[test]
    fn tag_list_for_user_adds_here_link() {
        let tags = [TagSummary {
            slug: "rust".into(),
            display: "Rust".into(),
        }];
        let html = render_tag_list(&tags, &TagCtx::ForUser("alice".into()));
        assert!(
            html.contains(
                "<a class=\"j-tag-here\" href=\"/~alice/tags/rust\" title=\"On this blog\">"
            ),
            "{html}"
        );
    }

    #[test]
    fn empty_tag_list_renders_nothing() {
        assert_eq!(render_tag_list(&[], &TagCtx::SiteWide), "");
    }

    #[test]
    fn post_content_shows_header_time_for_anon_and_hides_it_for_author() {
        let ctx = TagCtx::SiteWide;
        let mut view = PostView {
            username: "bob",
            title: None,
            banner: None,
            summary: None,
            rendered_html: "<p>b</p>",
            time: "2026-01-01 00:00",
            permalink: "",
            tags: &[],
            tag_ctx: &ctx,
            is_author: false,
        };
        assert!(render_post_content(&view)
            .contains("<span class=\"j-post-time\">2026-01-01 00:00</span>"));
        view.is_author = true;
        assert!(!render_post_content(&view).contains("j-post-time"));
    }

    #[test]
    fn post_content_renders_draft_banner_when_present() {
        let ctx = TagCtx::SiteWide;
        let view = PostView {
            username: "bob",
            title: None,
            banner: Some("Draft - visible only to you"),
            summary: Some("An excerpt"),
            rendered_html: "<p>b</p>",
            time: "2026-01-01 00:00",
            permalink: "",
            tags: &[],
            tag_ctx: &ctx,
            is_author: true,
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
        let view = PostView {
            username: "bob",
            title: Some("T"),
            banner: None,
            summary: None,
            rendered_html: "<p>b</p>",
            time: "2026-01-01 00:00",
            permalink: "/~bob/x",
            tags: &[],
            tag_ctx: &ctx,
            is_author: false,
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
}
