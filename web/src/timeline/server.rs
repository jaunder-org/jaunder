//! Host-only support for the timeline vertical: the cursor-paginated storage
//! queries behind the `#[server]` endpoints in [`super::api`].
//!
//! Each `fetch_*` is shared by its server fn and the `server` crate's public
//! projector (anonymous viewer) — one query, no drift — and every one of them
//! assembles its page through [`page_from_rows`], so the over-fetch/has-more
//! rule is spelled exactly once.
//!
//! The cursor arrives already parsed: the endpoints take a bundled
//! [`PageCursor`] and project it with `storage::keyset_cursor`, so no `fetch_*`
//! here has a half-a-cursor case to reject.

use common::ids::UserId;
use common::pagination::PageSize;
use common::seed::TimelinePage;
use common::tag::Tag;
use common::username::Username;
use common::visibility::{ViewerIdentity, viewer_user_id};
use storage::{
    PostCursor, PostRecord, PostStorage, UserStorage, list_by_tag_rows, to_post_cursor, wire_cursor,
};

use crate::error::{InternalError, InternalResult};
use crate::posts::rendered_post;

/// Assemble a cursor-paginated [`TimelinePage`] from one over-fetched row set
/// (`page_size + 1` rows detect `has_more`). Shared by every `fetch_*` below.
pub(super) fn page_from_rows(
    mut rows: Vec<PostRecord>,
    page_size: PageSize,
    viewer_user_id: Option<UserId>,
) -> TimelinePage {
    // The inverse of `PageSize::fetch_limit`: both halves of the has-more rule live on
    // `PageSize`, so neither is spelled by hand here (#696).
    let has_more = page_size.has_more(rows.len());
    rows.truncate(page_size.page_len());
    let next_cursor = has_more
        .then(|| rows.last().map(to_post_cursor))
        .flatten()
        .map(|c| wire_cursor(&c));
    let posts = rows
        .into_iter()
        .filter_map(|post| rendered_post(post, viewer_user_id))
        .collect();
    TimelinePage {
        posts,
        next_cursor,
        has_more,
    }
}

/// The shared "posts by user" query, used by both the `list_by_user` server
/// fn and the public projector (anonymous viewer). One query, no drift.
///
/// # Errors
///
/// Returns a storage error if the listing query fails.
pub async fn fetch_user_posts(
    posts: &dyn PostStorage,
    viewer: &ViewerIdentity,
    username: &Username,
    cursor: Option<PostCursor>,
    limit: Option<PageSize>,
) -> InternalResult<TimelinePage> {
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
/// Returns a storage error if the listing query fails.
pub async fn fetch_local_timeline(
    posts: &dyn PostStorage,
    viewer: &ViewerIdentity,
    cursor: Option<PostCursor>,
    limit: Option<PageSize>,
) -> InternalResult<TimelinePage> {
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

/// The shared "posts site-wide carrying a tag" query, used by both the
/// `list_by_tag` server fn and the public projector (anonymous viewer).
///
/// # Errors
///
/// Returns a storage error if the listing query fails.
pub async fn fetch_posts_by_tag(
    posts: &dyn PostStorage,
    viewer: &ViewerIdentity,
    tag: &Tag,
    cursor: Option<PostCursor>,
    limit: Option<PageSize>,
) -> InternalResult<TimelinePage> {
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
/// Returns a not-found error for an unknown user, or a storage error.
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
    ///
    /// `list_drafts` is guarded by a twin in `super::super`'s test module: it is a
    /// `#[server]` fn rather than a plain fetcher, so it needs a reactive owner and an
    /// authenticated context this module's fetchers do not.
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
        fetch_local_timeline(&posts, &ViewerIdentity::Anonymous, None, Some(page_size))
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
