//! Timeline / listing post surface: the cursor-paginated `#[server]` endpoints
//! that return [`TimelinePage`]s (user posts, local timeline, home feed, and
//! the by-tag variants), split out from the single-post lifecycle in
//! [`super`]. `#[server]` functions register by their `endpoint` string, not
//! their module path, so this relocation has no routing impact.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::WebResult;
use crate::tags::TagSummary;

#[cfg(feature = "ssr")]
use {
    super::server::{list_by_tag_rows, parse_post_cursor, timeline_post_summary, to_post_cursor},
    crate::auth::require_auth,
    crate::error::InternalError,
    crate::viewer::{viewer_identity, viewer_user_id},
    common::{tag::Tag, username::Username},
    std::sync::Arc,
    storage::{PostStorage, UserStorage},
};

/// A published post row returned by timeline listing endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelinePostSummary {
    pub post_id: i64,
    pub username: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub slug: String,
    pub rendered_html: String,
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
    pub next_cursor_post_id: Option<i64>,
    pub has_more: bool,
}

/// Lists published, non-deleted posts for a user using cursor pagination.
#[server(endpoint = "/list_user_posts")]
pub async fn list_user_posts(
    username: String,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<TimelinePage> {
    boundary!("list_user_posts", {
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let username = username
            .trim()
            .parse::<Username>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;

        let viewer = viewer_identity().await;
        let viewer_user_id = viewer_user_id(&viewer);

        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let mut rows = posts
            .list_published_by_user(
                &username,
                cursor.as_ref(),
                fetch_limit,
                &viewer,
                chrono::Utc::now(),
            )
            .await
            .map_err(InternalError::storage)?;

        let has_more = rows.len() > page_size as usize;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();

        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(post, viewer_user_id))
            .collect();

        Ok(TimelinePage {
            posts,
            next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
            next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
            has_more,
        })
    })
}

/// Lists published, non-deleted posts across all users using cursor pagination.
#[server(endpoint = "/list_local_timeline")]
pub async fn list_local_timeline(
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<TimelinePage> {
    boundary!("list_local_timeline", {
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let viewer = viewer_identity().await;
        let viewer_user_id = viewer_user_id(&viewer);

        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let mut rows = posts
            .list_published(cursor.as_ref(), fetch_limit, &viewer, chrono::Utc::now())
            .await
            .map_err(InternalError::storage)?;

        let has_more = rows.len() > page_size as usize;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(post, viewer_user_id))
            .collect();

        Ok(TimelinePage {
            posts,
            next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
            next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
            has_more,
        })
    })
}

/// Lists published, non-deleted posts by the authenticated user using cursor pagination.
#[server(endpoint = "/list_home_feed")]
pub async fn list_home_feed(
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<TimelinePage> {
    boundary!("list_home_feed", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let viewer = viewer_identity().await;
        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let mut rows = posts
            .list_published_by_user(
                &auth.username,
                cursor.as_ref(),
                fetch_limit,
                &viewer,
                chrono::Utc::now(),
            )
            .await
            .map_err(InternalError::storage)?;

        let has_more = rows.len() > page_size as usize;
        rows.truncate(page_size as usize);

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

/// Lists published, non-deleted posts site-wide carrying `tag`.
#[server(endpoint = "/list_posts_by_tag")]
pub async fn list_posts_by_tag(
    tag: String,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<TimelinePage> {
    boundary!("list_posts_by_tag", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let tag_slug = tag
            .trim()
            .parse::<Tag>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let viewer = viewer_identity().await;
        let viewer_user_id = viewer_user_id(&viewer);

        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let rows = list_by_tag_rows(
            posts
                .list_posts_by_tag(
                    &tag_slug,
                    cursor.as_ref(),
                    fetch_limit,
                    &viewer,
                    chrono::Utc::now(),
                )
                .await,
        )?;

        let has_more = rows.len() > page_size as usize;
        let mut rows = rows;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(post, viewer_user_id))
            .collect();

        Ok(TimelinePage {
            posts,
            next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
            next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
            has_more,
        })
    })
}

/// Lists published, non-deleted posts by `username` carrying `tag`.
#[server(endpoint = "/list_user_posts_by_tag")]
pub async fn list_user_posts_by_tag(
    username: String,
    tag: String,
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<TimelinePage> {
    boundary!("list_user_posts_by_tag", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();
        let username = username
            .trim()
            .parse::<Username>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let tag_slug = tag
            .trim()
            .parse::<Tag>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let viewer = viewer_identity().await;
        let viewer_user_id = viewer_user_id(&viewer);

        let author = users
            .get_user_by_username(&username)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("user"))?;

        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let fetch_limit = page_size.saturating_add(1);

        let rows = list_by_tag_rows(
            posts
                .list_user_posts_by_tag(
                    author.user_id,
                    &tag_slug,
                    cursor.as_ref(),
                    fetch_limit,
                    &viewer,
                    chrono::Utc::now(),
                )
                .await,
        )?;

        let has_more = rows.len() > page_size as usize;
        let mut rows = rows;
        rows.truncate(page_size as usize);

        let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
        let posts = rows
            .into_iter()
            .filter_map(|post| timeline_post_summary(post, viewer_user_id))
            .collect();

        Ok(TimelinePage {
            posts,
            next_cursor_created_at: next_cursor.as_ref().map(|c| c.created_at.to_rfc3339()),
            next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
            has_more,
        })
    })
}
