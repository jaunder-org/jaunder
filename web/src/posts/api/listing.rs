//! Timeline / listing post surface: the cursor-paginated `#[server]` endpoints
//! that return [`TimelinePage`]s (user posts, local timeline, home feed, and
//! the by-tag variants), split out from the single-post lifecycle in
//! [`super`]. `#[server]` functions register by their `endpoint` string, not
//! their module path, so this relocation has no routing impact.

use leptos::prelude::*;

use common::seed::TimelinePage;
use common::{ids::PostId, pagination::PageSize, tag::Tag, time::UtcInstant, username::Username};

use crate::error::WebResult;

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::{InternalError, InternalResult},
    crate::posts::server::timeline_post_summary,
    crate::viewer::viewer_identity,
    common::ids::UserId,
    common::visibility::{viewer_user_id, ViewerIdentity},
    std::sync::Arc,
    storage::{
        list_by_tag_rows, parse_post_cursor, to_post_cursor, PostCursor, PostRecord, PostStorage,
        UserStorage,
    },
};

/// Assemble a cursor-paginated [`TimelinePage`] from one over-fetched row set
/// (`page_size + 1` rows detect `has_more`). Shared by every `fetch_*` below.
#[cfg(feature = "server")]
fn page_from_rows(
    mut rows: Vec<PostRecord>,
    page_size: PageSize,
    viewer_user_id: Option<UserId>,
) -> TimelinePage {
    // The inverse of `PageSize::fetch_limit`: both halves of the has-more rule live on
    // `PageSize`, so neither is spelled by hand here (#696).
    let has_more = page_size.has_more(rows.len());
    rows.truncate(page_size.page_len());
    let next_cursor = has_more.then(|| rows.last().map(to_post_cursor)).flatten();
    let posts = rows
        .into_iter()
        .filter_map(|post| timeline_post_summary(post, viewer_user_id))
        .collect();
    TimelinePage {
        posts,
        next_cursor_created_at: next_cursor.as_ref().map(|c| UtcInstant::from(c.created_at)),
        next_cursor_post_id: next_cursor.as_ref().map(|c| c.post_id),
        has_more,
    }
}

/// The shared "posts by user" query, used by both the `list_by_user` server
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
    cursor_created_at: Option<chrono::DateTime<chrono::Utc>>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> InternalResult<TimelinePage> {
    let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
    let page_size = limit.unwrap_or_default();
    let rows = posts
        .list_published_by_user(
            username,
            cursor.as_ref(),
            page_size.fetch_limit(),
            viewer,
            chrono::Utc::now(),
        )
        .await?;
    Ok(page_from_rows(rows, page_size, viewer_user_id(viewer)))
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
    cursor_created_at: Option<chrono::DateTime<chrono::Utc>>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> InternalResult<TimelinePage> {
    let cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
    let page_size = limit.unwrap_or_default();
    let rows = posts
        .list_published(
            cursor.as_ref(),
            page_size.fetch_limit(),
            viewer,
            chrono::Utc::now(),
        )
        .await?;
    Ok(page_from_rows(rows, page_size, viewer_user_id(viewer)))
}

/// Lists published, non-deleted posts for a user using cursor pagination.
#[server(endpoint = "/posts/list_by_user")]
#[tracing::instrument(name = "web.posts.list_by_user")]
pub async fn list_by_user(
    username: Username,
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    boundary!({
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
    })
}

/// Lists published, non-deleted posts across all users using cursor pagination.
#[server(endpoint = "/posts/list_local_timeline")]
#[tracing::instrument(name = "web.posts.list_local_timeline")]
pub async fn list_local_timeline(
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    boundary!({
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
    })
}

/// Lists published, non-deleted posts by the authenticated user using cursor pagination.
#[server(endpoint = "/posts/list_home_feed")]
#[tracing::instrument(name = "web.posts.list_home_feed")]
pub async fn list_home_feed(
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    boundary!({
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
    })
}

/// The shared "posts site-wide carrying a tag" query, used by both the
/// `list_by_tag` server fn and the public projector (anonymous viewer).
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
    cursor_created_at: Option<chrono::DateTime<chrono::Utc>>,
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
                page_size.fetch_limit(),
                viewer,
                chrono::Utc::now(),
            )
            .await,
    )?;
    Ok(page_from_rows(rows, page_size, viewer_user_id(viewer)))
}

/// The shared "posts by a user carrying a tag" query, used by both the
/// `list_by_user_and_tag` server fn and the public projector.
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
                page_size.fetch_limit(),
                viewer,
                chrono::Utc::now(),
            )
            .await,
    )?;
    Ok(page_from_rows(rows, page_size, viewer_user_id(viewer)))
}

/// Lists published, non-deleted posts site-wide carrying `tag`.
#[server(endpoint = "/posts/list_by_tag")]
#[tracing::instrument(name = "web.posts.list_by_tag")]
pub async fn list_by_tag(
    tag: Tag,
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    boundary!({
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
    })
}

