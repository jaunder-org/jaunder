use std::sync::Arc;

use axum::http::StatusCode;
use chrono::Datelike;
use common::ids::{PostId, UserId};
use common::seed::{AuthoredPost, TimelinePage};
use common::test_support::parse_audience_name;
use server_fn::ServerFn;
use web::posts::{EditPostPreview, SavedPost};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{
    create_session_for, create_user_and_session, post_form, post_json_with_credentials,
};
use storage::test_support::{
    Backend, SeedRawPost, SeedUser, SeededPost, TestEnv, backends, backends_matrix,
};

use super::fixtures::{
    create_post_json, get_post_form, list_drafts, list_home_feed, list_local_timeline,
    publish_post_form, update_post_json,
};

async fn get_post_preview_form(
    state: &Arc<storage::AppState>,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let body = format!("post_id={post_id}");
    post_form(
        state,
        <web::posts::GetPreview as ServerFn>::PATH,
        body,
        cookie,
    )
    .await
}

/// Which endpoint a `*_rejects_unauthenticated` case exercises. Each variant
/// fires the same request the original standalone test fired, with no session
/// cookie, through that endpoint's existing request builder.
#[derive(Copy, Clone)]
enum UnauthEndpoint {
    CreatePost,
    UpdatePost,
    ListDrafts,
    PublishPost,
    ListHomeFeed,
}

async fn unauthenticated_request(
    state: &Arc<storage::AppState>,
    endpoint: UnauthEndpoint,
) -> (StatusCode, String) {
    match endpoint {
        UnauthEndpoint::CreatePost => {
            create_post_json(state, "body", "markdown", None, false, None).await
        }
        UnauthEndpoint::UpdatePost => {
            update_post_json(
                state,
                PostId::from(42),
                "body",
                "markdown",
                None,
                false,
                None,
            )
            .await
        }
        UnauthEndpoint::ListDrafts => list_drafts(state, None, 10, None).await,
        UnauthEndpoint::PublishPost => publish_post_form(state, PostId::from(99), None).await,
        UnauthEndpoint::ListHomeFeed => list_home_feed(state, None, 50, None).await,
    }
}

