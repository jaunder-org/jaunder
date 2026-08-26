//! The pure post-render twins (ADR-0070 extra host-compiled leaf).
//!
//! These non-reactive, plain-string HTML builders are shared by BOTH the
//! server-side projector (`crate::app::render`, via `render_shell`) and
//! the reactive `PostDisplay` component (`crate::posts::component`): both call the SAME
//! function on the SAME data, so the projector's server-painted post markup and
//! the client's reactive first paint coincide byte-for-byte (flash-free, #181 /
//! ADR-0044). There is deliberately NO leptos reactivity here — plain string
//! building only, like `common::feed`.
//!
//! This leaf is ungated and host-compiled (an "extra leaf" beside
//! `mod`/`api`/`server`/`component`, ADR-0070), so the twins stay host-tested and
//! coverage-measured; the `#[cfg(test)] mod tests` below are the coincidence
//! tests that protect the byte-identical output.

use crate::home::render::render_masthead;
use crate::html::Markup;
use crate::taglist::TagCtx;
use crate::timeline::render::render_load_more;
use crate::{avatar, taglist, topbar};
use common::ids::PostId;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::render::RenderedHtml;
use common::root_relative_url::RootRelativeUrl;
use common::seed::{PageSeed, RenderedPost, TagSummary};
use common::time::UtcInstant;
use common::username::Username;
use maud::html;

/// Formats a [`UtcInstant`] as `"YYYY-MM-DD HH:MM"` (UTC wall-clock). Shared with
/// the reactive components so the projected post time and the reactive re-render
/// coincide.
#[must_use]
pub(crate) fn format_post_time(ts: UtcInstant) -> String {
    ts.value().format("%Y-%m-%d %H:%M").to_string()
}

/// Builds the known-valid edit route for a typed post identifier.
///
/// The literal route shape begins at the site root and `PostId`'s display form
/// is a path-safe integer, so validation cannot fail.
#[must_use]
pub(crate) fn edit_post_url(post_id: PostId) -> RootRelativeUrl {
    let Ok(url) = RootRelativeUrl::try_from(format!("/posts/{post_id}/edit")) else {
        unreachable!("post edit route is root-relative by construction");
    };
    url
}

/// The `<main class="j-main">` inner content for a route — mirrors each reactive
/// page's markup (Topbar + wrappers + posts + load-more) so the seeded first paint
/// coincides. Split from [`crate::app::render_shell`] so the permalink Suspense
/// fallback can reuse just [`permalink_article`].
pub(crate) fn render_body(seed: &PageSeed) -> Markup {
    match seed {
        // Permalink: no Topbar; a single article inside `j-scroll`/`j-page`.
        PageSeed::Permalink(authored) => Markup::new(html! {
            div class="j-scroll" {
                div class="j-page" { (permalink_article(&authored.post)) }
            }
        }),
        // Home (anonymous "Local" mode): the shared masthead as the leading chrome,
        // then the same flush timeline body as the profile/tag pages.
        PageSeed::SiteTimeline(page) => render_timeline_page(
            &render_masthead(),
            &page.posts,
            page.has_more,
            &TagCtx::SiteWide,
            "No posts yet.",
        ),
        PageSeed::Profile { username, page } => render_timeline_page(
            &topbar::render(
                &format!("Posts by {username}"),
                Some("User timeline"),
                &Markup::empty(),
            ),
            &page.posts,
            page.has_more,
            &TagCtx::ForUser(username.clone()),
            "No posts yet.",
        ),
        PageSeed::SiteTag { tag, page } => render_timeline_page(
            &topbar::render(
                &format!("#{tag}"),
                Some("Posts on this instance"),
                &Markup::empty(),
            ),
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
                &Markup::empty(),
            ),
            &page.posts,
            page.has_more,
            &TagCtx::ForUser(username.clone()),
            "No posts with this tag yet.",
        ),
    }
}

