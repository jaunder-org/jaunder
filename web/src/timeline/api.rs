//! The timeline vertical's `#[server]` endpoints: the cursor-paginated queries
//! that return a [`TimelinePage`] (a user's posts, the site-wide local timeline,
//! the authenticated home feed, and the two by-tag variants).
//!
//! The wire types they exchange are defined in `common::seed` and re-exported
//! through `crate::posts`; the host-only query helpers these bodies call live in
//! the [`super::server`] leaf. `timeline/mod.rs` is wiring only and re-exports
//! these under the stable `crate::timeline::…` paths that call sites and the
//! server-fn registrar depend on.
//!
//! Every endpoint here takes the keyset cursor as one bundled [`PageCursor`]
//! rather than a pair of `Option`s, so a half cursor is rejected at arg-decode
//! and no body has one to validate. A nested struct cannot travel through the
//! default form-urlencoded codec, which is why each carries `input = Json` —
//! required, not stylistic.

use common::seed::{PageCursor, TimelinePage};
use common::{pagination::PageSize, tag::Tag, username::Username};
use leptos::server_fn::codec::Json;

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
    storage::{keyset_cursor, PostStorage, UserStorage},
};

/// Lists published, non-deleted posts for a user using cursor pagination.
#[macros::server(input = Json)]
pub async fn list_by_user(
    username: Username,
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer_identity().await;
    fetch_user_posts(
        posts.as_ref(),
        &viewer,
        &username,
        keyset_cursor(cursor),
        limit,
    )
    .await
}

/// Lists published, non-deleted posts across all users using cursor pagination.
#[macros::server(input = Json)]
pub async fn list_local_timeline(
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer_identity().await;
    fetch_local_timeline(posts.as_ref(), &viewer, keyset_cursor(cursor), limit).await
}

/// Lists published, non-deleted posts by the authenticated user using cursor pagination.
#[macros::server(input = Json)]
pub async fn list_home_feed(
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    let auth = require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();

    let cursor = keyset_cursor(cursor);
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
#[macros::server(input = Json)]
pub async fn list_by_tag(
    tag: Tag,
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer_identity().await;
    fetch_posts_by_tag(posts.as_ref(), &viewer, &tag, keyset_cursor(cursor), limit).await
}

/// Lists published, non-deleted posts by `username` carrying `tag`.
#[macros::server(input = Json)]
pub async fn list_by_user_and_tag(
    username: Username,
    tag: Tag,
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let viewer = viewer_identity().await;
    fetch_user_posts_by_tag(
        posts.as_ref(),
        users.as_ref(),
        &viewer,
        &username,
        &tag,
        keyset_cursor(cursor),
        limit,
    )
    .await
}
