use crate::error::{ErrorClass, ErrorKind, InternalError, WebError};
use common::ids::UserId;
use common::post_summary::PostSummary;
use common::seed::{AuthoredPost, RenderedPost, TagSummary};
use leptos::context::use_context;
use leptos_axum::ResponseOptions;
use storage::{PostRecord, PostTag, PublicPresentationPostRecord};

/// Build the listing row for a **published** post. `RenderedPost::published_at` is
/// optional, so the bail below — not the type — is what keeps a draft out of a public
/// timeline; `listing::page_from_rows` drops the `None` (see the guard test).
#[must_use]
pub fn rendered_post(
    post: PublicPresentationPostRecord,
    viewer_user_id: Option<UserId>,
) -> Option<RenderedPost> {
    post.published_at?;
    let is_author = viewer_user_id == Some(post.user_id);
    let permalink = Some(post.permalink());
    Some(rendered_post_from_record(
        post.post,
        is_author,
        permalink,
        Some(post.content_license),
    ))
}

/// Build one published Home row without public Content Rights presentation.
#[must_use]
pub fn rendered_home_post(
    post: PostRecord,
    viewer_user_id: Option<UserId>,
) -> Option<RenderedPost> {
    post.published_at?;
    let is_author = viewer_user_id == Some(post.user_id);
    let permalink = Some(post.permalink());
    Some(rendered_post_from_record(post, is_author, permalink, None))
}

/// Translates one storage record into the shared rendered-post projection.
///
/// Timeline rows call this directly and therefore do not request the host-only
/// effective-summary projection reserved for permalink metadata.
fn rendered_post_from_record(
    post: PostRecord,
    is_author: bool,
    permalink: Option<common::root_relative_url::RootRelativeUrl>,
    content_license: Option<common::content_license::ContentLicense>,
) -> RenderedPost {
    let PostRecord {
        post_id,
        author_username,
        author_display_name,
        rendered_title,
        slug,
        rendered_html,
        created_at,
        published_at,
        summary,
        tags,
        ..
    } = post;
    RenderedPost {
        post_id,
        username: author_username,
        display_name: author_display_name,
        content_license,
        rendered_title,
        summary,
        slug,
        rendered_html,
        created_at,
        published_at,
        permalink,
        is_author,
        tags: post_tags_to_summaries(tags),
    }
}

fn post_tags_to_summaries(tags: Vec<PostTag>) -> Vec<TagSummary> {
    tags.into_iter()
        .map(|t| TagSummary {
            slug: t.tag_slug,
            display: t.tag_display,
        })
        .collect()
}

/// Selects authored summary before deriving presentation metadata from rendered HTML.
///
/// This host-only seam preserves `RenderedPost.summary` as authored content while
/// making the fallback available only to surfaces that explicitly request it.
#[must_use]
pub fn effective_summary(post: &PostRecord) -> Option<PostSummary> {
    post.summary
        .clone()
        .or_else(|| host::render::summarize_rendered_html(&post.rendered_html))
}

/// Build a permalink post — draft or published — for its author's own surfaces
/// and for the projector's seed.
///
/// This function owns the shared `PostRecord` to `RenderedPost` translation.
/// Drafts remain valid here: `is_author` comes from the caller's session check,
/// and the permalink is withheld from a draft, which has no public URL.
#[must_use]
pub fn authored_post(post: PostRecord, is_author: bool) -> AuthoredPost {
    authored_post_from_record(post, is_author, None)
}

/// Builds a public permalink projection with the author's current Content License.
#[must_use]
pub fn public_authored_post(post: PublicPresentationPostRecord, is_author: bool) -> AuthoredPost {
    authored_post_from_record(post.post, is_author, Some(post.content_license))
}

fn authored_post_from_record(
    post: PostRecord,
    is_author: bool,
    content_license: Option<common::content_license::ContentLicense>,
) -> AuthoredPost {
    // Only published posts have a public permalink. For drafts, the permalink is None.
    let permalink = post.published_at.is_some().then(|| post.permalink());
    // Metadata gets the effective projection; the rendered row below deliberately
    // keeps the authored summary untouched for public and AtomPub parity.
    let permalink_description = effective_summary(&post);
    let title = post.title.clone();
    let body = post.body.clone();
    let format = post.format;
    AuthoredPost {
        post: rendered_post_from_record(post, is_author, permalink, content_license),
        title,
        body,
        format,
        permalink_description,
    }
}