/// One permalink post as an `<article>`. Shared by the projector's permalink page
/// and the reactive `PostPage`'s Suspense fallback so they coincide. Takes the
/// rendered half of an `AuthoredPost` so it builds the very same `PostView` as
/// [`render_posts`] — the coincidence is structural, not a copied body.
#[must_use]
pub(crate) fn permalink_article(post: &RenderedPost) -> Markup {
    let ctx = TagCtx::ForUser(post.username.clone());
    render_post_article(&PostView {
        username: &post.username,
        title: post.title.as_ref(),
        banner: None,
        summary: post.summary.as_ref(),
        rendered_html: &post.rendered_html,
        time: &format_post_time(post.display_time()),
        permalink: post.permalink.as_ref(),
        tags: &post.tags,
        tag_ctx: &ctx,
    })
}

/// Concatenated `<article>`s for a list of timeline posts (anonymous, so no action
/// column), each coincident with the reactive `PostCard`/`PostDisplay` output.
#[must_use]
fn render_posts(posts: &[RenderedPost], tag_ctx: &TagCtx) -> Markup {
    Markup::new(html! {
        @for post in posts {
            @let time = format_post_time(post.display_time());
            (render_post_article(&PostView {
                username: &post.username,
                title: post.title.as_ref(),
                banner: None,
                summary: post.summary.as_ref(),
                rendered_html: &post.rendered_html,
                time: &time,
                permalink: post.permalink.as_ref(),
                tags: &post.tags,
                tag_ctx,
            }))
        }
    })
}

/// The fields needed to render one post, borrowed from a `RenderedPost`. `time`
/// is already formatted (see [`format_post_time`]).
pub(crate) struct PostView<'a> {
    pub username: &'a Username,
    pub title: Option<&'a PostTitle>,
    pub banner: Option<&'a str>,
    pub summary: Option<&'a PostSummary>,
    pub rendered_html: &'a RenderedHtml,
    pub time: &'a str,
    pub permalink: Option<&'a RootRelativeUrl>,
    pub tags: &'a [TagSummary],
    pub tag_ctx: &'a TagCtx,
}

/// One post as a full `<article class="j-post">…</article>` — the projector's
/// coincident unit. The reactive `PostDisplay` renders the SAME inner HTML (via
/// [`render_post_inner`] / [`render_post_content`]) so a seeded first paint and
/// the reactive re-render are byte-identical.
#[must_use]
pub(crate) fn render_post_article(view: &PostView) -> Markup {
    Markup::new(html! {
        article class="j-post" { (render_post_inner(view)) }
    })
}

