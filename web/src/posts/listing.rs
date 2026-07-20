//! Timeline / listing post surface: the cursor-paginated `#[server]` endpoints
//! that return [`TimelinePage`]s (user posts, local timeline, home feed, and
//! the by-tag variants), split out from the single-post lifecycle in
//! [`super`]. `#[server]` functions register by their `endpoint` string, not
//! their module path, so this relocation has no routing impact.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use common::{
    ids::PostId, pagination::PageSize, post_title::PostTitle, render::RenderedHtml, slug::Slug,
    tag::Tag, username::Username,
};

use crate::error::WebResult;
use crate::posts::deserialize_rendered_html;
use crate::tags::TagSummary;

#[cfg(feature = "server")]
use {
    super::server::timeline_post_summary,
    crate::auth::require_auth,
    crate::error::{InternalError, InternalResult},
    crate::viewer::viewer_identity,
    common::ids::UserId,
    common::visibility::{viewer_user_id, ViewerIdentity},
    std::sync::Arc,
    storage::{
        list_by_tag_rows, parse_post_cursor, to_post_cursor, PostCursor, PostRecord, PostStorage,
        UserStorage,
    },
};

/// A published post row returned by timeline listing endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelinePostSummary {
    pub post_id: PostId,
    pub username: Username,
    pub title: Option<PostTitle>,
    pub summary: Option<String>,
    pub slug: Slug,
    #[serde(deserialize_with = "deserialize_rendered_html")]
    pub rendered_html: RenderedHtml,
    pub created_at: String,
    pub published_at: String,
    pub permalink: String,
    /// True when the viewing user is the post author.
    pub is_author: bool,
    /// Tags applied to this post, ordered by canonical slug.
    pub tags: Vec<TagSummary>,
}

/// A cursor-paginated page of timeline posts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelinePage {
    pub posts: Vec<TimelinePostSummary>,
    pub next_cursor_created_at: Option<String>,
    pub next_cursor_post_id: Option<PostId>,
    pub has_more: bool,
}

/// Assemble a cursor-paginated [`TimelinePage`] from one over-fetched row set
/// (`page_size + 1` rows detect `has_more`). Shared by every `fetch_*` below.
#[cfg(feature = "server")]
fn page_from_rows(
    mut rows: Vec<PostRecord>,
    page_size: u32,
    viewer_user_id: Option<UserId>,
) -> TimelinePage {
    let has_more = rows.len() > page_size as usize;
    rows.truncate(page_size as usize);
    let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
    let posts = rows
        .into_iter()
        .filter_map(|post| timeline_post_summary(post, viewer_user_id))
        .collect();
    TimelinePage {
        posts,
        next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
        next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
        has_more,
    }
}

/// The shared "posts by user" query, used by both the `list_user_posts` server
/// fn and the public projector (anonymous viewer). One query, no drift.
///
/// # Errors
///
/// Returns a validation error for an unparseable cursor, or a storage error if
/// the listing query fails.
#[cfg(feature = "server")]
pub async fn fetch_user_posts(
    posts: &dyn PostStorage,
    viewer: &ViewerIdentity,
    username: &Username,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> InternalResult<TimelinePage> {
    let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
    let page_size = limit.unwrap_or_default();
    let rows = posts
        .list_published_by_user(
            username,
            cursor.as_ref(),
            page_size.value().saturating_add(1),
            viewer,
            chrono::Utc::now(),
        )
        .await?;
    Ok(page_from_rows(
        rows,
        page_size.value(),
        viewer_user_id(viewer),
    ))
}

/// The shared site-wide timeline query, used by both the `list_local_timeline`
/// server fn and the public projector (anonymous viewer).
///
/// # Errors
///
/// Returns a validation error for an unparseable cursor, or a storage error if
/// the listing query fails.
#[cfg(feature = "server")]
pub async fn fetch_local_timeline(
    posts: &dyn PostStorage,
    viewer: &ViewerIdentity,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> InternalResult<TimelinePage> {
    let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
    let page_size = limit.unwrap_or_default();
    let rows = posts
        .list_published(
            cursor.as_ref(),
            page_size.value().saturating_add(1),
            viewer,
            chrono::Utc::now(),
        )
        .await?;
    Ok(page_from_rows(
        rows,
        page_size.value(),
        viewer_user_id(viewer),
    ))
}

/// Lists published, non-deleted posts for a user using cursor pagination.
#[server(endpoint = "/list_user_posts")]
pub async fn list_user_posts(
    username: Username,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    boundary!("list_user_posts", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let viewer = viewer_identity().await;
        fetch_user_posts(
            posts.as_ref(),
            &viewer,
            &username,
            cursor_created_at,
            cursor_post_id,
            limit,
        )
        .await
    })
}

/// Lists published, non-deleted posts across all users using cursor pagination.
#[server(endpoint = "/list_local_timeline")]
pub async fn list_local_timeline(
    cursor_created_at: Option<String>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    boundary!("list_local_timeline", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let viewer = viewer_identity().await;
        fetch_local_timeline(
            posts.as_ref(),
            &viewer,
            cursor_created_at,
            cursor_post_id,
            limit,
        )
        .await
    })
}

