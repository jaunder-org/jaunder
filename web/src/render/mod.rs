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
        PageSeed::Permalink(post) => post_article(
            &post.username,
            post.title.as_deref(),
            post.summary.as_deref(),
            &post.rendered_html,
            post.published_at.as_deref().unwrap_or(&post.created_at),
            post.permalink.as_deref().unwrap_or_default(),
            &post.tags,
        ),
        PageSeed::SiteTimeline(page) => timeline("Jaunder", &page.posts),
        PageSeed::Profile { username, page } => {
            timeline(&format!("Posts by {username}"), &page.posts)
        }
        PageSeed::SiteTag { tag, page } => timeline(&format!("#{tag}"), &page.posts),
        PageSeed::UserTag {
            username,
            tag,
            page,
        } => timeline(&format!("#{tag} by {username}"), &page.posts),
    }
}

/// One post as an `<article class="j-post">`, mirroring `PostDisplay`.
///
/// `rendered_html` is already-sanitized HTML produced upstream at store time,
/// so it is injected verbatim (exactly as `PostDisplay` does via `inner_html`);
/// every other field is escaped.
fn post_article(
    username: &str,
    title: Option<&str>,
    summary: Option<&str>,
    rendered_html: &str,
    time: &str,
    permalink: &str,
    tags: &[TagSummary],
) -> String {
    let user = escape_html(username);
    let title_html = title.map_or_else(String::new, |t| {
        format!("<div class=\"j-post-title\">{}</div>", escape_html(t))
    });
    let summary_html = summary.map_or_else(String::new, |s| {
        format!("<p class=\"j-post-summary\">{}</p>", escape_html(s))
    });
    format!(
        concat!(
            "<article class=\"j-post\">",
            "<header class=\"j-post-head\">",
            "<span class=\"j-post-name\">{user}</span>",
            "<span class=\"j-post-handle\">@{user}</span>",
            "<span class=\"j-spacer\"></span>",
            "<a class=\"j-post-plink\" href=\"{permalink}\">",
            "<time class=\"j-post-time\">{time}</time></a>",
            "</header>",
            "{title_html}{summary_html}",
            "<div class=\"j-post-body\">{body}</div>",
            "<footer class=\"j-post-foot\">{tags}</footer>",
            "</article>",
        ),
        user = user,
        permalink = escape_html(permalink),
        time = escape_html(time),
        title_html = title_html,
        summary_html = summary_html,
        body = rendered_html,
        tags = tag_list(tags),
    )
}

/// A list of post summaries as a timeline page.
fn timeline(heading: &str, posts: &[TimelinePostSummary]) -> String {
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
        out.push_str(&post_article(
            &post.username,
            post.title.as_deref(),
            post.summary.as_deref(),
            &post.rendered_html,
            &post.published_at,
            &post.permalink,
            &post.tags,
        ));
    }
    out.push_str("</div>");
    out
}

/// The footer tag chips, linking to each tag's site-wide page.
fn tag_list(tags: &[TagSummary]) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let mut out = String::from("<ul class=\"j-post-tags\">");
    for tag in tags {
        let _ = write!(
            out,
            "<li><a class=\"j-tag\" href=\"/tags/{slug}\">{display}</a></li>",
            slug = escape_html(&tag.slug),
            display = escape_html(&tag.display),
        );
    }
    out.push_str("</ul>");
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
        assert!(html.contains("2026-01-02T03:04:05Z"), "{html}");
    }

    #[test]
    fn page_seed_round_trips_through_json() {
        let seed = PageSeed::Permalink(sample_post());
        let json = serde_json::to_string(&seed).unwrap();
        let back: PageSeed = serde_json::from_str(&json).unwrap();
        assert_eq!(seed, back);
    }
}
