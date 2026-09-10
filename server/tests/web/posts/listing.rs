use std::sync::Arc;

use axum::{Router, http::StatusCode};
use common::seed::{Page, PublicPresentation, RenderedPost, TimelineCursor, TimelineOrder};
use common::tag::TagLabel;
use common::test_support::{parse_post_body, parse_tag_label};
use common::theme::Theme;
use common::time::UtcInstant;
use jiff::ToSpan;
use server_fn::ServerFn;
use storage::PostFormat;
use web::posts::{PostInputs, UnpublishedPost};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{
    confirmed_created_post, create_post_json, create_session_for, create_user_and_session,
    make_app, post_form, post_json,
};
use storage::test_support::{Backend, SeedRawPost, SeedUser, backends, backends_matrix};

use super::fixtures::{list_drafts, list_local_timeline, list_scheduled, list_user_posts};

async fn list_posts_by_tag(app: Router, tag: &str, cookie: Option<&str>) -> (StatusCode, String) {
    post_json(
        app,
        <web::timeline::ListByTag as ServerFn>::PATH,
        serde_json::json!({
            "tag": tag,
            "request": { "order": "newest", "cursor": null, "limit": 50 },
        }),
        cookie,
    )
    .await
}

async fn list_user_posts_by_tag(
    app: Router,
    username: &str,
    tag: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::timeline::ListByUserAndTag as ServerFn>::PATH,
        serde_json::json!({
            "username": username,
            "tag": tag,
            "request": { "order": "newest", "cursor": null, "limit": 50 },
        }),
        cookie,
    )
    .await
}

async fn list_home_feed_in_order(
    app: Router,
    order: TimelineOrder,
    cursor: Option<TimelineCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::timeline::ListHomeFeed as ServerFn>::PATH,
        serde_json::json!({
            "request": { "order": order, "cursor": cursor, "limit": limit },
        }),
        cookie,
    )
    .await
}

