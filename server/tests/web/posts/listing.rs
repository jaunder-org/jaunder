use std::sync::Arc;

use axum::http::StatusCode;
use common::seed::{Page, PublicPresentation, RenderedPost};
use common::tag::TagLabel;
use common::test_support::{parse_post_body, parse_tag_label};
use common::theme::Theme;
use server_fn::ServerFn;
use storage::PostFormat;
use web::posts::{PostInputs, SavedPost, UnpublishedPost};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{
    confirmed_mutation, create_post_json, create_session_for, create_user_and_session, post_form,
    post_json,
};
use storage::test_support::{Backend, SeedRawPost, SeedUser, TestEnv, backends, backends_matrix};

use super::fixtures::{
    list_drafts, list_home_feed, list_local_timeline, list_scheduled, list_user_posts,
};

async fn list_posts_by_tag(
    state: &Arc<storage::AppState>,
    tag: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        state,
        <web::timeline::ListByTag as ServerFn>::PATH,
        serde_json::json!({ "tag": tag, "cursor": null, "limit": 50 }),
        cookie,
    )
    .await
}

async fn list_user_posts_by_tag(
    state: &Arc<storage::AppState>,
    username: &str,
    tag: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        state,
        <web::timeline::ListByUserAndTag as ServerFn>::PATH,
        serde_json::json!({ "username": username, "tag": tag, "cursor": null, "limit": 50 }),
        cookie,
    )
    .await
}