/// The inner HTML of `<article class="j-post">` for the **anonymous** layout:
/// avatar + the content column, with no author-action slot. Mirrors
/// `PostDisplay`'s children when no `children` are passed.
#[must_use]
pub(crate) fn render_post_inner(view: &PostView) -> Markup {
    Markup::new(html! {
        (avatar::render(view.username, 38))
        div style="min-width:0;display:flex;gap:8px;align-items:flex-start" {
            div style="flex:1;min-width:0" { (render_post_content(view)) }
        }
    })
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
pub(crate) fn render_post_content(view: &PostView) -> Markup {
    Markup::new(html! {
        header class="j-post-head" {
            span class="j-post-name" { (view.username) }
            span class="j-post-handle" { "@" (view.username) }
            span class="j-spacer" {}
            span class="j-post-time" { (view.time) }
        }
        @if let Some(title) = view.title {
            div class="j-post-title" {
                @if let Some(permalink) = view.permalink {
                    a href=(&**permalink) { (title) }
                } @else {
                    (title)
                }
            }
        }
        @if let Some(banner) = view.banner {
            p class="draft-banner" { (banner) }
        }
        @if let Some(summary) = view.summary {
            p class="j-post-summary" { (summary) }
        }
        // The one place trusted HTML enters the markup layer. `rendered_html` is a
        // `RenderedHtml`, so the door is the only way to spell this and the
        // `raw-html-door` gate accounts for it.
        div class="j-post-body" { (Markup::from_rendered_html(view.rendered_html)) }
        footer class="j-post-foot" {
            (taglist::render(view.tags, view.tag_ctx))
            span class="j-spacer" {}
        }
    })
}

/// A timeline page's `<main>` content: the given leading `chrome` (a `Topbar`, or
/// home's masthead), then a bare `j-scroll` holding either the empty placeholder
/// or the posts followed by the load-more button — the same flush, wrapper-free
/// structure the shared `TimelineRows` renders, so the projector paint and the
/// reactive `HomePage` / `UserTimelinePage` / `SiteTagPage` / `UserTagPage`
/// coincide (the anonymous `SubscribeButton` renders nothing).
#[must_use]
fn render_timeline_page(
    chrome: &Markup,
    posts: &[RenderedPost],
    has_more: bool,
    tag_ctx: &TagCtx,
    empty_text: &str,
) -> Markup {
    Markup::new(html! {
        (chrome)
        div class="j-scroll" {
            @if posts.is_empty() {
                p { (empty_text) }
            } @else {
                (render_posts(posts, tag_ctx))
                (render_load_more(has_more))
            }
        }
    })
}

/// Shared coincidence-test fixtures. Both this module's tests and the projector's
/// tests in `crate::app::render` build their expectations from these SAME values, so the
/// projector↔reactive byte-coincidence is checked against one definition (a divergent
/// copy would silently make the two suites assert on different posts).
#[cfg(test)]
pub(crate) mod test_fixtures {
    use super::*;
    use common::ids::PostId;
    use common::render::PostFormat;
    use common::seed::{AuthoredPost, TimelinePage};
    use common::test_support::{
        parse_post_body, parse_post_summary, parse_root_relative_url, parse_username,
        parse_utc_instant,
    };

    pub(crate) fn sample_post() -> AuthoredPost {
        AuthoredPost {
            post: RenderedPost {
                post_id: PostId::from(7),
                username: parse_username("alice"),
                title: Some(common::test_support::parse_post_title("Hello & <World>")),
                summary: None,
                slug: "hello".parse().unwrap(),
                rendered_html: RenderedHtml::from_trusted("<p>Hi <em>there</em></p>"),
                created_at: parse_utc_instant("2026-01-02T03:04:05Z"),
                published_at: Some(parse_utc_instant("2026-01-02T03:04:05Z")),
                permalink: Some(parse_root_relative_url("/~alice/2026/01/02/hello")),
                is_author: false,
                is_draft: false,
                tags: vec![TagSummary {
                    slug: "rust".parse().unwrap(),
                    display: "Rust".parse().unwrap(),
                }],
            },
            body: parse_post_body("raw"),
            format: PostFormat::Markdown,
        }
    }

    pub(crate) fn sample_summary() -> RenderedPost {
        RenderedPost {
            post_id: PostId::from(1),
            username: parse_username("bob"),
            title: Some(common::test_support::parse_post_title("First")),
            summary: Some(parse_post_summary("An excerpt")),
            slug: "first".parse().unwrap(),
            rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
            created_at: parse_utc_instant("2026-01-01T00:00:00Z"),
            published_at: Some(parse_utc_instant("2026-01-01T00:00:00Z")),
            permalink: Some(parse_root_relative_url("/~bob/2026/01/01/first")),
            is_author: false,
            is_draft: false,
            tags: vec![],
        }
    }

    pub(crate) fn one_post_page() -> TimelinePage {
        TimelinePage {
            posts: vec![sample_summary()],
            next_cursor: None,
            has_more: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_fixtures::{one_post_page, sample_post, sample_summary};
    use super::*;
    use common::seed::TimelinePage;
    use common::test_support::{
        parse_post_summary, parse_post_title, parse_root_relative_url, parse_username,
        parse_utc_instant,
    };

    #[test]
    fn format_post_time_includes_time_portion() {
        let ts = parse_utc_instant("2026-04-23T10:30:00+00:00");
        assert_eq!(format_post_time(ts), "2026-04-23 10:30");
    }

    #[test]
    fn edit_post_url_composes_known_route() {
        let edit_url = edit_post_url(PostId::from(7));
        let edit_url: &str = &edit_url;
        assert_eq!(edit_url, "/posts/7/edit");
    }

    #[test]
    fn format_post_time_canonicalizes_offset_to_utc() {
        // A non-UTC offset is canonicalized to UTC before formatting: 15:45-05:00
        // is 20:45Z.
        let ts = parse_utc_instant("2026-04-23T15:45:00-05:00");
        assert_eq!(format_post_time(ts), "2026-04-23 20:45");
    }

    #[test]
    fn format_post_time_handles_utc_z_suffix() {
        let ts = parse_utc_instant("2026-04-23T10:30:00Z");
        assert_eq!(format_post_time(ts), "2026-04-23 10:30");
    }

    #[test]
    fn author_content_column_coincides_with_the_anonymous_paint() {
        // #181, ADR-0044 D4/D8: the authed own-post PostDisplay injects
        // render_post_content into the same content <div> the projector's anonymous
        // render_post_inner wraps. render_post_content is viewer-independent, so the
        // authed re-render cannot diverge from the paint — no localized flash.
        let ctx = TagCtx::ForUser(parse_username("alice"));
        let title = parse_post_title("T");
        let author = parse_username("alice");
        let body = RenderedHtml::from_trusted("<p>b</p>");
        let view = PostView {
            username: &author,
            title: Some(&title),
            banner: None,
            summary: None,
            rendered_html: &body,
            time: "2026-01-01 00:00",
            permalink: Some(&parse_root_relative_url("/~alice/x")),
            tags: &[],
            tag_ctx: &ctx,
        };
        let content = render_post_content(&view).into_string();
        // The timestamp stays in the header — the specific divergence #181 fixed. The
        // projector painted it anonymously, so the authed content column must too.
        assert!(
            content.contains("<span class=\"j-post-time\">2026-01-01 00:00</span>"),
            "content column must keep the header time to coincide: {content}"
        );
        // The anonymous inner the projector paints embeds that exact content column
        // verbatim — the authed Some arm injects the identical string.
        assert!(
            render_post_inner(&view).as_str().contains(&content),
            "anonymous inner must embed the identical content column: {content}"
        );
    }

    #[test]
    fn permalink_body_escapes_title_but_injects_rendered_html_raw() {
        let html = render_body(&PageSeed::Permalink(sample_post())).into_string();
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
    fn timeline_renders_each_post_and_heading() {
        let page = TimelinePage {
            posts: vec![sample_summary()],
            next_cursor: None,
            has_more: false,
        };
        let html = render_body(&PageSeed::Profile {
            username: parse_username("bob"),
            page,
        })
        .into_string();
        assert!(html.contains("Posts by bob"), "expected heading: {html}");
        assert!(html.contains("First"), "expected post title: {html}");
        assert!(html.contains("<p>body</p>"), "expected body: {html}");
    }

    #[test]
    fn empty_timeline_renders_placeholder() {
        let page = TimelinePage {
            posts: vec![],
            next_cursor: None,
            has_more: false,
        };
        let html = render_body(&PageSeed::SiteTimeline(page)).into_string();
        assert!(html.contains("No posts yet."), "{html}");
    }

    #[test]
    fn body_covers_tag_page_headings() {
        let site = render_body(&PageSeed::SiteTag {
            tag: "rust".parse().unwrap(),
            page: one_post_page(),
        })
        .into_string();
        // Tag pages render the Topbar (h1 + sub), then a bare j-scroll > posts (flush,
        // no j-page — matches TimelineRows and the home test below).
        assert!(site.contains("<h1>#rust</h1>"), "{site}");
        assert!(site.contains("Posts on this instance"), "{site}");
        assert!(
            site.contains("<div class=\"j-scroll\"><article class=\"j-post\">"),
            "{site}"
        );
        assert!(site.contains("First"), "expected post rendered: {site}");

        let user = render_body(&PageSeed::UserTag {
            username: parse_username("bob"),
            tag: "rust".parse().unwrap(),
            page: one_post_page(),
        })
        .into_string();
        assert!(user.contains("<h1>#rust</h1>"), "{user}");
        assert!(user.contains("Posts by ~bob"), "{user}");
    }

    #[test]
    fn home_local_body_has_topbar_hero_signin_and_posts() {
        let html = render_body(&PageSeed::SiteTimeline(one_post_page())).into_string();
        assert!(html.contains("<h1>jaunder.local</h1>"), "{html}");
        assert!(
            html.contains("<a href=\"/login\" class=\"j-btn j-anon-only\">Sign in</a>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<a href=\"/register\" class=\"j-btn is-primary j-anon-only\">Register</a>"
            ),
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
        let with = render_body(&PageSeed::SiteTimeline(page)).into_string();
        assert!(with.contains("<button>Load more</button>"), "{with}");

        let without = render_body(&PageSeed::SiteTimeline(one_post_page())).into_string();
        assert!(!without.contains("Load more"), "{without}");
    }

    #[test]
    fn timeline_page_empty_states_differ_by_route() {
        let empty = TimelinePage {
            posts: vec![],
            next_cursor: None,
            has_more: false,
        };
        let profile = render_body(&PageSeed::Profile {
            username: parse_username("bob"),
            page: empty.clone(),
        })
        .into_string();
        assert!(profile.contains("<p>No posts yet.</p>"), "{profile}");
        let tag = render_body(&PageSeed::SiteTag {
            tag: "rust".parse().unwrap(),
            page: empty,
        })
        .into_string();
        assert!(tag.contains("<p>No posts with this tag yet.</p>"), "{tag}");
    }

    #[test]
    fn permalink_body_has_no_topbar_and_wraps_article_in_page() {
        let html = render_body(&PageSeed::Permalink(sample_post())).into_string();
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
        post.post.published_at = None;
        post.post.permalink = None;
        let html = render_body(&PageSeed::Permalink(post)).into_string();
        // The time is formatted (mirroring the reactive `format_post_time`), not raw.
        assert!(html.contains("2026-01-02 03:04"), "{html}");
    }

    #[test]
    fn post_content_always_shows_the_header_time() {
        // Viewer-independent (#181, ADR-0044 D4): the timestamp stays in the header
        // for every viewer, so the owner's own-post content column coincides with
        // the projector's anonymous paint (the action column is purely additive).
        let ctx = TagCtx::SiteWide;
        let author = parse_username("bob");
        let body = RenderedHtml::from_trusted("<p>b</p>");
        let view = PostView {
            username: &author,
            title: None,
            banner: None,
            summary: None,
            rendered_html: &body,
            time: "2026-01-01 00:00",
            permalink: None,
            tags: &[],
            tag_ctx: &ctx,
        };
        assert!(
            render_post_content(&view)
                .as_str()
                .contains("<span class=\"j-post-time\">2026-01-01 00:00</span>")
        );
    }

    #[test]
    fn post_content_keeps_title_as_text_without_permalink() {
        let ctx = TagCtx::SiteWide;
        let author = parse_username("bob");
        let body = RenderedHtml::from_trusted("<p>b</p>");
        let title = parse_post_title("Draft title");
        let view = PostView {
            username: &author,
            title: Some(&title),
            banner: None,
            summary: None,
            rendered_html: &body,
            time: "2026-01-01 00:00",
            permalink: None,
            tags: &[],
            tag_ctx: &ctx,
        };
        let html = render_post_content(&view).into_string();
        assert!(
            html.contains("<div class=\"j-post-title\">Draft title</div>"),
            "{html}"
        );
        assert!(!html.contains("<div class=\"j-post-title\"><a "), "{html}");
    }

    #[test]
    fn post_content_renders_draft_banner_when_present() {
        let ctx = TagCtx::SiteWide;
        let author = parse_username("bob");
        let body = RenderedHtml::from_trusted("<p>b</p>");
        let summary = parse_post_summary("An excerpt");
        let view = PostView {
            username: &author,
            title: None,
            banner: Some("Draft - visible only to you"),
            summary: Some(&summary),
            rendered_html: &body,
            time: "2026-01-01 00:00",
            permalink: None,
            tags: &[],
            tag_ctx: &ctx,
        };
        let html = render_post_content(&view).into_string();
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
        let author = parse_username("bob");
        let body = RenderedHtml::from_trusted("<p>b</p>");
        let title = parse_post_title("T");
        let view = PostView {
            username: &author,
            title: Some(&title),
            banner: None,
            summary: None,
            rendered_html: &body,
            time: "2026-01-01 00:00",
            permalink: Some(&parse_root_relative_url("/~bob/x")),
            tags: &[],
            tag_ctx: &ctx,
        };
        let html = render_post_article(&view).into_string();
        assert!(html.starts_with("<article class=\"j-post\">"), "{html}");
        assert!(html.ends_with("</article>"), "{html}");
        // Title links to the permalink, mirroring `PostDisplay`.
        assert!(
            html.contains("<div class=\"j-post-title\"><a href=\"/~bob/x\">T</a></div>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"j-post-time\">2026-01-01 00:00</span>"),
            "{html}"
        );
    }
}