#[apply(backends)]
#[tokio::test]
async fn list_drafts_returns_current_user_drafts_with_cursor_pagination(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author_cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    let stranger_cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("first"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let first_draft = confirmed_created_post(&body);

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("second"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let second_draft = confirmed_created_post(&body);

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(parse_post_body("visible"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("private"), PostFormat::Markdown)
        },
        Some(&stranger_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = list_drafts(app.clone(), None, 1, Some(&author_cookie)).await;
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

    let (status, body) = list_drafts(app.clone(), Some(cursor), 10, Some(&author_cookie)).await;
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
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    // Seed a scheduled post (future `published_at`) and a live post (past)
    // directly via storage — the web compose datetime control is Task 6.
    let now = UtcInstant::now();
    let scheduled_at = UtcInstant::from(
        now.value()
            .checked_add(72.hours())
            .expect("fixture is within Timestamp range"),
    );
    let sched_id = SeedRawPost::new(author.user_id)
        .published_at(scheduled_at)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await
        .post_id;
    let live_at = UtcInstant::from(
        now.value()
            .checked_sub(24.hours())
            .expect("fixture is within Timestamp range"),
    );
    let live_id = SeedRawPost::new(author.user_id)
        .published_at(live_at)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await
        .post_id;

    let (status, body) = list_drafts(app.clone(), None, 50, Some(&author.cookie())).await;
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
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let stranger = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let author_cookie = author.cookie();

    let now = UtcInstant::now();
    let same_time = UtcInstant::from(
        now.value()
            .checked_add(72.hours())
            .expect("fixture is within Timestamp range"),
    );

    let draft_id = SeedRawPost::new(author.user_id)
        .draft()
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await
        .post_id;
    let live_at = UtcInstant::from(
        now.value()
            .checked_sub(24.hours())
            .expect("fixture is within Timestamp range"),
    );
    let live_id = SeedRawPost::new(author.user_id)
        .published_at(live_at)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await
        .post_id;
    let deleted_at = UtcInstant::from(
        now.value()
            .checked_add(48.hours())
            .expect("fixture is within Timestamp range"),
    );
    let deleted_id = SeedRawPost::new(author.user_id)
        .published_at(deleted_at)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await
        .post_id;
    let posts = Arc::clone(&env.posts());
    env.write_scope()
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
    let other_at = UtcInstant::from(
        now.value()
            .checked_add(24.hours())
            .expect("fixture is within Timestamp range"),
    );
    let other_id = SeedRawPost::new(stranger.user_id)
        .published_at(other_at)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await
        .post_id;

    let earlier_at = UtcInstant::from(
        now.value()
            .checked_add(24.hours())
            .expect("fixture is within Timestamp range"),
    );
    let earlier_id = SeedRawPost::new(author.user_id)
        .published_at(earlier_at)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await
        .post_id;
    let same_a_id = SeedRawPost::new(author.user_id)
        .published_at(same_time)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await
        .post_id;
    let same_b_id = SeedRawPost::new(author.user_id)
        .published_at(same_time)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await
        .post_id;
    let later_at = UtcInstant::from(
        now.value()
            .checked_add(120.hours())
            .expect("fixture is within Timestamp range"),
    );
    let later_id = SeedRawPost::new(author.user_id)
        .published_at(later_at)
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await
        .post_id;

    let (status, body) = list_scheduled(app.clone(), None, 2, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Page<UnpublishedPost> = serde_json::from_str(&body).unwrap();
    assert_eq!(first_page.posts.len(), 2, "body: {body}");
    assert!(first_page.has_more, "body: {body}");
    let cursor = first_page
        .next_cursor
        .expect("page 1 has more, so it carries a cursor");

    let (status, body) = list_scheduled(app.clone(), Some(cursor), 10, Some(&author_cookie)).await;
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
// handler body: each cursor is a typed field. Draft and scheduled listings use
// `PageCursor`; a timeline request nests `TimelineCursor`. Thus a half cursor
// is missing a required struct field. We assert the half cursor names the
// component it is missing, and otherwise only that the
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
        "request": { "order": "newest", "cursor": { "published_at": "2026-04-16T10:11:12+00:00" }, "limit": 10 },
    }),
    serde_json::json!({
        "username": "author",
        "request": { "order": "newest", "cursor": { "published_at": "bad-time", "post_id": 12 }, "limit": 10 },
    })
)]
#[case::list_local_timeline(
    <web::timeline::ListLocalTimeline as ServerFn>::PATH,
    serde_json::json!({ "request": { "order": "newest", "cursor": { "published_at": "2026-04-16T10:11:12+00:00" }, "limit": 10 } }),
    serde_json::json!({ "request": { "order": "newest", "cursor": { "published_at": "bad-time", "post_id": 12 }, "limit": 10 } })
)]
#[case::list_home_feed(
    <web::timeline::ListHomeFeed as ServerFn>::PATH,
    serde_json::json!({ "request": { "order": "newest", "cursor": { "published_at": "2026-04-16T10:11:12+00:00" }, "limit": 10 } }),
    serde_json::json!({ "request": { "order": "newest", "cursor": { "published_at": "bad-time", "post_id": 12 }, "limit": 10 } })
)]
#[tokio::test]
async fn list_rejects_invalid_cursor_inputs(
    backend: Backend,
    #[case] uri: &str,
    #[case] half_cursor_body: serde_json::Value,
    #[case] bad_time_body: serde_json::Value,
) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_json(app.clone(), uri, half_cursor_body, Some(&cookie)).await;
    assert_ne!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.contains("post_id"),
        "the rejection names the missing cursor component: {body}"
    );

    let (status, body) = post_json(app.clone(), uri, bad_time_body, Some(&cookie)).await;
    assert_ne!(status, StatusCode::OK, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn timeline_rejects_a_cursor_from_the_opposite_order(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let (status, body) = post_json(
        app,
        <web::timeline::ListByUser as ServerFn>::PATH,
        serde_json::json!({
            "username": author.username,
            "request": {
                "order": "oldest",
                "cursor": {
                    "published_at": "2026-04-16T10:11:12+00:00",
                    "post_id": 1,
                    "order": "newest",
                },
                "limit": 10,
            },
        }),
        None,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "opposite-order cursor must reject: {body}"
    );
    assert!(body.contains("order mismatch"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_returns_published_posts_with_cursor_pagination(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let author_cookie = author.cookie();
    let other_cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    storage::test_support::seed_posts(env.posts(), env.write_scope(), author.user_id, 51, true)
        .await;

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("private"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(parse_post_body("body"), PostFormat::Markdown)
        },
        Some(&other_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = list_user_posts(app.clone(), &author.username, None, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Page<RenderedPost, TimelineCursor> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost, TimelineCursor>>>(&body)
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

    let (status, body) = list_user_posts(
        app.clone(),
        &author.username,
        first_page.next_cursor,
        50,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: Page<RenderedPost, TimelineCursor> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost, TimelineCursor>>>(&body)
            .unwrap()
            .page;
    assert_eq!(second_page.posts.len(), 1, "body: {body}");
    assert!(!second_page.has_more, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_rejects_invalid_username(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) = list_user_posts(app.clone(), "Invalid Name", None, 50, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(body.contains("username"), "body: {body}");
}

// The cursor's shape ON THE WIRE, asserted as bytes rather than through a helper.
// A behavioural test cannot see this: moving the signature to a `TimelinePageRequest`
// while leaving the form-urlencoded codec in place would still round-trip through
// `list_user_posts` and pass. So both halves are hand-built here — the nested
// JSON object must decode, and the flat `cursor_created_at`/`cursor_post_id` pair
// must not, which is what pins the codec change itself.
#[apply(backends)]
#[tokio::test]
async fn list_by_user_takes_a_nested_json_cursor_and_no_longer_the_flat_pair(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;
    storage::test_support::seed_posts(env.posts(), env.write_scope(), author.user_id, 2, true)
        .await;

    let nested = serde_json::json!({
        "username": author.username,
        "request": {
            "order": "newest",
            "cursor": { "published_at": "2026-01-01T00:00:00Z", "post_id": 7, "order": "newest" },
            "limit": 10,
        },
    });
    let (status, body) = post_json(
        app.clone(),
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
        app.clone(),
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
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;
    storage::test_support::seed_posts(env.posts(), env.write_scope(), author.user_id, 2, true)
        .await;

    let (status, body) = list_user_posts(app.clone(), &author.username, None, 1, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Page<RenderedPost, TimelineCursor> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost, TimelineCursor>>>(&body)
            .unwrap()
            .page;
    assert_eq!(first_page.posts.len(), 1, "body: {body}");
    let cursor = first_page
        .next_cursor
        .expect("page 1 has more, so it carries a cursor");

    let (status, body) =
        list_user_posts(app.clone(), &author.username, Some(cursor), 1, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: Page<RenderedPost, TimelineCursor> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost, TimelineCursor>>>(&body)
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
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;
    let other = SeedUser::new()
        .seed(std::sync::Arc::clone(&env.users()), env.write_scope())
        .await;
    let author_cookie = create_session_for(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
        author.user_id,
    )
    .await
    .cookie();
    storage::test_support::seed_posts(env.posts(), env.write_scope(), author.user_id, 26, true)
        .await;
    storage::test_support::seed_posts(env.posts(), env.write_scope(), other.user_id, 26, true)
        .await;

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(parse_post_body("private"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(true),
            ..PostInputs::new(parse_post_body("gone"), PostFormat::Markdown)
        },
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let deleted = confirmed_created_post(&body);
    let posts = Arc::clone(&env.posts());
    env.write_scope()
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

    let (status, body) = list_local_timeline(app.clone(), None, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Page<RenderedPost, TimelineCursor> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost, TimelineCursor>>>(&body)
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

    let (status, body) = list_local_timeline(app.clone(), first_page.next_cursor, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: Page<RenderedPost, TimelineCursor> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost, TimelineCursor>>>(&body)
            .unwrap()
            .page;
    assert_eq!(second_page.posts.len(), 2, "body: {body}");
    assert!(!second_page.has_more, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_home_feed_returns_authenticated_users_published_posts_only(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let author_cookie = author.cookie();
    let other_cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let now = UtcInstant::now();
    let mut admitted_post_ids = Vec::with_capacity(51);
    for hours_ago in (1_i64..=51).rev() {
        let published_at = UtcInstant::from(
            now.value()
                .checked_sub(hours_ago.hours())
                .expect("fixture is within Timestamp range"),
        );
        let post_id = SeedRawPost::new(author.user_id)
            .published_at(published_at)
            .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
            .await
            .post_id;
        admitted_post_ids.push(post_id);
    }

    let (status, body) = create_post_json(
        app.clone(),
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
            app.clone(),
            PostInputs {
                publish: Some(true),
                ..PostInputs::new(parse_post_body(&request_body), PostFormat::Markdown)
            },
            Some(&other_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create body: {body}");
    }

    let mut ids_by_order = Vec::with_capacity(2);
    for order in [TimelineOrder::Newest, TimelineOrder::Oldest] {
        let expected_ids: Vec<_> = match order {
            TimelineOrder::Newest => admitted_post_ids.iter().rev().copied().collect(),
            TimelineOrder::Oldest => admitted_post_ids.clone(),
        };

        let (status, body) =
            list_home_feed_in_order(app.clone(), order, None, 50, Some(&author_cookie)).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let first_page: Page<RenderedPost, TimelineCursor> = serde_json::from_str(&body).unwrap();
        assert_eq!(first_page.posts.len(), 50, "body: {body}");
        assert!(first_page.has_more, "body: {body}");
        let cursor = first_page
            .next_cursor
            .expect("page 1 has more, so it carries a cursor");
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
        let first_page_ids: Vec<_> = first_page.posts.iter().map(|post| post.post_id).collect();
        assert_eq!(first_page_ids, expected_ids[..50], "body: {body}");

        let (status, body) =
            list_home_feed_in_order(app.clone(), order, Some(cursor), 50, Some(&author_cookie))
                .await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        let second_page: Page<RenderedPost, TimelineCursor> = serde_json::from_str(&body).unwrap();
        assert_eq!(second_page.posts.len(), 1, "body: {body}");
        assert!(!second_page.has_more, "body: {body}");
        assert!(second_page.next_cursor.is_none(), "body: {body}");
        let second_page_ids: Vec<_> = second_page.posts.iter().map(|post| post.post_id).collect();
        assert_eq!(second_page_ids, expected_ids[50..], "body: {body}");

        let ids: Vec<_> = first_page_ids.into_iter().chain(second_page_ids).collect();
        assert_eq!(ids, expected_ids, "body: {body}");
        ids_by_order.push(ids);
    }

    assert_eq!(
        ids_by_order[0],
        ids_by_order[1].iter().rev().copied().collect::<Vec<_>>()
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_carries_tags_per_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let cookie = session.cookie();

    let (status, body) = create_post_json(
        app.clone(),
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
    let created = confirmed_created_post(&body);

    // Apply two tags via the storage layer (the create_post tags param lands
    // in tags.5; here we just verify the timeline surface threads them
    // through).
    // Applied in reverse-slug order so the slug assertion below tests ordering
    // (#772) rather than coinciding with insertion order.
    storage::test_support::set_post_tags_confirmed(
        &env.write_scope(),
        std::sync::Arc::clone(&env.posts()),
        created.post_id,
        session.user_id,
        &[
            "web".parse::<TagLabel>().unwrap(),
            "Rust".parse::<TagLabel>().unwrap(),
        ],
    )
    .await
    .unwrap();

    let (status, body) =
        list_user_posts(app.clone(), &session.username, None, 50, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "list body: {body}");
    let page: Page<RenderedPost, TimelineCursor> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost, TimelineCursor>>>(&body)
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
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) = list_user_posts(app.clone(), "nobody", None, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let presentation: PublicPresentation<Page<RenderedPost, TimelineCursor>> =
        serde_json::from_str(&body).unwrap();
    assert_eq!(
        presentation.theme,
        common::theme::PublishedThemePresentation::built_in(Theme::Studio)
    );
    assert!(presentation.page.posts.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag_returns_matching_posts_from_all_users(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    // Two authors each post twice; only some posts get the target tag.
    let alice = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let alice_cookie = alice.cookie();
    let bob = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let bob_cookie = bob.cookie();

    let create = |cookie: String, body: &'static str, tags: Vec<TagLabel>| {
        let app = app.clone();
        async move {
            let (status, body) = create_post_json(
                app.clone(),
                PostInputs {
                    publish: Some(true),
                    tags: Some(tags),
                    ..PostInputs::new(parse_post_body(body), PostFormat::Markdown)
                },
                Some(&cookie),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "create body: {body}");
            confirmed_created_post(&body)
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

    let (status, body) = list_posts_by_tag(app.clone(), "rust", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: Page<RenderedPost, TimelineCursor> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost, TimelineCursor>>>(&body)
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
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) = list_posts_by_tag(app.clone(), "rust", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: Page<RenderedPost, TimelineCursor> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost, TimelineCursor>>>(&body)
            .unwrap()
            .page;
    assert!(page.posts.is_empty());
    assert!(!page.has_more);
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_scopes_to_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let author = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let alice_cookie = author.cookie();
    let bob_cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    // Alice ("author") + Bob each post with shared tag.
    let create = |cookie: String, body: &'static str| {
        let app = app.clone();
        async move {
            let (status, body) = create_post_json(
                app.clone(),
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

    let (status, body) =
        list_user_posts_by_tag(app.clone(), &author.username, "shared", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: Page<RenderedPost, TimelineCursor> =
        serde_json::from_str::<PublicPresentation<Page<RenderedPost, TimelineCursor>>>(&body)
            .unwrap()
            .page;
    assert_eq!(page.posts.len(), 1);
    assert_eq!(page.posts[0].username, author.username);
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_unknown_user_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (status, body) = list_user_posts_by_tag(app.clone(), "nobody", "rust", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("user"), "body: {body}");
}
