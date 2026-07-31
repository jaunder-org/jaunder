//! The timeline vertical's `#[server]` endpoints: the cursor-paginated queries
//! that return a [`TimelinePage`] (a user's posts, the site-wide local timeline,
//! the authenticated home feed, and the two by-tag variants).
//!
//! The wire types they exchange are defined in `common::seed` and re-exported
//! through `crate::posts`; the host-only query helpers these bodies call live in
//! the [`super::server`] leaf. `timeline/mod.rs` is wiring only and re-exports
//! these under the stable `crate::timeline::…` paths that call sites and the
//! server-fn registrar depend on.

use common::seed::TimelinePage;
use common::{ids::PostId, pagination::PageSize, tag::Tag, time::UtcInstant, username::Username};

use crate::error::WebResult;

// Server-only imports for the `#[server]` fn bodies (gated on `feature = "server"`).
#[cfg(feature = "server")]
use {
    super::server::{
        fetch_local_timeline, fetch_posts_by_tag, fetch_user_posts, fetch_user_posts_by_tag,
        page_from_rows,
    },
    crate::auth::require_auth,
    crate::viewer::viewer_identity,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{parse_post_cursor, PostStorage, UserStorage},
};

/// Lists published, non-deleted posts for a user using cursor pagination.
#[macros::server]
pub async fn list_by_user(
    username: Username,
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer_identity().await;
    fetch_user_posts(
        posts.as_ref(),
        &viewer,
        &username,
        cursor_created_at.map(UtcInstant::value),
        cursor_post_id,
        limit,
    )
    .await
}

/// Lists published, non-deleted posts across all users using cursor pagination.
#[macros::server]
pub async fn list_local_timeline(
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer_identity().await;
    fetch_local_timeline(
        posts.as_ref(),
        &viewer,
        cursor_created_at.map(UtcInstant::value),
        cursor_post_id,
        limit,
    )
    .await
}

/// Lists published, non-deleted posts by the authenticated user using cursor pagination.
#[macros::server]
pub async fn list_home_feed(
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    let auth = require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();

    let cursor = parse_post_cursor(cursor_created_at.map(UtcInstant::value), cursor_post_id)?;
    let viewer = viewer_identity().await;
    let page_size = limit.unwrap_or_default();

    let rows = posts
        .list_published_by_user(
            &auth.username,
            cursor.as_ref(),
            page_size.fetch_limit(),
            &viewer,
            chrono::Utc::now(),
        )
        .await?;

    // Was a hand-rolled copy of `page_from_rows` — the second place the has-more
    // rule was spelled out, and the one that could drift from the shared helper.
    Ok(page_from_rows(rows, page_size, Some(auth.user_id)))
}

/// Lists published, non-deleted posts site-wide carrying `tag`.
#[macros::server]
pub async fn list_by_tag(
    tag: Tag,
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer_identity().await;
    fetch_posts_by_tag(
        posts.as_ref(),
        &viewer,
        &tag,
        cursor_created_at.map(UtcInstant::value),
        cursor_post_id,
        limit,
    )
    .await
}

/// Lists published, non-deleted posts by `username` carrying `tag`.
#[macros::server]
pub async fn list_by_user_and_tag(
    username: Username,
    tag: Tag,
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let viewer = viewer_identity().await;
    let cursor = parse_post_cursor(cursor_created_at.map(UtcInstant::value), cursor_post_id)?;
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
}
