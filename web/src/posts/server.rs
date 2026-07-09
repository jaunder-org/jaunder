use crate::error::{ErrorClass, ErrorKind, InternalError, InternalResult, WebError};
use crate::tags::TagSummary;
use chrono::{Datelike, NaiveDate, Utc};
use common::slug::Slug;
use common::username::Username;
use common::visibility::ViewerIdentity;
use leptos::context::use_context;
use leptos_axum::ResponseOptions;
use storage::{
    post_tag_diff, ListByTagError, PerformCreationError, PerformUpdateError, PermalinkDate,
    PostCursor, PostRecord, PostStorage, PostTag,
};

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
        username: author_username.to_string(),
        title,
        summary,
        slug: slug.to_string(),
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
            slug: t.tag_slug.to_string(),
            display: t.tag_display,
        })
        .collect()
}

pub fn list_by_tag_rows(
    result: Result<Vec<PostRecord>, ListByTagError>,
) -> InternalResult<Vec<PostRecord>> {
    match result {
        Ok(rows) => Ok(rows),
        Err(ListByTagError::TagNotFound) => Ok(Vec::new()),
        Err(ListByTagError::Internal(e)) => Err(InternalError::storage(e)),
    }
}

/// Diff the existing tag set against `desired` (a Vec of validated display
/// tokens) and apply the difference: `tag_post` for new entries, `untag_post`
/// for removed entries. Re-applying an existing tag with new display casing
/// is a no-op at the slug level (the storage layer keys on slug); the
/// display casing of the existing row is preserved.
pub async fn apply_post_tag_diff(
    posts: &dyn PostStorage,
    post_id: i64,
    desired: &[String],
) -> InternalResult<()> {
    let existing = posts.get_tags_for_post(post_id).await?;
    let diff = post_tag_diff(&existing, desired);

    for display in diff.to_add {
        posts
            .tag_post(post_id, display)
            .await
            .map_err(|e| InternalError::server_message(e.to_string()))?;
    }
    for slug in diff.to_remove {
        posts
            .untag_post(post_id, slug)
            .await
            .map_err(|e| InternalError::server_message(e.to_string()))?;
    }
    Ok(())
}