#[apply(backends)]
#[tokio::test]
async fn list_drafts_returns_current_user_drafts_with_cursor_pagination(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_cookie = create_user_and_session(&state).await.cookie();
    let stranger_cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("first"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let first_draft: SavedPost = confirmed_mutation(&body);

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("second"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let second_draft: SavedPost = confirmed_mutation(&body);

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(parse_post_body("visible"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("private"), PostFormat::Markdown)
        },
        Some(&stranger_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = list_drafts(&state, None, 1, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Page<UnpublishedPost> = serde_json::from_str(&body).unwrap();
    assert_eq!(first_page.posts.len(), 1, "body: {body}");
    let first_entry = &first_page.posts[0];
    assert!(
        first_entry.post.post_id == first_draft.post_id
            || first_entry.post.post_id == second_draft.post_id,
        "unexpected post_id on first page: {body}"
    );

    // The page itself carries where the next one starts, so the client never
    // reassembles a cursor from row fields.
    assert!(first_page.has_more, "two drafts, page of 1: {body}");
    let cursor = first_page
        .next_cursor
        .expect("page 1 has more, so it carries a cursor");

    let (status, body) = list_drafts(&state, Some(cursor), 10, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: Page<UnpublishedPost> = serde_json::from_str(&body).unwrap();
    assert_eq!(second_page.posts.len(), 1, "body: {body}");
    assert!(!second_page.has_more, "the tail page ends here: {body}");
    let second_entry = &second_page.posts[0];

    assert_ne!(first_entry.post.post_id, second_entry.post.post_id);
    let mut ids = vec![first_entry.post.post_id, second_entry.post.post_id];
    ids.sort_unstable_by_key(|id| i64::from(*id));
    let mut expected_ids = vec![first_draft.post_id, second_draft.post_id];
    expected_ids.sort_unstable_by_key(|id| i64::from(*id));
    assert_eq!(ids, expected_ids);
}

// A future-scheduled post is surfaced through `list_drafts` with a populated
// `published_at`, while a live post stays off the drafts surface (issue #70).
#[apply(backends)]
#[tokio::test]
async fn list_drafts_surfaces_scheduled_with_marker_excludes_live(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;

    // Seed a scheduled post (future `published_at`) and a live post (past)
    // directly via storage — the web compose datetime control is Task 6.
    let now = chrono::Utc::now();
    let sched_id = SeedRawPost::new(author.user_id)
        .published_at(common::time::UtcInstant::from(
            now + chrono::Duration::days(3),
        ))
        .seed(&state)
        .await
        .post_id;
    let live_id = SeedRawPost::new(author.user_id)
        .published_at(common::time::UtcInstant::from(
            now - chrono::Duration::days(1),
        ))
        .seed(&state)
        .await
        .post_id;

    let (status, body) = list_drafts(&state, None, 50, Some(&author.cookie())).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let drafts: Page<UnpublishedPost> = serde_json::from_str(&body).unwrap();

    let sched = drafts
        .posts
        .iter()
        .find(|d| d.post.post_id == sched_id)
        .unwrap_or_else(|| panic!("scheduled post must appear in drafts: {body}"));
    assert!(
        sched.post.published_at.is_some(),
        "scheduled post must carry published_at: {body}"
    );
    assert!(
        !drafts.posts.iter().any(|d| d.post.post_id == live_id),
        "live post must not appear in drafts: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_scheduled_returns_current_user_future_posts_ordered_by_schedule(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let stranger = create_user_and_session(&state).await;
    let author_cookie = author.cookie();

    let now = chrono::Utc::now();
    let same_time = now + chrono::Duration::days(3);

    let draft_id = SeedRawPost::new(author.user_id)
        .draft()
        .seed(&state)
        .await
        .post_id;
    let live_id = SeedRawPost::new(author.user_id)
        .published_at(common::time::UtcInstant::from(
            now - chrono::Duration::days(1),
        ))
        .seed(&state)
        .await
        .post_id;
    let deleted_id = SeedRawPost::new(author.user_id)
        .published_at(common::time::UtcInstant::from(
            now + chrono::Duration::days(2),
        ))
        .seed(&state)
        .await
        .post_id;
    let posts = Arc::clone(&state.posts);
    state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                posts
                    .soft_delete_post(
                        transaction,
                        deleted_id,
                        author.user_id,
                        common::time::UtcInstant::now(),
                    )
                    .await
            })
        })
        .await
        .unwrap();
    let other_id = SeedRawPost::new(stranger.user_id)
        .published_at(common::time::UtcInstant::from(
            now + chrono::Duration::days(1),
        ))
        .seed(&state)
        .await
        .post_id;

    let earlier_id = SeedRawPost::new(author.user_id)
        .published_at(common::time::UtcInstant::from(
            now + chrono::Duration::days(1),
        ))
        .seed(&state)
        .await
        .post_id;
    let same_a_id = SeedRawPost::new(author.user_id)
        .published_at(common::time::UtcInstant::from(same_time))
        .seed(&state)
        .await
        .post_id;
    let same_b_id = SeedRawPost::new(author.user_id)
        .published_at(common::time::UtcInstant::from(same_time))
        .seed(&state)
        .await
        .post_id;
    let later_id = SeedRawPost::new(author.user_id)
        .published_at(common::time::UtcInstant::from(
            now + chrono::Duration::days(5),
        ))
        .seed(&state)
        .await
        .post_id;

    let (status, body) = list_scheduled(&state, None, 2, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Page<UnpublishedPost> = serde_json::from_str(&body).unwrap();
    assert_eq!(first_page.posts.len(), 2, "body: {body}");
    assert!(first_page.has_more, "body: {body}");
    let cursor = first_page
        .next_cursor
        .expect("page 1 has more, so it carries a cursor");

    let (status, body) = list_scheduled(&state, Some(cursor), 10, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: Page<UnpublishedPost> = serde_json::from_str(&body).unwrap();
    assert_eq!(second_page.posts.len(), 2, "body: {body}");
    assert!(!second_page.has_more, "body: {body}");
    assert!(second_page.next_cursor.is_none(), "body: {body}");

    let ids: Vec<_> = first_page
        .posts
        .iter()
        .chain(second_page.posts.iter())
        .map(|row| row.post.post_id)
        .collect();
    let mut same_time_ids = [same_a_id, same_b_id];
    same_time_ids.sort_unstable_by_key(|id| i64::from(*id));
    let expected_ids = vec![earlier_id, same_time_ids[0], same_time_ids[1], later_id];
    assert_eq!(ids, expected_ids);

    for excluded_id in [draft_id, live_id, deleted_id, other_id] {
        assert!(
            !ids.contains(&excluded_id),
            "scheduled list included excluded post {excluded_id}: {ids:?}"
        );
    }
    assert!(
        first_page
            .posts
            .iter()
            .chain(second_page.posts.iter())
            .all(|row| row.post.published_at.is_some()),
        "scheduled rows must carry published_at"
    );
}

// Shape B — invalid-cursor cluster across the cursor-paginated endpoints.
// Each fires two requests: a half-specified cursor (a `cursor` object carrying a
// valid instant but no `post_id`) and an unparseable timestamp inside an
// otherwise complete cursor. Both are rejected at arg-decode, before the
// handler body: the cursor is one `PageCursor` field (ADR-0065 typing all the
// way down), so a half cursor is a missing required struct field. We assert the
// half cursor names the component it is missing, and otherwise only that the
// request is rejected, rather than pinning the decode-layer wording. Only the
// endpoint URI and the (username-carrying where required) request bodies vary.
// An author session is always created and passed — the public endpoints ignore
// it but still run the same cursor decode, so a single setup serves every row
// without branching.
#[apply(backends_matrix)]
#[case::list_drafts(
    <web::posts::ListDrafts as ServerFn>::PATH,
    serde_json::json!({ "cursor": { "created_at": "2026-04-16T10:11:12+00:00" }, "limit": 10 }),
    serde_json::json!({ "cursor": { "created_at": "bad-time", "post_id": 10 }, "limit": 10 })
)]
#[case::list_scheduled(
    <web::posts::ListScheduled as ServerFn>::PATH,
    serde_json::json!({ "cursor": { "created_at": "2026-04-16T10:11:12+00:00" }, "limit": 10 }),
    serde_json::json!({ "cursor": { "created_at": "bad-time", "post_id": 11 }, "limit": 10 })
)]
#[case::list_user_posts(
    <web::timeline::ListByUser as ServerFn>::PATH,
    serde_json::json!({
        "username": "author",
        "cursor": { "created_at": "2026-04-16T10:11:12+00:00" },
        "limit": 10,
    }),
    serde_json::json!({
        "username": "author",
        "cursor": { "created_at": "bad-time", "post_id": 12 },
        "limit": 10,
    })
)]
#[case::list_local_timeline(
    <web::timeline::ListLocalTimeline as ServerFn>::PATH,
    serde_json::json!({ "cursor": { "created_at": "2026-04-16T10:11:12+00:00" }, "limit": 10 }),
    serde_json::json!({ "cursor": { "created_at": "bad-time", "post_id": 12 }, "limit": 10 })
)]
#[case::list_home_feed(
    <web::timeline::ListHomeFeed as ServerFn>::PATH,
    serde_json::json!({ "cursor": { "created_at": "2026-04-16T10:11:12+00:00" }, "limit": 10 }),
    serde_json::json!({ "cursor": { "created_at": "bad-time", "post_id": 12 }, "limit": 10 })
)]
#[tokio::test]
async fn list_rejects_invalid_cursor_inputs(
    backend: Backend,
    #[case] uri: &str,
    #[case] half_cursor_body: serde_json::Value,
    #[case] bad_time_body: serde_json::Value,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = post_json(&state, uri, half_cursor_body, Some(&cookie)).await;
    assert_ne!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.contains("post_id"),
        "the rejection names the missing cursor component: {body}"
    );

    let (status, body) = post_json(&state, uri, bad_time_body, Some(&cookie)).await;
    assert_ne!(status, StatusCode::OK, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_returns_published_posts_with_cursor_pagination(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let author_cookie = author.cookie();
    let other_cookie = create_user_and_session(&state).await.cookie();

    storage::test_support::seed_posts(&state, author.user_id, 51, true).await;

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("private"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(parse_post_body("body"), PostFormat::Markdown)
        },
        Some(&other_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = list_user_posts(&state, &author.username, None, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Page<RenderedPost> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost>>>(&body)
            .unwrap()
            .page;
    assert_eq!(first_page.posts.len(), 50, "body: {body}");
    assert!(first_page.has_more, "body: {body}");
    assert!(first_page.next_cursor.is_some(), "body: {body}");
    assert!(
        first_page.posts.iter().all(|post| post
            .permalink
            .as_ref()
            .is_some_and(|p| p.starts_with(&format!("/~{}/", author.username)))),
        "body: {body}"
    );
    assert!(
        first_page.posts.iter().all(|post| post
            .title
            .as_deref()
            .is_none_or(|title| !title.contains("Draft"))),
        "body: {body}"
    );

    let (status, body) =
        list_user_posts(&state, &author.username, first_page.next_cursor, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: Page<RenderedPost> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost>>>(&body)
            .unwrap()
            .page;
    assert_eq!(second_page.posts.len(), 1, "body: {body}");
    assert!(!second_page.has_more, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_rejects_invalid_username(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = list_user_posts(&state, "Invalid Name", None, 50, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("username"), "body: {body}");
}

// The cursor's shape ON THE WIRE, asserted as bytes rather than through a helper.
// A behavioural test cannot see this: moving the signature to one `PageCursor`
// while leaving the form-urlencoded codec in place would still round-trip through
// `list_user_posts` and pass. So both halves are hand-built here — the nested
// JSON object must decode, and the flat `cursor_created_at`/`cursor_post_id` pair
// must not, which is what pins the codec change itself.
#[apply(backends)]
#[tokio::test]
async fn list_by_user_takes_a_nested_json_cursor_and_no_longer_the_flat_pair(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = SeedUser::new().seed(&state).await;
    storage::test_support::seed_posts(&state, author.user_id, 2, true).await;

    let nested = serde_json::json!({
        "username": author.username,
        "cursor": { "created_at": "2026-01-01T00:00:00Z", "post_id": 7 },
        "limit": 10,
    });
    let (status, body) = post_json(
        &state,
        <web::timeline::ListByUser as ServerFn>::PATH,
        nested,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let flat = format!(
        "username={}&cursor_created_at=2026-01-01T00:00:00%2B00:00&cursor_post_id=7&limit=10",
        author.username
    );
    let (status, body) = post_form(
        &state,
        <web::timeline::ListByUser as ServerFn>::PATH,
        flat,
        None,
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "the flat urlencoded cursor pair must no longer decode: {body}"
    );
}

// The behavioural half of the same change: the cursor a page hands back is fed
// straight back in as one value and advances the listing.
#[apply(backends)]
#[tokio::test]
async fn timeline_page_two_uses_the_cursor_the_first_page_returned(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = SeedUser::new().seed(&state).await;
    storage::test_support::seed_posts(&state, author.user_id, 2, true).await;

    let (status, body) = list_user_posts(&state, &author.username, None, 1, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Page<RenderedPost> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost>>>(&body)
            .unwrap()
            .page;
    assert_eq!(first_page.posts.len(), 1, "body: {body}");
    let cursor = first_page
        .next_cursor
        .expect("page 1 has more, so it carries a cursor");

    let (status, body) = list_user_posts(&state, &author.username, Some(cursor), 1, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: Page<RenderedPost> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost>>>(&body)
            .unwrap()
            .page;
    assert_eq!(second_page.posts.len(), 1, "body: {body}");
    assert_ne!(
        second_page.posts[0].post_id, first_page.posts[0].post_id,
        "the cursor advanced the listing: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_local_timeline_returns_published_posts_with_cursor_pagination(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = SeedUser::new().seed(&state).await;
    let other = SeedUser::new().seed(&state).await;
    let author_cookie = create_session_for(&state, author.user_id).await.cookie();
    storage::test_support::seed_posts(&state, author.user_id, 26, true).await;
    storage::test_support::seed_posts(&state, other.user_id, 26, true).await;

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("private"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(parse_post_body("gone"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let deleted: SavedPost = confirmed_mutation(&body);
    let posts = Arc::clone(&state.posts);
    state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move {
                posts
                    .soft_delete_post(
                        transaction,
                        deleted.post_id,
                        author.user_id,
                        common::time::UtcInstant::now(),
                    )
                    .await
            })
        })
        .await
        .unwrap();

    let (status, body) = list_local_timeline(&state, None, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Page<RenderedPost> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost>>>(&body)
            .unwrap()
            .page;
    assert_eq!(first_page.posts.len(), 50, "body: {body}");
    assert!(first_page.has_more, "body: {body}");
    assert!(first_page.next_cursor.is_some(), "body: {body}");
    assert!(
        first_page
            .posts
            .iter()
            .any(|post| post.username == author.username),
        "body: {body}"
    );
    assert!(
        first_page
            .posts
            .iter()
            .any(|post| post.username == other.username),
        "body: {body}"
    );
    assert!(
        first_page
            .posts
            .iter()
            .all(|post| post.permalink.as_ref().is_some_and(|p| p.starts_with("/~"))),
        "body: {body}"
    );
    assert!(
        first_page.posts.iter().all(|post| post
            .title
            .as_deref()
            .is_none_or(|title| { !title.contains("Draft") && !title.contains("Deleted") })),
        "body: {body}"
    );

    let (status, body) = list_local_timeline(&state, first_page.next_cursor, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: Page<RenderedPost> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost>>>(&body)
            .unwrap()
            .page;
    assert_eq!(second_page.posts.len(), 2, "body: {body}");
    assert!(!second_page.has_more, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_home_feed_returns_authenticated_users_published_posts_only(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let author_cookie = author.cookie();
    let other_cookie = create_user_and_session(&state).await.cookie();

    storage::test_support::seed_posts(&state, author.user_id, 51, true).await;

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("private"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    for i in 0..3 {
        let request_body = format!("# Post {i}\n\nbody");
        let (status, body) = create_post_json(
            &state,
            PostInputs {
                publish: Some(true),
                ..PostInputs::new(parse_post_body(&request_body), PostFormat::Markdown)
            },
            Some(&other_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create body: {body}");
    }

    let (status, body) = list_home_feed(&state, None, 50, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Page<RenderedPost> = serde_json::from_str(&body).unwrap();
    assert_eq!(first_page.posts.len(), 50, "body: {body}");
    assert!(first_page.has_more, "body: {body}");
    assert!(first_page.next_cursor.is_some(), "body: {body}");
    assert!(
        first_page
            .posts
            .iter()
            .all(|post| post.username == author.username),
        "body: {body}"
    );
    assert!(
        first_page.posts.iter().all(|post| post
            .title
            .as_deref()
            .is_none_or(|title| { !title.contains("Other") && !title.contains("Draft") })),
        "body: {body}"
    );

    let (status, body) =
        list_home_feed(&state, first_page.next_cursor, 50, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: Page<RenderedPost> = serde_json::from_str(&body).unwrap();
    assert_eq!(second_page.posts.len(), 1, "body: {body}");
    assert!(!second_page.has_more, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_carries_tags_per_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let (status, body) = create_post_json(
        &state,
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(
                parse_post_body("# Tagged Post\n\nbody"),
                PostFormat::Markdown,
            )
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = confirmed_mutation(&body);

    // Apply two tags via the storage layer (the create_post tags param lands
    // in tags.5; here we just verify the timeline surface threads them
    // through).
    // Applied in reverse-slug order so the slug assertion below tests ordering
    // (#772) rather than coinciding with insertion order.
    storage::test_support::set_post_tags_confirmed(
        &state.write_scope,
        std::sync::Arc::clone(&state.posts),
        created.post_id,
        session.user_id,
        &[
            "web".parse::<TagLabel>().unwrap(),
            "Rust".parse::<TagLabel>().unwrap(),
        ],
    )
    .await
    .unwrap();

    let (status, body) = list_user_posts(&state, &session.username, None, 50, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "list body: {body}");
    let page: Page<RenderedPost> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost>>>(&body)
            .unwrap()
            .page;
    assert_eq!(page.posts.len(), 1);
    let post = &page.posts[0];
    let slugs: Vec<&str> = post.tags.iter().map(|t| t.slug.as_ref()).collect();
    assert_eq!(slugs, vec!["rust", "web"]);
    // Display casing is preserved (author-provided).
    assert!(post.tags.iter().any(|t| t.display == "Rust"));
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_for_unknown_user_keeps_empty_profile_with_site_theme(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = list_user_posts(&state, "nobody", None, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let presentation: PublicPresentation<Page<RenderedPost>> = serde_json::from_str(&body).unwrap();
    assert_eq!(presentation.theme, Theme::Studio);
    assert!(presentation.page.posts.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag_returns_matching_posts_from_all_users(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Two authors each post twice; only some posts get the target tag.
    let alice = create_user_and_session(&state).await;
    let alice_cookie = alice.cookie();
    let bob = create_user_and_session(&state).await;
    let bob_cookie = bob.cookie();

    let create = |cookie: String, body: &'static str, tags: Vec<TagLabel>| {
        let state = Arc::clone(&state);
        async move {
            let (status, body) = create_post_json(
                &state,
                PostInputs {
                    publish: Some(true),
                    tags: Some(tags),
                    ..PostInputs::new(parse_post_body(body), PostFormat::Markdown)
                },
                Some(&cookie),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "create body: {body}");
            confirmed_mutation::<SavedPost>(&body)
        }
    };

    create(
        alice_cookie.clone(),
        "# Alice A\n\nbody",
        vec![parse_tag_label("rust"), parse_tag_label("web")],
    )
    .await;
    create(
        alice_cookie,
        "# Alice B\n\nbody",
        vec![parse_tag_label("rust")],
    )
    .await;
    create(
        bob_cookie.clone(),
        "# Bob A\n\nbody",
        vec![parse_tag_label("rust"), parse_tag_label("perf")],
    )
    .await;
    create(
        bob_cookie,
        "# Bob B\n\nbody",
        vec![parse_tag_label("javascript")],
    )
    .await;

    let (status, body) = list_posts_by_tag(&state, "rust", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: Page<RenderedPost> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost>>>(&body)
            .unwrap()
            .page;
    // Three posts carry the "rust" tag, across both authors.
    assert_eq!(page.posts.len(), 3);
    let usernames: std::collections::HashSet<&str> =
        page.posts.iter().map(|p| p.username.as_ref()).collect();
    assert!(usernames.contains(&*alice.username));
    assert!(usernames.contains(&*bob.username));
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag_returns_empty_for_unknown_tag(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = list_posts_by_tag(&state, "rust", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: Page<RenderedPost> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost>>>(&body)
            .unwrap()
            .page;
    assert!(page.posts.is_empty());
    assert!(!page.has_more);
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_scopes_to_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let alice_cookie = author.cookie();
    let bob_cookie = create_user_and_session(&state).await.cookie();

    // Alice ("author") + Bob each post with shared tag.
    let create = |cookie: String, body: &'static str| {
        let state = Arc::clone(&state);
        async move {
            let (status, body) = create_post_json(
                &state,
                PostInputs {
                    publish: Some(true),
                    tags: Some(vec![parse_tag_label("shared")]),
                    ..PostInputs::new(parse_post_body(body), PostFormat::Markdown)
                },
                Some(&cookie),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "create body: {body}");
        }
    };
    create(alice_cookie, "# Author Post\n\nbody").await;
    create(bob_cookie, "# Bob Post\n\nbody").await;

    let (status, body) = list_user_posts_by_tag(&state, &author.username, "shared", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: Page<RenderedPost> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost>>>(&body)
            .unwrap()
            .page;
    assert_eq!(page.posts.len(), 1);
    assert_eq!(page.posts[0].username, author.username);
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_unknown_user_returns_not_found(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = list_user_posts_by_tag(&state, "nobody", "rust", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("user"), "body: {body}");
}
