use crate::error::{ErrorClass, ErrorKind, InternalError, WebError};
use common::ids::UserId;
use common::seed::{AuthoredPost, RenderedPost, TagSummary};
use common::time::UtcInstant;
use leptos::context::use_context;
use leptos_axum::ResponseOptions;
use storage::{PostRecord, PostTag};

/// Build the listing row for a **published** post. `RenderedPost::published_at` is
/// optional, so the bail below — not the type — is what keeps a draft out of a public
/// timeline; `listing::page_from_rows` drops the `None` (see the guard test).
pub fn rendered_post(post: PostRecord, viewer_user_id: Option<UserId>) -> Option<RenderedPost> {
    let published_at = post.published_at?;
    let permalink = post.permalink();
    let PostRecord {
        post_id,
        user_id,
        author_username,
        title,
        summary,
        slug,
        rendered_html,
        created_at,
        tags,
        ..
    } = post;
    Some(RenderedPost {
        post_id,
        username: author_username,
        title,
        summary,
        slug,
        rendered_html,
        created_at: UtcInstant::from(created_at),
        published_at: Some(UtcInstant::from(published_at)),
        permalink: Some(permalink),
        is_author: viewer_user_id == Some(user_id),
        // Only ever built from a published post (the `?` above bails on a draft).
        is_draft: false,
        tags: post_tags_to_summaries(tags),
    })
}

fn post_tags_to_summaries(tags: Vec<PostTag>) -> Vec<TagSummary> {
    tags.into_iter()
        .map(|t| TagSummary {
            slug: t.tag_slug,
            display: t.tag_display,
        })
        .collect()
}

/// Build a permalink post — draft or published — for its author's own surfaces
/// and for the projector's seed.
///
/// Deliberately **not** built on [`rendered_post`]: three of the inner fields are
/// derived differently here. A draft permalink is exactly what this serves, so
/// there is no bail; `is_author` comes from the caller's session check rather
/// than a viewer comparison; `is_draft` follows `published_at`; and the permalink
/// is withheld from a draft, which has no public URL. Routing this through the
/// listing builder would change the seeded draft paint with nothing to point at.
#[must_use]
pub fn authored_post(post: PostRecord, is_author: bool) -> AuthoredPost {
    // Only published posts have a public permalink. For drafts, the permalink is None.
    let permalink = post.published_at.is_some().then(|| post.permalink());
    let PostRecord {
        post_id,
        author_username,
        title,
        slug,
        body,
        format,
        rendered_html,
        created_at,
        published_at,
        summary,
        tags,
        ..
    } = post;
    AuthoredPost {
        post: RenderedPost {
            post_id,
            username: author_username,
            title,
            summary,
            slug,
            rendered_html,
            created_at: UtcInstant::from(created_at),
            is_draft: published_at.is_none(),
            published_at: published_at.map(UtcInstant::from),
            permalink,
            is_author,
            tags: post_tags_to_summaries(tags),
        },
        body,
        format,
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
    #[test]
    fn authored_post_carries_summary_and_source() {
        use crate::posts::server::authored_post;
        use chrono::{TimeZone, Utc};
        use common::test_support::{parse_post_body, parse_post_summary, parse_username};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
        };
        use storage::{PostFormat, PostRecord, RenderedHtml};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let author_username = parse_username("author");
        let slug = "hello-world".parse::<Slug>().unwrap();

        let authored = authored_post(
            PostRecord {
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username,
                title: Some(common::test_support::parse_post_title("Title")),
                slug,
                body: parse_post_body("body"),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                created_at: base_time,
                updated_at: base_time,
                published_at: Some(base_time),
                deleted_at: None,
                summary: Some(parse_post_summary("the summary")),
                tags: vec![],
            },
            true,
        );
        // The rendered half reads through the nesting; the source half is what
        // `AuthoredPost` adds on top of it.
        assert_eq!(
            authored.post.summary,
            Some(parse_post_summary("the summary"))
        );
        assert_eq!(authored.body, "body");
        assert_eq!(authored.format, PostFormat::Markdown);
    }

    // `authored_post` and `rendered_post` build the same twelve inner fields, and
    // three of them diverge: a draft permalink is exactly what `authored_post`
    // serves, so it must not bail, must report `is_draft`, and must withhold the
    // public permalink. Delegating to `rendered_post` (or factoring a shared inner
    // builder) passes every other test and surfaces only as an unexplained
    // ADR-0044 paint diff on the seeded draft permalink — this pins it.
    #[cfg(feature = "server")]
    #[test]
    fn authored_post_leaves_a_draft_published_at_none() {
        use crate::posts::server::authored_post;
        use chrono::{TimeZone, Utc};
        use common::test_support::{parse_post_body, parse_username};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
        };
        use storage::{PostFormat, PostRecord, RenderedHtml};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let author_username = parse_username("author");
        let slug = "unpublished".parse::<Slug>().unwrap();

        let authored = authored_post(
            PostRecord {
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username,
                title: Some(common::test_support::parse_post_title("Title")),
                slug,
                body: parse_post_body("body"),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
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
            authored.post.is_draft,
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
        use chrono::{TimeZone, Utc};
        use common::test_support::{parse_post_body, parse_username};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
        };
        use storage::{PostFormat, PostRecord, RenderedHtml};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let author_username = parse_username("author");
        let slug = "unpublished".parse::<Slug>().unwrap();

        let built = rendered_post(
            PostRecord {
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username,
                title: Some(common::test_support::parse_post_title("Title")),
                slug,
                body: parse_post_body("body"),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                created_at: base_time,
                updated_at: base_time,
                published_at: None,
                deleted_at: None,
                summary: None,
                tags: vec![],
            },
            Some(UserId::from(2)),
        );
        assert!(
            built.is_none(),
            "a draft must never become a public listing row"
        );
    }
}