/// Lists published, non-deleted posts by the authenticated user using cursor pagination.
#[server(endpoint = "/list_home_feed")]
pub async fn list_home_feed(
    cursor_created_at: Option<String>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    boundary!("list_home_feed", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let viewer = viewer_identity().await;
        let page_size = limit.unwrap_or_default();
        let fetch_limit = page_size.value().saturating_add(1);

        let mut rows = posts
            .list_published_by_user(
                &auth.username,
                cursor.as_ref(),
                fetch_limit,
                &viewer,
                chrono::Utc::now(),
            )
            .await?;

        let has_more = rows.len() > page_size.value() as usize;
        rows.truncate(page_size.value() as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(post, Some(auth.user_id)))
            .collect();

        Ok(TimelinePage {
            posts,
            next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
            next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
            has_more,
        })
    })
}

/// The shared "posts site-wide carrying a tag" query, used by both the
/// `list_posts_by_tag` server fn and the public projector (anonymous viewer).
///
/// # Errors
///
/// Returns a validation error for an unparseable cursor, or a storage error if
/// the listing query fails.
#[cfg(feature = "server")]
pub async fn fetch_posts_by_tag(
    posts: &dyn PostStorage,
    viewer: &ViewerIdentity,
    tag: &Tag,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> InternalResult<TimelinePage> {
    let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
    let page_size = limit.unwrap_or_default();
    let rows = list_by_tag_rows(
        posts
            .list_posts_by_tag(
                tag,
                cursor.as_ref(),
                page_size.value().saturating_add(1),
                viewer,
                chrono::Utc::now(),
            )
            .await,
    )?;
    Ok(page_from_rows(
        rows,
        page_size.value(),
        viewer_user_id(viewer),
    ))
}

/// The shared "posts by a user carrying a tag" query, used by both the
/// `list_user_posts_by_tag` server fn and the public projector.
///
/// # Errors
///
/// Returns a validation error for an unparseable cursor, a not-found error for
/// an unknown user, or a storage error.
#[cfg(feature = "server")]
pub async fn fetch_user_posts_by_tag(
    posts: &dyn PostStorage,
    users: &dyn UserStorage,
    viewer: &ViewerIdentity,
    username: &Username,
    tag: &Tag,
    cursor: Option<PostCursor>,
    limit: Option<PageSize>,
) -> InternalResult<TimelinePage> {
    let author = users
        .get_user_by_username(username)
        .await?
        .ok_or_else(|| InternalError::not_found("user"))?;
    let page_size = limit.unwrap_or_default();
    let rows = list_by_tag_rows(
        posts
            .list_user_posts_by_tag(
                author.user_id,
                tag,
                cursor.as_ref(),
                page_size.value().saturating_add(1),
                viewer,
                chrono::Utc::now(),
            )
            .await,
    )?;
    Ok(page_from_rows(
        rows,
        page_size.value(),
        viewer_user_id(viewer),
    ))
}

/// Lists published, non-deleted posts site-wide carrying `tag`.
#[server(endpoint = "/list_posts_by_tag")]
pub async fn list_posts_by_tag(
    tag: Tag,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    boundary!("list_posts_by_tag", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let viewer = viewer_identity().await;
        fetch_posts_by_tag(
            posts.as_ref(),
            &viewer,
            &tag,
            cursor_created_at,
            cursor_post_id,
            limit,
        )
        .await
    })
}

/// Lists published, non-deleted posts by `username` carrying `tag`.
#[server(endpoint = "/list_user_posts_by_tag")]
pub async fn list_user_posts_by_tag(
    username: Username,
    tag: Tag,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    boundary!("list_user_posts_by_tag", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();
        let viewer = viewer_identity().await;
        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        fetch_user_posts_by_tag(
            posts.as_ref(),
            users.as_ref(),
            &viewer,
            &username,
            &tag,
            cursor,
            limit,
        )
        .await
    })
}

#[cfg(all(test, feature = "server"))]
mod tests {
    // Helper fns in this feature-gated test module aren't covered by clippy's
    // allow-{unwrap,expect}-in-tests, so allow the test-scaffolding panics.
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{fetch_posts_by_tag, fetch_user_posts_by_tag};
    use common::ids::UserId;
    use common::tag::Tag;
    use common::test_support::parse_username;
    use common::visibility::ViewerIdentity;
    use storage::{ListByTagError, MockPostStorage, MockUserStorage, UserRecord};

    fn user(user_id: UserId, username: &str) -> UserRecord {
        UserRecord {
            user_id,
            username: parse_username(username),
            display_name: None,
            bio: None,
            created_at: chrono::Utc::now(),
            last_authenticated_at: None,
            email: None,
            email_verified: false,
            is_operator: false,
        }
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn fetch_posts_by_tag_propagates_storage_error() {
        let mut posts = MockPostStorage::new();
        posts
            .expect_list_posts_by_tag()
            .returning(|_tag, _cursor, _limit, _viewer, _now| {
                Err(ListByTagError::Internal(sqlx::Error::PoolClosed))
            });
        let result = fetch_posts_by_tag(
            &posts,
            &ViewerIdentity::Anonymous,
            &"rust".parse::<Tag>().unwrap(),
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_err());
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn fetch_user_posts_by_tag_propagates_storage_error() {
        let mut users = MockUserStorage::new();
        users
            .expect_get_user_by_username()
            .returning(|_username| Ok(Some(user(UserId::from(2), "author"))));
        let mut posts = MockPostStorage::new();
        posts.expect_list_user_posts_by_tag().returning(
            |_uid, _tag, _cursor, _limit, _viewer, _now| {
                Err(ListByTagError::Internal(sqlx::Error::PoolClosed))
            },
        );
        let result = fetch_user_posts_by_tag(
            &posts,
            &users,
            &ViewerIdentity::Anonymous,
            &parse_username("author"),
            &"rust".parse::<Tag>().unwrap(),
            None,
            None,
        )
        .await;
        assert!(result.is_err());
    }
}