pub fn to_post_cursor(post: &PostRecord) -> PostCursor {
    PostCursor {
        created_at: post.created_at,
        post_id: post.post_id,
    }
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
        username: author_username.to_string(),
        title,
        slug: slug.to_string(),
        body,
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

/// The shared public-permalink lookup used by both the `get_post` server fn and
/// the non-reactive public projector.
///
/// Validates the date, then does the visibility-filtered store lookup for
/// `viewer`. The caller maps the record to a `PostResponse` with its own
/// `is_author` (the projector always anonymous → `false`; the server fn derives
/// it from the session), so there is one query and no drift between the two
/// public surfaces.
///
/// # Errors
///
/// Returns a validation error for an impossible calendar date, or a storage
/// error if the permalink lookup fails.
pub async fn fetch_post_record(
    posts: &dyn PostStorage,
    viewer: &ViewerIdentity,
    username: &Username,
    year: i32,
    month: u32,
    day: u32,
    slug: &Slug,
) -> InternalResult<Option<PostRecord>> {
    NaiveDate::from_ymd_opt(year, month, day)
        .ok_or_else(|| InternalError::validation("Invalid permalink"))?;
    posts
        .get_post_by_permalink(
            username,
            PermalinkDate { year, month, day },
            slug,
            viewer,
            Utc::now(),
        )
        .await
        .map_err(InternalError::storage)
}

pub fn parse_post_cursor(
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
) -> InternalResult<Option<PostCursor>> {
    match (cursor_created_at, cursor_post_id) {
        (None, None) => Ok(None),
        (Some(created_at), Some(post_id)) => {
            let created_at = chrono::DateTime::parse_from_rfc3339(created_at.trim())
                .map_err(|e| {
                    InternalError::masked(
                        ErrorKind::Validation,
                        ErrorClass::Client,
                        "invalid cursor_created_at",
                        anyhow::Error::new(e),
                    )
                })?
                .with_timezone(&Utc);
            Ok(Some(PostCursor {
                created_at,
                post_id,
            }))
        }
        _ => Err(InternalError::validation(
            "cursor_created_at and cursor_post_id must be provided together",
        )),
    }
}

pub async fn find_draft_by_permalink_for_user(
    posts: &dyn PostStorage,
    user_id: i64,
    year: i32,
    month: u32,
    day: u32,
    slug: &Slug,
) -> InternalResult<Option<PostRecord>> {
    let mut cursor = None;

    // Search through up to 10,000 drafts (200 pages of 50). This 200-iteration
    // limit is a safety bound to prevent infinite loops or excessive DB load
    // while still being large enough for almost any user's draft list.
    for _ in 0..200 {
        let drafts = posts
            .list_drafts_by_user(user_id, cursor.as_ref(), 50, chrono::Utc::now())
            .await?;
        if drafts.is_empty() {
            return Ok(None);
        }

        let next_cursor = drafts.last().map(to_post_cursor);

        if let Some(found) = drafts.into_iter().find(|post| {
            post.slug == *slug
                && post.created_at.year() == year
                && post.created_at.month() == month
                && post.created_at.day() == day
        }) {
            return Ok(Some(found));
        }

        let Some(next_cursor) = next_cursor else {
            unreachable!("drafts is non-empty after the is_empty guard, so last() is Some")
        };
        cursor = Some(next_cursor);
    }

    Ok(None)
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

pub fn perform_update_error(error: PerformUpdateError) -> InternalError {
    match error {
        PerformUpdateError::EmptyPost | PerformUpdateError::InvalidSlug => {
            InternalError::validation(error.to_string())
        }
        PerformUpdateError::NotFound | PerformUpdateError::Unauthorized => {
            InternalError::not_found("Post")
        }
        PerformUpdateError::Storage(error) => InternalError::storage(error),
    }
}

pub fn perform_creation_error(err: PerformCreationError) -> InternalError {
    match err {
        PerformCreationError::EmptyPost => InternalError::validation("post body is required"),
        PerformCreationError::InvalidSlug(e) => InternalError::validation(e.to_string()),
        PerformCreationError::Exhausted(_) => {
            InternalError::server_message("unable to allocate a unique slug after 100 attempts")
        }
        PerformCreationError::CreatedNotFound => {
            InternalError::server_message("created post not found")
        }
        PerformCreationError::Storage(e) => InternalError::storage(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perform_update_error_maps_each_arm() {
        use crate::error::WebError;
        use storage::PerformUpdateError;

        let empty = perform_update_error(PerformUpdateError::EmptyPost);
        assert!(matches!(
            crate::error::project(empty.kind(), empty.public_message()),
            WebError::Validation { .. }
        ));
        let invalid_slug = perform_update_error(PerformUpdateError::InvalidSlug);
        assert!(matches!(
            crate::error::project(invalid_slug.kind(), invalid_slug.public_message()),
            WebError::Validation { .. }
        ));
        let not_found = perform_update_error(PerformUpdateError::NotFound);
        assert!(matches!(
            crate::error::project(not_found.kind(), not_found.public_message()),
            WebError::NotFound { .. }
        ));
        let unauthorized = perform_update_error(PerformUpdateError::Unauthorized);
        assert!(matches!(
            crate::error::project(unauthorized.kind(), unauthorized.public_message()),
            WebError::NotFound { .. }
        ));
        let storage = perform_update_error(PerformUpdateError::Storage(sqlx::Error::PoolClosed));
        assert!(matches!(
            crate::error::project(storage.kind(), storage.public_message()),
            WebError::Storage { .. }
        ));
    }

    #[test]
    fn perform_creation_error_maps_each_arm() {
        use crate::error::WebError;
        use storage::PerformCreationError;

        let empty = perform_creation_error(PerformCreationError::EmptyPost);
        assert!(matches!(
            crate::error::project(empty.kind(), empty.public_message()),
            WebError::Validation { .. }
        ));
        let invalid_slug =
            perform_creation_error(PerformCreationError::InvalidSlug(common::slug::InvalidSlug));
        assert!(matches!(
            crate::error::project(invalid_slug.kind(), invalid_slug.public_message()),
            WebError::Validation { .. }
        ));
        let exhausted = perform_creation_error(PerformCreationError::Exhausted(5));
        assert!(matches!(
            crate::error::project(exhausted.kind(), exhausted.public_message()),
            WebError::Server { .. }
        ));
        let created_not_found = perform_creation_error(PerformCreationError::CreatedNotFound);
        assert!(matches!(
            crate::error::project(created_not_found.kind(), created_not_found.public_message()),
            WebError::Server { .. }
        ));
        let storage =
            perform_creation_error(PerformCreationError::Storage(sqlx::Error::PoolClosed));
        assert!(matches!(
            crate::error::project(storage.kind(), storage.public_message()),
            WebError::Storage { .. }
        ));
    }

    #[test]
    fn list_by_tag_rows_maps_each_arm() {
        assert!(list_by_tag_rows(Ok(vec![])).is_ok());

        let tag_not_found = list_by_tag_rows(Err(ListByTagError::TagNotFound));
        assert!(matches!(tag_not_found, Ok(rows) if rows.is_empty()));

        let internal = list_by_tag_rows(Err(ListByTagError::Internal(sqlx::Error::PoolClosed)));
        assert!(internal.is_err());
    }

    #[cfg(feature = "server")]
    #[test]
    fn post_response_carries_summary() {
        use crate::posts::server::post_response;
        use chrono::{TimeZone, Utc};
        use common::{slug::Slug, username::Username};
        use storage::{PostFormat, PostRecord};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let author_username = "author".parse::<Username>().unwrap();
        let slug = "hello-world".parse::<Slug>().unwrap();

        let response = post_response(
            PostRecord {
                post_id: 1,
                user_id: 2,
                author_username,
                title: Some("Title".to_string()),
                slug,
                body: "body".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
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

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn find_draft_by_permalink_returns_none_after_exhausting_pages() {
        // guard:no-backend — mock store
        use chrono::{TimeZone, Utc};
        use common::{slug::Slug, username::Username};
        use storage::{MockPostStorage, PostFormat, PostRecord};

        let mut mock = MockPostStorage::new();
        // Every call returns a full 50-row page of drafts whose slug never matches
        // the searched permalink, each row carrying a distinct created_at/post_id so
        // `to_post_cursor` yields an advancing (non-`None`) cursor. Since the page is
        // always non-empty and never matches, all 200 iterations of the safety bound
        // run and the loop falls through to `Ok(None)`.
        mock.expect_list_drafts_by_user()
            .returning(|_user_id, _cursor, _limit, _now| {
                let base = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
                let username = "author".parse::<Username>().unwrap();
                let slug = "other-slug".parse::<Slug>().unwrap();
                let page = (0..50_i64)
                    .map(|i| PostRecord {
                        post_id: i,
                        user_id: 1,
                        author_username: username.clone(),
                        title: None,
                        slug: slug.clone(),
                        body: String::new(),
                        format: PostFormat::Markdown,
                        rendered_html: String::new(),
                        created_at: base + chrono::Duration::seconds(i),
                        updated_at: base,
                        published_at: None,
                        deleted_at: None,
                        summary: None,
                        tags: vec![],
                    })
                    .collect();
                Ok(page)
            });

        let searched = "target-slug".parse::<Slug>().unwrap();
        let result = find_draft_by_permalink_for_user(&mock, 1, 2020, 1, 1, &searched)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    mod sqlite_storage_tests {
        use super::*;

        async fn in_memory_post_storage() -> storage::SqlitePostStorage {
            let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
            sqlx::migrate!("../storage/migrations/sqlite")
                .run(&pool)
                .await
                .unwrap();
            storage::SqlitePostStorage::new(pool)
        }

        #[tokio::test]
        async fn apply_post_tag_diff_skips_invalid_tag_display() {
            let posts = in_memory_post_storage().await;
            // "hello_world" contains an underscore → Tag::from_str fails → continue
            let result = apply_post_tag_diff(&posts, 1, &["hello_world".to_string()]).await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn find_draft_by_permalink_for_user_returns_none_when_no_drafts() {
            let posts = in_memory_post_storage().await;
            let slug: Slug = "my-draft".parse().unwrap();
            let result = find_draft_by_permalink_for_user(&posts, 1, 2026, 5, 22, &slug).await;
            assert!(matches!(result, Ok(None)));
        }
    }
}
