use crate::error::{ErrorClass, ErrorKind, InternalError, WebError};
use crate::tags::TagSummary;
use leptos::context::use_context;
use leptos_axum::ResponseOptions;
use storage::{PostRecord, PostTag};

pub fn timeline_post_summary(
    post: PostRecord,
    viewer_user_id: Option<i64>,
) -> Option<super::TimelinePostSummary> {
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
    Some(super::TimelinePostSummary {
        post_id,
        username: author_username,
        // #402 Task 3 seam: temporary, removed in Task 4 (TimelinePostSummary.title stays String).
        title: title.map(String::from),
        summary,
        slug,
        rendered_html,
        created_at: created_at.to_rfc3339(),
        published_at: published_at.to_rfc3339(),
        permalink,
        is_author: viewer_user_id == Some(user_id),
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

#[must_use]
pub fn post_response(post: PostRecord, is_author: bool) -> super::PostResponse {
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
    super::PostResponse {
        post_id,
        username: author_username,
        // #402 Task 3 seam: temporary, removed in Task 4 (PostResponse fields stay String).
        title: title.map(String::from),
        slug,
        body: String::from(body),
        format: format.to_string(),
        rendered_html,
        created_at: created_at.to_rfc3339(),
        is_draft: published_at.is_none(),
        published_at: published_at.map(|t| t.to_rfc3339()),
        is_author,
        tags: post_tags_to_summaries(tags),
        permalink,
        summary,
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
    fn post_response_carries_summary() {
        use crate::posts::server::post_response;
        use chrono::{TimeZone, Utc};
        use common::{slug::Slug, username::Username};
        use storage::{PostFormat, PostRecord, RenderedHtml};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let author_username = "author".parse::<Username>().unwrap();
        let slug = "hello-world".parse::<Slug>().unwrap();

        let response = post_response(
            PostRecord {
                post_id: 1,
                user_id: 2,
                author_username,
                title: Some("Title".to_string().into()),
                slug,
                body: "body".to_string().into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                created_at: base_time,
                updated_at: base_time,
                published_at: Some(base_time),
                deleted_at: None,
                summary: Some("the summary".into()),
                tags: vec![],
            },
            true,
        );
        assert_eq!(response.summary, Some("the summary".into()));
    }
}