pub fn not_found_error() -> InternalError {
    set_not_found_status();
    InternalError::not_found("Post")
}

fn set_not_found_status() {
    if let Some(opts) = use_context::<ResponseOptions>() {
        opts.set_status(axum::http::StatusCode::NOT_FOUND);
    }
}

/// Masks a private/unauthorized post as a 404 instead of a 403: a distinct
/// "forbidden" would confirm the post exists to a viewer not allowed to see it,
/// leaking its existence. Fail closed to an indistinguishable not-found while
/// preserving the real cause in the operator message.
pub fn private_post_not_found_error(error: &InternalError) -> InternalError {
    set_not_found_status();
    InternalError::masked(
        ErrorKind::NotFound,
        ErrorClass::Client,
        WebError::not_found("Post").to_string(),
        anyhow::Error::msg(format!(
            "private post hidden behind not-found response: {}",
            error.operator_message()
        )),
    )
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "server")]
    fn public_post(post: storage::PostRecord) -> storage::PublicPresentationPostRecord {
        storage::PublicPresentationPostRecord {
            post,
            content_license: common::content_license::ContentLicense::AllRightsReserved,
        }
    }

    #[cfg(feature = "server")]
    #[test]
    fn authored_post_carries_summary_and_source() {
        use crate::posts::server::authored_post;
        use common::test_support::{
            parse_display_name, parse_post_body, parse_post_summary, parse_username,
        };
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
            time::UtcInstant,
        };
        use storage::{PostFormat, PostRecord};

        let base_time: UtcInstant = "2026-04-16T10:11:12Z".parse().unwrap();
        let author_username = parse_username("author");
        let slug = "hello-world".parse::<Slug>().unwrap();

        let authored = authored_post(
            PostRecord {
                author_display_name: Some(parse_display_name("Ada Lovelace")),
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username,
                title: Some(common::test_support::parse_post_title("Title")),
                rendered_title: Some(common::test_support::rendered_post_title("Title")),
                slug,
                body: parse_post_body("body"),
                format: PostFormat::Markdown,
                rendered_html: common::test_support::rendered_html("<p>body</p>"),
                created_at: base_time,
                updated_at: base_time,
                published_at: Some(base_time),
                deleted_at: None,
                summary: Some(parse_post_summary("the\n\nsummary\u{a0}  wins")),
                tags: vec![],
            },
            true,
        );
        // The rendered half reads through the nesting; the source half is what
        // `AuthoredPost` adds on top of it.
        assert_eq!(
            authored.post.summary,
            Some(parse_post_summary("the\n\nsummary\u{a0}  wins"))
        );
        assert_eq!(
            authored.permalink_description,
            Some(parse_post_summary("the\n\nsummary\u{a0}  wins")),
            "authored permalink metadata remains unchanged"
        );
        assert_eq!(
            authored.post.display_name,
            Some(parse_display_name("Ada Lovelace"))
        );
        assert_eq!(authored.body, "body");
        assert_eq!(authored.format, PostFormat::Markdown);
    }

    #[cfg(feature = "server")]
    #[test]
    fn effective_summary_derives_from_rendered_html_when_authored_summary_is_absent() {
        use crate::posts::server::effective_summary;
        use common::test_support::{parse_post_body, parse_username};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
            time::UtcInstant,
        };
        use storage::{PostFormat, PostRecord};

        let time: UtcInstant = "2026-04-16T10:11:12Z".parse().unwrap();
        let summary = effective_summary(&PostRecord {
            author_display_name: None,
            post_id: PostId::from(1),
            user_id: UserId::from(2),
            author_username: parse_username("author"),
            title: None,
            rendered_title: None,
            slug: "rendered-summary".parse::<Slug>().unwrap(),
            body: parse_post_body("Legacy *markup* summary."),
            format: PostFormat::Markdown,
            rendered_html: common::test_support::rendered_html(
                "<p>Legacy <em>markup</em> summary.</p>",
            ),
            created_at: time,
            updated_at: time,
            published_at: None,
            deleted_at: None,
            summary: None,
            tags: vec![],
        });

        assert_eq!(summary.as_deref(), Some("Legacy markup summary."));
    }

    // `authored_post` and `rendered_post` build the same eleven inner fields, and
    // two of them diverge: a draft permalink is exactly what `authored_post`
    // serves, so it must not bail and must withhold the public permalink.
    // Delegating to `rendered_post` (or factoring a shared inner builder) passes every
    // other test and surfaces only as an unexplained ADR-0044 paint diff on the seeded
    // draft permalink — this pins it.
    #[cfg(feature = "server")]
    #[test]
    fn authored_post_leaves_a_draft_published_at_none() {
        use crate::posts::server::authored_post;
        use common::test_support::{parse_post_body, parse_username};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
            time::UtcInstant,
        };
        use storage::{PostFormat, PostRecord};

        let base_time: UtcInstant = "2026-04-16T10:11:12Z".parse().unwrap();
        let author_username = parse_username("author");
        let slug = "unpublished".parse::<Slug>().unwrap();

        let authored = authored_post(
            PostRecord {
                author_display_name: None,
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username,
                title: Some(common::test_support::parse_post_title("Title")),
                rendered_title: Some(common::test_support::rendered_post_title("Title")),
                slug,
                body: parse_post_body("body"),
                format: PostFormat::Markdown,
                rendered_html: common::test_support::rendered_html("<p>body</p>"),
                created_at: base_time,
                updated_at: base_time,
                published_at: None,
                deleted_at: None,
                summary: None,
                tags: vec![],
            },
            true,
        );
        assert_eq!(
            authored.post.published_at, None,
            "a draft has no publication instant"
        );
        assert!(
            authored.post.is_draft(),
            "a draft permalink must paint its draft banner"
        );
        assert_eq!(
            authored.post.permalink, None,
            "a draft has no public permalink"
        );
    }

    // `RenderedPost::published_at` is `Option`, so nothing type-level stops the
    // builder from happily rendering a draft — only the `post.published_at?` bail
    // does, and `listing::page_from_rows`'s `filter_map` is what turns that `None`
    // into "omitted from the page". Pin the bail here: making the builder infallible
    // would publish every draft into the public timelines.
    #[cfg(feature = "server")]
    #[test]
    fn rendered_post_refuses_a_draft() {
        use crate::posts::server::rendered_post;
        use common::test_support::{parse_post_body, parse_username};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
            time::UtcInstant,
        };
        use storage::{PostFormat, PostRecord};

        let base_time: UtcInstant = "2026-04-16T10:11:12Z".parse().unwrap();
        let author_username = parse_username("author");
        let slug = "unpublished".parse::<Slug>().unwrap();

        let built = rendered_post(
            public_post(PostRecord {
                author_display_name: None,
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username,
                title: Some(common::test_support::parse_post_title("Title")),
                rendered_title: Some(common::test_support::rendered_post_title("Title")),
                slug,
                body: parse_post_body("body"),
                format: PostFormat::Markdown,
                rendered_html: common::test_support::rendered_html("<p>body</p>"),
                created_at: base_time,
                updated_at: base_time,
                published_at: None,
                deleted_at: None,
                summary: None,
                tags: vec![],
            }),
            Some(UserId::from(2)),
        );
        assert!(
            built.is_none(),
            "a draft must never become a public listing row"
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn public_rendered_post_keeps_derived_summary_absent() {
        use crate::posts::server::rendered_post;
        use common::test_support::{parse_post_body, parse_username};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
            time::UtcInstant,
        };
        use storage::{PostFormat, PostRecord};

        let time: UtcInstant = "2026-04-16T10:11:12Z".parse().unwrap();
        let timeline_post = rendered_post(
            public_post(PostRecord {
                author_display_name: None,
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username: parse_username("author"),
                title: None,
                rendered_title: None,
                slug: "timeline".parse::<Slug>().unwrap(),
                body: parse_post_body("source body"),
                format: PostFormat::Markdown,
                rendered_html: common::test_support::rendered_html("<p>derived text</p>"),
                created_at: time,
                updated_at: time,
                published_at: Some(time),
                deleted_at: None,
                summary: None,
                tags: vec![],
            }),
            None,
        )
        .expect("published records build timeline rows");

        // TDD: public Post summaries retain authored-only semantics; the rendered
        // text may serve explicit metadata projections but must not become a paragraph.
        assert_eq!(timeline_post.summary, None);
        assert_eq!(
            timeline_post.content_license,
            Some(common::content_license::ContentLicense::AllRightsReserved),
            "public projections carry the wrapper's current Content License"
        );
    }
}