/// Lists published, non-deleted posts by `username` carrying `tag`.
#[server(endpoint = "/posts/list_by_user_and_tag")]
#[tracing::instrument(name = "web.posts.list_by_user_and_tag")]
pub async fn list_by_user_and_tag(
    username: Username,
    tag: Tag,
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<TimelinePage> {
    boundary!({
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
    })
}

#[cfg(all(test, feature = "server"))]
mod tests {
    // Helper fns in this feature-gated test module aren't covered by clippy's
    // allow-{unwrap,expect}-in-tests, so allow the test-scaffolding panics.
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{
        fetch_local_timeline, fetch_posts_by_tag, fetch_user_posts, fetch_user_posts_by_tag,
    };
    use common::ids::{PostId, UserId};
    use common::pagination::PageSize;
    use common::tag::Tag;
    use common::test_support::{parse_slug, parse_username};
    use common::visibility::ViewerIdentity;
    use storage::{
        ListByTagError, MockPostStorage, MockUserStorage, PostFormat, PostRecord, RenderedHtml,
        UserRecord,
    };

    fn post(post_id: i64) -> PostRecord {
        let now = chrono::Utc::now();
        PostRecord {
            post_id: PostId::from(post_id),
            user_id: UserId::from(1),
            author_username: parse_username("alice"),
            title: None,
            slug: parse_slug("hello-world"),
            body: "body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
            created_at: now,
            updated_at: now,
            published_at: Some(now),
            deleted_at: None,
            summary: None,
            tags: vec![],
        }
    }

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

    /// The has-more convention, end to end at a real call site (#696).
    ///
    /// Asserts both halves against the same `PageSize`: the limit handed to storage is
    /// `fetch_limit()` (the page **plus the probing row**), and the page assembled from
    /// the result reports `has_more` from that row's presence and truncates it away.
    /// The limit assertion is what would catch a call site reverting to hand-rolled
    /// arithmetic — the defect this issue exists to remove.
    // guard:no-backend — mock store
    #[tokio::test]
    async fn fetch_user_posts_over_fetches_by_one_and_reports_has_more() {
        for (returned, expect_more) in [(5usize, false), (6usize, true)] {
            let page_size = PageSize::clamped(5);
            let mut posts = MockPostStorage::new();
            posts
                .expect_list_published_by_user()
                .withf(move |_u, _c, limit, _v, _n| *limit == page_size.fetch_limit())
                .returning(move |_u, _c, _l, _v, _n| {
                    // `try_from(...).unwrap_or` rather than an `as` cast: total, and the
                    // ids only have to be distinct.
                    Ok((0..returned)
                        .map(|i| post(i64::try_from(i).unwrap_or(0) + 1))
                        .collect())
                });

            let page = fetch_user_posts(
                &posts,
                &ViewerIdentity::Anonymous,
                &parse_username("alice"),
                None,
                None,
                Some(page_size),
            )
            .await
            .expect("listing succeeds");

            assert_eq!(page.has_more, expect_more, "has_more for {returned} rows");
            // The probing row never reaches the caller.
            assert_eq!(page.posts.len(), returned.min(page_size.page_len()));
        }
    }

    /// Every paginated fetcher over-fetches by one — not just the one that happened to
    /// get a test.
    ///
    /// `fetch_posts_by_tag` shipped briefly with `exact_limit()` here, which caps the
    /// query at exactly the page and so makes `has_more` permanently `false` — "load
    /// more" silently dies on the site-wide tag feed. Nothing caught it: the test above
    /// covers `fetch_user_posts` only, and the tag fetchers had just an
    /// error-propagation test. Asserting the limit for **each** fetcher is what closes
    /// that gap, and it is cheap because the assertion is the same one.
    // guard:no-backend — mock store
    #[tokio::test]
    async fn every_paginated_fetcher_asks_storage_for_the_probing_row() {
        let page_size = PageSize::clamped(5);
        let expected = page_size.fetch_limit();

        // `fetch_local_timeline` — site-wide timeline.
        let mut posts = MockPostStorage::new();
        posts
            .expect_list_published()
            .withf(move |_c, limit, _v, _n| *limit == expected)
            .returning(|_c, _l, _v, _n| Ok(vec![]));
        fetch_local_timeline(
            &posts,
            &ViewerIdentity::Anonymous,
            None,
            None,
            Some(page_size),
        )
        .await
        .expect("local timeline succeeds");

        // `fetch_posts_by_tag` — site-wide by-tag. This is the site that regressed.
        let mut posts = MockPostStorage::new();
        posts
            .expect_list_posts_by_tag()
            .withf(move |_t, _c, limit, _v, _n| *limit == expected)
            .returning(|_t, _c, _l, _v, _n| Ok(vec![]));
        fetch_posts_by_tag(
            &posts,
            &ViewerIdentity::Anonymous,
            &"rust".parse::<Tag>().expect("valid tag"),
            None,
            None,
            Some(page_size),
        )
        .await
        .expect("by-tag succeeds");
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