// Shape B — `*_rejects_unauthenticated` cluster across endpoints. Identical
// assertion (INTERNAL_SERVER_ERROR + "unauthorized"); only the endpoint (and
// thus the request builder) varies.
#[apply(backends_matrix)]
#[case::create_post(UnauthEndpoint::CreatePost)]
#[case::update_post(UnauthEndpoint::UpdatePost)]
#[case::list_drafts(UnauthEndpoint::ListDrafts)]
#[case::publish_post(UnauthEndpoint::PublishPost)]
#[case::list_home_feed(UnauthEndpoint::ListHomeFeed)]
#[tokio::test]
async fn endpoint_rejects_unauthenticated(backend: Backend, #[case] endpoint: UnauthEndpoint) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = unauthenticated_request(&state, endpoint).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_returns_draft_to_author_only(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let author_cookie = author.cookie();
    let stranger_cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = create_post_json(
        &state,
        "# Draft

draft",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .unwrap();

    let (status, body) = get_post_form(
        &state,
        &author.username,
        record.created_at.year(),
        record.created_at.month(),
        record.created_at.day(),
        &created.slug,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");

    let (status, body) = get_post_form(
        &state,
        &author.username,
        record.created_at.year(),
        record.created_at.month(),
        record.created_at.day(),
        &created.slug,
        Some(&stranger_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");

    let (status, body) = get_post_form(
        &state,
        &author.username,
        record.created_at.year(),
        record.created_at.month(),
        record.created_at.day(),
        &created.slug,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("\"is_draft\":true"), "body: {body}");
    assert!(body.contains("Draft"), "body: {body}");

    let (status, body) = get_post_preview_form(&state, created.post_id, Some(&author_cookie)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "author preview should succeed: {body}"
    );
    assert!(body.contains("Draft"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_preview_shows_draft_to_author_only(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_cookie = create_user_and_session(&state).await.cookie();
    let stranger_cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = create_post_json(
        &state,
        "# Preview Draft

draft",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();

    let before = chrono::Utc::now();
    let (status, body) = get_post_preview_form(&state, created.post_id, Some(&author_cookie)).await;
    let after = chrono::Utc::now();
    assert_eq!(status, StatusCode::OK, "author preview failed: {body}");

    let preview: EditPostPreview = serde_json::from_str(&body).unwrap();
    assert_eq!(preview.post.post.post_id, created.post_id);
    assert_eq!(preview.post.body.as_ref(), "# Preview Draft\n\ndraft\n");
    assert!(preview.post.post.published_at.is_none());
    assert!(preview.fetched_at.value() >= before);
    assert!(preview.fetched_at.value() <= after);

    let (status, body) =
        get_post_preview_form(&state, created.post_id, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");

    let (status, body) = get_post_preview_form(&state, created.post_id, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_hides_drafts_from_guests(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let author_cookie = author.cookie();

    let (status, body) = create_post_json(
        &state,
        "draft",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: SavedPost = serde_json::from_str(&body).unwrap();
    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .unwrap();

    let (status, body) = get_post_form(
        &state,
        &author.username,
        record.created_at.year(),
        record.created_at.month(),
        record.created_at.day(),
        &created.slug,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_returns_scheduled_post_at_canonical_permalink_to_author(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author = create_user_and_session(&state).await;
    let cookie = author.cookie();
    let scheduled_at = chrono::Utc::now() + chrono::Duration::days(30);
    let scheduled = SeedRawPost::new(author.user_id)
        .published_at(scheduled_at)
        .seed(&state)
        .await;

    let (status, body) = get_post_form(
        &state,
        &author.username,
        scheduled_at.year(),
        scheduled_at.month(),
        scheduled_at.day(),
        scheduled.slug.as_ref(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let returned: AuthoredPost = serde_json::from_str(&body).unwrap();
    assert_eq!(returned.post.post_id, scheduled.post_id);
    assert!(returned.post.is_author);
}

// ---------------------------------------------------------------------------
// Content visibility — Layer A (Task 16): timeline reads thread the real
// viewer (viewer_identity) through the store resolution filter instead of the
// Anonymous stopgap. These are server-fn-level tests; the exhaustive storage
// resolution matrix lives in `storage.rs`.
// ---------------------------------------------------------------------------

/// Creates a published post for `author` with the given audience targeting,
/// directly through the store (the web create path is Public-only in Layer A).
/// Returns the [`SeededPost`] so callers read back the autogenerated slug.
async fn create_targeted_post(
    state: &Arc<storage::AppState>,
    author: UserId,
    audiences: Vec<common::visibility::AudienceTarget>,
) -> SeededPost {
    SeedRawPost::new(author)
        .audiences(audiences)
        .seed(state)
        .await
}

/// The set of post slugs visible in a local-timeline response.
fn timeline_slugs(page: &TimelinePage) -> std::collections::BTreeSet<String> {
    page.posts.iter().map(|p| p.slug.to_string()).collect()
}

#[apply(backends)]
#[tokio::test]
async fn local_timeline_enforces_visibility_for_viewer(#[case] backend: Backend) {
    use common::visibility::AudienceTarget;

    let TestEnv { state, base: _base } = backend.setup().await;

    let author = SeedUser::new().seed(&state).await.user_id;
    let subscriber = SeedUser::new().seed(&state).await.user_id;
    let stranger = SeedUser::new().seed(&state).await.user_id;

    let local = state.subscriptions.local_channel_id().await.unwrap();
    // A named audience containing the subscriber's subscription. `subscribe` is
    // idempotent, so this both establishes the active subscription and yields
    // the subscription id for audience membership.
    let friends = state
        .audiences
        .create_audience(author, &parse_audience_name("Friends"))
        .await
        .unwrap();
    let sub_id = state
        .subscriptions
        .subscribe(author, local, &i64::from(subscriber).to_string())
        .await
        .unwrap();
    state
        .audiences
        .add_member(author, friends, sub_id)
        .await
        .unwrap();

    let public = create_targeted_post(&state, author, vec![AudienceTarget::Public]).await;
    let subscribers = create_targeted_post(&state, author, vec![AudienceTarget::Subscribers]).await;
    let named = create_targeted_post(&state, author, vec![AudienceTarget::Named(friends)]).await;
    let private = create_targeted_post(&state, author, vec![]).await;

    let author_session = create_session_for(&state, author).await;
    let subscriber_session = create_session_for(&state, subscriber).await;
    let stranger_session = create_session_for(&state, stranger).await;
    let author_cookie = author_session.cookie();
    let subscriber_cookie = subscriber_session.cookie();
    let stranger_cookie = stranger_session.cookie();

    // Anonymous viewer: only the Public post.
    let (status, body) = list_local_timeline(&state, None, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let anon: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(
        timeline_slugs(&anon),
        [public.slug.to_string()].into_iter().collect(),
        "anonymous viewer sees only Public; body: {body}"
    );

    // Author: sees all of their own posts, including the private one.
    let (status, body) = list_local_timeline(&state, None, 50, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let authored: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(
        timeline_slugs(&authored),
        [
            public.slug.to_string(),
            subscribers.slug.to_string(),
            named.slug.to_string(),
            private.slug.to_string(),
        ]
        .into_iter()
        .collect(),
        "author sees own posts regardless of audience; body: {body}"
    );

    // Active subscriber + named member: Public + Subscribers + Named (not Private).
    let (status, body) = list_local_timeline(&state, None, 50, Some(&subscriber_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let sub: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(
        timeline_slugs(&sub),
        [
            public.slug.to_string(),
            subscribers.slug.to_string(),
            named.slug.to_string(),
        ]
        .into_iter()
        .collect(),
        "subscriber sees Public + Subscribers + admitted Named; body: {body}"
    );
    assert!(
        sub.posts.iter().all(|p| !p.is_author),
        "subscriber is not the author; body: {body}"
    );

    // Explicit Bearer identity is authoritative over an unrelated ambient cookie.
    let authorization = format!("Bearer {}", subscriber_session.token);
    let response = post_json_with_credentials(
        &state,
        <web::timeline::ListLocalTimeline as ServerFn>::PATH,
        serde_json::json!({ "cursor": null, "limit": 50 }),
        Some(&stranger_cookie),
        Some(&authorization),
        true,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "body: {}", response.body);
    let bearer_page: TimelinePage = serde_json::from_str(&response.body).unwrap();
    assert_eq!(
        timeline_slugs(&bearer_page),
        [
            public.slug.to_string(),
            subscribers.slug.to_string(),
            named.slug.to_string(),
        ]
        .into_iter()
        .collect()
    );
    assert!(
        response
            .set_cookies
            .iter()
            .any(|value| value.contains("Max-Age=0"))
    );

    // A present but failed explicit credential rejects instead of becoming
    // anonymous or falling back to the valid cookie.
    let response = post_json_with_credentials(
        &state,
        <web::timeline::ListLocalTimeline as ServerFn>::PATH,
        serde_json::json!({ "cursor": null, "limit": 50 }),
        Some(&subscriber_cookie),
        Some("Bearer unknown-token"),
        true,
    )
    .await;
    assert_ne!(response.status, StatusCode::OK);
    assert!(serde_json::from_str::<TimelinePage>(&response.body).is_err());
    assert!(response.set_cookies.is_empty());

    // Authed non-subscriber: only the Public post (same reach as anonymous,
    // proving viewer_identity yields a Channel viewer that is correctly *not*
    // admitted to subscriber/named content).
    let (status, body) = list_local_timeline(&state, None, 50, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let stranger_page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(
        timeline_slugs(&stranger_page),
        [public.slug.to_string()].into_iter().collect(),
        "authed non-subscriber sees only Public; body: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn single_post_permalink_hides_subscribers_post_from_anonymous(#[case] backend: Backend) {
    use common::visibility::AudienceTarget;

    let TestEnv { state, base: _base } = backend.setup().await;
    let author = SeedUser::new().seed(&state).await;
    let subscriber = SeedUser::new().seed(&state).await.user_id;

    let local = state.subscriptions.local_channel_id().await.unwrap();
    state
        .subscriptions
        .subscribe(author.user_id, local, &i64::from(subscriber).to_string())
        .await
        .unwrap();

    let seeded =
        create_targeted_post(&state, author.user_id, vec![AudienceTarget::Subscribers]).await;
    let post = state
        .posts
        .get_post_by_id(
            seeded.post_id,
            &common::visibility::ViewerIdentity::local(author.user_id),
        )
        .await
        .unwrap()
        .unwrap();
    let published = post.published_at.unwrap();
    let (y, m, d) = (published.year(), published.month(), published.day());

    // Anonymous → 404 (the resolution filter hides the subscribers-only post).
    let (status, _body) = get_post_form(
        &state,
        &author.username,
        y,
        m,
        d,
        seeded.slug.as_ref(),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "anonymous must not see subscribers-only post"
    );

    // Active subscriber → 200.
    let subscriber_cookie = create_session_for(&state, subscriber).await.cookie();
    let (status, body) = get_post_form(
        &state,
        &author.username,
        y,
        m,
        d,
        seeded.slug.as_ref(),
        Some(&subscriber_cookie),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "subscriber must see subscribers-only post; body: {body}"
    );
}
