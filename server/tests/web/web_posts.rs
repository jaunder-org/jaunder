use std::sync::Arc;

use axum::http::StatusCode;
use chrono::Datelike;
use common::ids::{PostId, UserId};
use common::tag::TagLabel;
use common::test_support::parse_audience_name;
use common::time::UtcInstant;
use common::visibility::{AudienceBase, AudienceSelection};
use storage::{PostFormat, RenderedHtml};
use web::posts::{
    CreatePostResult, DraftSummary, PublishPostResult, TimelinePage, UpdatePostResult,
};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{post_form, post_json, session_cookie};
use storage::test_support::{backends, backends_matrix, Backend, TestBase, TestEnv};

async fn unpublish_post_form(
    state: Arc<storage::AppState>,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_form(
        state,
        "/api/unpublish_post",
        format!("post_id={post_id}"),
        cookie,
    )
    .await
}

async fn create_post_json(
    state: Arc<storage::AppState>,
    body: &str,
    format: &str,
    slug_override: Option<&str>,
    publish: bool,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let payload = serde_json::json!({
        "args": {
            "body": body,
            "format": format,
            "slug_override": slug_override,
            "publish": publish,
        }
    });
    post_json(state, "/api/create_post", payload, cookie).await
}

async fn update_post_json(
    state: Arc<storage::AppState>,
    post_id: PostId,
    body: &str,
    format: &str,
    slug_override: Option<&str>,
    publish: bool,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let payload = serde_json::json!({
        "args": {
            "post_id": post_id,
            "body": body,
            "format": format,
            "slug_override": slug_override,
            "publish": publish,
        }
    });
    post_json(state, "/api/update_post", payload, cookie).await
}

async fn get_post_form(
    state: Arc<storage::AppState>,
    username: &str,
    year: i32,
    month: u32,
    day: u32,
    slug: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let body = format!("username={username}&year={year}&month={month}&day={day}&slug={slug}");
    post_form(state, "/api/get_post", body, cookie).await
}

async fn get_post_preview_form(
    state: Arc<storage::AppState>,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let body = format!("post_id={post_id}");
    post_form(state, "/api/get_post_preview", body, cookie).await
}

#[apply(backends)]
#[tokio::test]
async fn create_post_persists_rendered_published_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    // Title embedded as # heading in the body (verbatim storage)
    let (status, body) = create_post_json(
        Arc::clone(&state),
        "# Hello World

**bold**",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "hello-world");
    assert!(created.published_at.is_some());

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title.as_deref(), Some("Hello World"));
    assert_eq!(record.slug.to_string(), "hello-world");
    assert_eq!(record.format, PostFormat::Markdown);
    assert!(record.published_at.is_some());
    assert!(
        record
            .rendered_html
            .as_ref()
            .contains("<strong>bold</strong>"),
        "rendered_html: {}",
        record.rendered_html
    );
    let published_at = record.published_at.expect("published post");
    let expected_permalink = format!(
        "/~author/{:04}/{:02}/{:02}/{}",
        published_at.year(),
        published_at.month(),
        published_at.day(),
        record.slug.as_ref()
    );
    assert_eq!(created.permalink, *expected_permalink);
}

#[apply(backends)]
#[tokio::test]
async fn create_post_retries_slug_conflicts_for_same_user_and_date(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    // Title embedded as # heading; two posts with same heading produce conflicting slugs
    let (first_status, first_body) = create_post_json(
        Arc::clone(&state),
        "# Repeated Title

first",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(first_status, StatusCode::OK, "body: {first_body}");

    let (second_status, second_body) = create_post_json(
        Arc::clone(&state),
        "# Repeated Title

second",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(second_status, StatusCode::OK, "body: {second_body}");
    let created: CreatePostResult = serde_json::from_str(&second_body).unwrap();
    assert_eq!(created.slug, "repeated-title-2");
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
    state: Arc<storage::AppState>,
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
        UnauthEndpoint::ListDrafts => list_drafts_form(state, None, None, 10, None).await,
        UnauthEndpoint::PublishPost => publish_post_form(state, PostId::from(99), None).await,
        UnauthEndpoint::ListHomeFeed => list_home_feed_form(state, None, None, 50, None).await,
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

    let (status, body) = unauthenticated_request(state, endpoint).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn create_post_accepts_slug_override_and_saves_draft(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "*bold*",
        "org",
        Some("Custom-Slug"),
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "custom-slug");
    assert!(created.published_at.is_none());
    // A draft now carries its canonical (created_at-based) permalink; the permalink
    // view renders the draft for the author (#24).
    assert!(
        created.permalink.as_ref().starts_with("/~author/"),
        "draft should carry a canonical permalink: {}",
        created.permalink
    );

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.slug.to_string(), "custom-slug");
    assert_eq!(record.format, PostFormat::Org);
    assert!(record.published_at.is_none());
    assert!(record.rendered_html.as_ref().contains("<b>bold</b>"));
}

#[apply(backends)]
#[tokio::test]
async fn create_post_accepts_titleless_body(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "Titleless note",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "titleless-note");
    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title, None);
    assert_eq!(record.body, "Titleless note");
}

#[apply(backends)]
#[tokio::test]
async fn create_post_extracts_markdown_heading_title(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "# Extracted Title

Body text",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "extracted-title");
    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title.as_deref(), Some("Extracted Title"));
    // Body is stored verbatim including the heading
    assert_eq!(record.body, "# Extracted Title\n\nBody text");
    // Rendered HTML contains the heading because body is rendered verbatim
    assert!(record
        .rendered_html
        .as_ref()
        .contains("<h1>Extracted Title</h1>"));
}

// Shape B — create_post rejection cluster. Identical setup (author + session)
// and assertion structure (INTERNAL_SERVER_ERROR + body substring); only the
// request body/format and the expected error message vary. (An invalid
// `slug_override` is no longer an in-handler validation error: the typed
// `Option<Slug>` wire arg rejects it at the serde boundary — client
// pre-validation is the user-facing path, per ADR-0065; the serde-bridge
// rejection is unit-tested in `common::slug`.)
#[apply(backends_matrix)]
#[case::empty_post("", "markdown", None, "post body is required")]
#[case::invalid_format("body", "invalid_format", None, "post format must be")]
#[tokio::test]
async fn create_post_rejects(
    backend: Backend,
    #[case] request_body: &str,
    #[case] format: &str,
    #[case] slug_override: Option<&str>,
    #[case] expected: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        request_body,
        format,
        slug_override,
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains(expected), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_returns_published_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "# Permalink

**bold**",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    let published_at = record
        .published_at
        .expect("published post should have published_at");
    let (status, body) = get_post_form(
        Arc::clone(&state),
        "author",
        published_at.year(),
        published_at.month(),
        published_at.day(),
        &created.slug,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Permalink"));
    assert!(body.contains("rendered_html"));
    assert!(body.contains("published_at"));
}

#[apply(backends)]
#[tokio::test]
async fn get_post_returns_draft_to_author_only(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );
    let stranger_cookie = session_cookie(
        &state
            .sessions
            .create_session(stranger_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "# Draft

draft",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
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
        Arc::clone(&state),
        "author",
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
        Arc::clone(&state),
        "author",
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
        Arc::clone(&state),
        "author",
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

    let (status, body) =
        get_post_preview_form(Arc::clone(&state), created.post_id, Some(&author_cookie)).await;
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
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );
    let stranger_cookie = session_cookie(
        &state
            .sessions
            .create_session(stranger_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "# Preview Draft

draft",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) =
        get_post_preview_form(Arc::clone(&state), created.post_id, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "author preview failed: {body}");
    assert!(body.contains("Preview Draft"), "body: {body}");

    let (status, body) =
        get_post_preview_form(Arc::clone(&state), created.post_id, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");

    let (status, body) = get_post_preview_form(state, created.post_id, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_hides_drafts_from_guests(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "draft",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
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
        Arc::clone(&state),
        "author",
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
async fn get_post_rejects_invalid_username(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = get_post_form(state, "Invalid Name", 2024, 1, 1, "missing", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("username"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_rejects_invalid_slug(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = get_post_form(state, "author", 2024, 1, 1, "Invalid Slug", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("slug"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_returns_not_found_for_missing_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = get_post_form(state, "author", 2024, 1, 1, "missing", None).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

async fn list_drafts_form(
    state: Arc<storage::AppState>,
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut parts = vec![format!("limit={limit}")];
    if let (Some(created_at), Some(post_id)) = (cursor_created_at, cursor_post_id) {
        parts.push(format!("cursor_created_at={created_at}"));
        parts.push(format!("cursor_post_id={post_id}"));
    }
    post_form(state, "/api/list_drafts", parts.join("&"), cookie).await
}

async fn publish_post_form(
    state: Arc<storage::AppState>,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_form(
        state,
        "/api/publish_post",
        format!("post_id={post_id}"),
        cookie,
    )
    .await
}

async fn list_user_posts_form(
    state: Arc<storage::AppState>,
    username: &str,
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut parts = vec![format!("username={username}"), format!("limit={limit}")];
    if let (Some(created_at), Some(post_id)) = (cursor_created_at, cursor_post_id) {
        parts.push(format!("cursor_created_at={created_at}"));
        parts.push(format!("cursor_post_id={post_id}"));
    }
    post_form(state, "/api/list_user_posts", parts.join("&"), cookie).await
}

async fn list_posts_by_tag_form(
    state: Arc<storage::AppState>,
    tag: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let body = format!("tag={tag}&limit=50");
    post_form(state, "/api/list_posts_by_tag", body, cookie).await
}

async fn list_user_posts_by_tag_form(
    state: Arc<storage::AppState>,
    username: &str,
    tag: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let body = format!("username={username}&tag={tag}&limit=50");
    post_form(state, "/api/list_user_posts_by_tag", body, cookie).await
}

async fn list_local_timeline_form(
    state: Arc<storage::AppState>,
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut parts = vec![format!("limit={limit}")];
    if let (Some(created_at), Some(post_id)) = (cursor_created_at, cursor_post_id) {
        parts.push(format!("cursor_created_at={created_at}"));
        parts.push(format!("cursor_post_id={post_id}"));
    }
    post_form(state, "/api/list_local_timeline", parts.join("&"), cookie).await
}

async fn list_home_feed_form(
    state: Arc<storage::AppState>,
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut parts = vec![format!("limit={limit}")];
    if let (Some(created_at), Some(post_id)) = (cursor_created_at, cursor_post_id) {
        parts.push(format!("cursor_created_at={created_at}"));
        parts.push(format!("cursor_post_id={post_id}"));
    }
    post_form(state, "/api/list_home_feed", parts.join("&"), cookie).await
}

#[apply(backends)]
#[tokio::test]
async fn update_post_updates_draft_content_and_slug(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "original",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    let post_id = created.post_id;

    // Title embedded as # heading; slug_override takes precedence over the derived slug
    let (status, body) = update_post_json(
        Arc::clone(&state),
        post_id,
        "# Updated Title

**new body**",
        "markdown",
        Some("updated-slug"),
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let updated: UpdatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(updated.slug, "updated-slug");
    assert!(updated.published_at.is_none());

    let record = state
        .posts
        .get_post_by_id(post_id, &common::visibility::ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title.as_deref(), Some("Updated Title"));
    assert_eq!(record.slug.to_string(), "updated-slug");
    assert!(record
        .rendered_html
        .as_ref()
        .contains("<strong>new body</strong>"));
}

#[apply(backends)]
#[tokio::test]
async fn update_post_freezes_slug_when_published(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    let post_id = created.post_id;
    let original_slug = created.slug.clone();

    let (status, body) = update_post_json(
        Arc::clone(&state),
        post_id,
        "new body",
        "markdown",
        Some("new-slug"),
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let updated: UpdatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(
        updated.slug, original_slug,
        "slug must not change after publication"
    );
    assert!(updated.published_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn update_post_publishes_draft(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "draft body",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert!(created.published_at.is_none());
    let post_id = created.post_id;

    let (status, body) = update_post_json(
        Arc::clone(&state),
        post_id,
        "draft body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let updated: UpdatePostResult = serde_json::from_str(&body).unwrap();
    assert!(updated.published_at.is_some());
    assert!(!updated.permalink.as_ref().is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn update_post_rejects_non_author(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );
    let stranger_cookie = session_cookie(
        &state
            .sessions
            .create_session(stranger_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "body",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = update_post_json(
        Arc::clone(&state),
        created.post_id,
        "hacked",
        "markdown",
        None,
        false,
        Some(&stranger_cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

// Shape B — update_post rejection cluster. Identical setup (author + session +
// a freshly created draft) and assertion structure (INTERNAL_SERVER_ERROR +
// body substring); only the update body/format and expected message vary. The
// initial draft body is immaterial to the assertion, so it is fixed.
#[apply(backends_matrix)]
#[case::empty_post("", "markdown", "post body or title is required")]
#[case::invalid_format("body", "invalid_format", "post format must be")]
#[tokio::test]
async fn update_post_rejects(
    backend: Backend,
    #[case] update_body: &str,
    #[case] update_format: &str,
    #[case] expected: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "original",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = update_post_json(
        Arc::clone(&state),
        created.post_id,
        update_body,
        update_format,
        None,
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains(expected), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_post_returns_not_found_for_missing_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = update_post_json(
        Arc::clone(&state),
        PostId::from(99999),
        "body",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_post_returns_not_found_for_deleted_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "body",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    state.posts.soft_delete_post(created.post_id).await.unwrap();

    let (status, body) = update_post_json(
        Arc::clone(&state),
        created.post_id,
        "body",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_drafts_returns_current_user_drafts_with_cursor_pagination(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );
    let stranger_cookie = session_cookie(
        &state
            .sessions
            .create_session(stranger_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "first",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let first_draft: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "second",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let second_draft: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "visible",
        "markdown",
        None,
        true,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "private",
        "markdown",
        None,
        false,
        Some(&stranger_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) =
        list_drafts_form(Arc::clone(&state), None, None, 1, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: Vec<DraftSummary> = serde_json::from_str(&body).unwrap();
    assert_eq!(first_page.len(), 1, "body: {body}");
    let first_entry = &first_page[0];
    assert!(
        first_entry.post_id == first_draft.post_id || first_entry.post_id == second_draft.post_id,
        "unexpected post_id on first page: {body}"
    );

    let (status, body) = list_drafts_form(
        Arc::clone(&state),
        Some(first_entry.created_at),
        Some(first_entry.post_id),
        10,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: Vec<DraftSummary> = serde_json::from_str(&body).unwrap();
    assert_eq!(second_page.len(), 1, "body: {body}");
    let second_entry = &second_page[0];

    assert_ne!(first_entry.post_id, second_entry.post_id);
    let mut ids = vec![first_entry.post_id, second_entry.post_id];
    ids.sort_unstable_by_key(|id| i64::from(*id));
    let mut expected_ids = vec![first_draft.post_id, second_draft.post_id];
    expected_ids.sort_unstable_by_key(|id| i64::from(*id));
    assert_eq!(ids, expected_ids);
}

// A future-scheduled post is surfaced through `list_drafts` with a populated
// `scheduled_at`, while a live post stays off the drafts surface (issue #70).
#[apply(backends)]
#[tokio::test]
async fn list_drafts_surfaces_scheduled_with_marker_excludes_live(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );

    // Seed a scheduled post (future `published_at`) and a live post (past)
    // directly via storage — the web compose datetime control is Task 6.
    let scheduled_post =
        |slug: &str, published_at: chrono::DateTime<chrono::Utc>| storage::CreatePostInput {
            user_id: author_id,
            title: Some(format!("Post {slug}").into()),
            slug: slug.parse().unwrap(),
            body: "body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
            published_at: Some(published_at),
            summary: None,
            audiences: vec![common::visibility::AudienceTarget::Public],
            idempotency_key: None,
        };
    let now = chrono::Utc::now();
    let sched_id = state
        .posts
        .create_post(&scheduled_post(
            "sched-web",
            now + chrono::Duration::days(3),
        ))
        .await
        .unwrap();
    let live_id = state
        .posts
        .create_post(&scheduled_post("live-web", now - chrono::Duration::days(1)))
        .await
        .unwrap();

    let (status, body) =
        list_drafts_form(Arc::clone(&state), None, None, 50, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let drafts: Vec<DraftSummary> = serde_json::from_str(&body).unwrap();

    let sched = drafts
        .iter()
        .find(|d| d.post_id == sched_id)
        .unwrap_or_else(|| panic!("scheduled post must appear in drafts: {body}"));
    assert!(
        sched.scheduled_at.is_some(),
        "scheduled post must carry scheduled_at: {body}"
    );
    assert!(
        !drafts.iter().any(|d| d.post_id == live_id),
        "live post must not appear in drafts: {body}"
    );
}

// A future `publish_at` on create schedules the post: storage records the exact
// future instant and the post stays off the public timeline until then (#70).
#[apply(backends)]
#[tokio::test]
async fn create_post_with_future_publish_at_is_scheduled(#[case] backend: Backend) {
    use chrono::TimeZone;
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let future = chrono::Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).unwrap();
    let payload = serde_json::json!({
        "args": {
            "body": "scheduled body",
            "format": "markdown",
            "publish": true,
            "publish_at": future.to_rfc3339(),
        }
    });
    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/create_post",
        payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.published_at, Some(future));

    // The scheduled post is invisible on the public timeline at "now".
    let published = state
        .posts
        .list_published(
            None,
            50,
            &common::visibility::ViewerIdentity::Anonymous,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    assert!(
        !published.iter().any(|p| p.post_id == created.post_id),
        "scheduled post must not appear in the public timeline"
    );
}

// Publishing without a `publish_at` goes live immediately: the post is stamped
// ~now and appears on the public timeline (#70).
#[apply(backends)]
#[tokio::test]
async fn create_post_publish_without_publish_at_is_live_now(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let cookie = session_cookie(&token);

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "live now body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("post should exist");
    let published_at = record
        .published_at
        .expect("published post has published_at");
    let now = chrono::Utc::now();
    assert!(
        (now - published_at).num_seconds().abs() < 60,
        "publish-now should stamp ~now, got {published_at}"
    );

    let published = state
        .posts
        .list_published(
            None,
            50,
            &common::visibility::ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    assert!(
        published.iter().any(|p| p.post_id == created.post_id),
        "publish-now post must appear in the public timeline"
    );
}

#[apply(backends)]
#[tokio::test]
async fn publish_post_publishes_draft_and_returns_permalink(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "draft body",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert!(created.published_at.is_none());

    let (status, body) =
        publish_post_form(Arc::clone(&state), created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "publish body: {body}");
    let published: PublishPostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(published.post_id, created.post_id);
    assert!(published.permalink.contains("/~author/"));

    let record = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(record.published_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn publish_post_rejects_non_author(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );
    let stranger_cookie = session_cookie(
        &state
            .sessions
            .create_session(stranger_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "secret",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = publish_post_form(state, created.post_id, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

// Shape B — invalid-cursor cluster across the four cursor-paginated endpoints.
// Each fires two requests: a half-specified cursor (a valid instant with no
// `cursor_post_id`, rejected by the handler's "must be provided together"
// pairing check) and an unparseable timestamp. The latter is now a typed
// `Option<UtcInstant>` wire arg (ADR-0065), so an unparseable value fails at
// arg-decode — before the handler body — rather than reaching the handler's
// "invalid cursor_created_at" check; we assert only that the request is
// rejected. Only the endpoint URI and the (already username-encoded where
// required) request bodies vary. An author session is always created and
// passed — the public endpoints ignore it but still run the same cursor
// validation, so a single setup serves every row without branching.
#[apply(backends_matrix)]
#[case::list_drafts(
    "/api/list_drafts",
    "cursor_created_at=2026-04-16T10:11:12%2B00:00&limit=10",
    "cursor_created_at=bad-time&cursor_post_id=10&limit=10"
)]
#[case::list_user_posts(
    "/api/list_user_posts",
    "username=author&cursor_created_at=2026-04-16T10:11:12%2B00:00&limit=10",
    "username=author&cursor_created_at=bad-time&cursor_post_id=12&limit=10"
)]
#[case::list_local_timeline(
    "/api/list_local_timeline",
    "cursor_created_at=2026-04-16T10:11:12%2B00:00&limit=10",
    "cursor_created_at=bad-time&cursor_post_id=12&limit=10"
)]
#[case::list_home_feed(
    "/api/list_home_feed",
    "cursor_created_at=2026-04-16T10:11:12%2B00:00&limit=10",
    "cursor_created_at=bad-time&cursor_post_id=12&limit=10"
)]
#[tokio::test]
async fn list_rejects_invalid_cursor_inputs(
    backend: Backend,
    #[case] uri: &str,
    #[case] half_cursor_body: &str,
    #[case] bad_time_body: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(user_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        uri,
        half_cursor_body.to_string(),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("must be provided together"), "body: {body}");

    let (status, body) = post_form(state, uri, bad_time_body.to_string(), Some(&cookie)).await;
    // An unparseable instant is rejected at typed-arg decode (ADR-0065), before
    // the handler runs — a hard decode error, not the handler's validation
    // message. Assert the request fails rather than pinning the decode-layer text.
    assert_ne!(status, StatusCode::OK, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn publish_post_returns_not_found_for_missing_or_deleted_posts(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) =
        publish_post_form(Arc::clone(&state), PostId::from(999_999), Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "body",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    state.posts.soft_delete_post(created.post_id).await.unwrap();

    let (status, body) = publish_post_form(state, created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_post_finds_author_draft_across_multiple_pages(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );

    let ids = storage::test_support::seed_posts(&state, author_id, 55, false).await;
    let first_post_id = ids[0];
    let record = state
        .posts
        .get_post_by_id(
            first_post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .expect("first draft should exist");

    let (status, body) = get_post_form(
        state,
        "author",
        record.created_at.year(),
        record.created_at.month(),
        record.created_at.day(),
        record.slug.as_ref(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("\"is_draft\":true"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_returns_published_posts_with_cursor_pagination(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let other_id = state
        .users
        .create_user(
            &"other".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );
    let other_cookie = session_cookie(
        &state
            .sessions
            .create_session(other_id, "test session")
            .await
            .unwrap(),
    );

    storage::test_support::seed_posts(&state, author_id, 51, true).await;

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "private",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "body",
        "markdown",
        None,
        true,
        Some(&other_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) =
        list_user_posts_form(Arc::clone(&state), "author", None, None, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(first_page.posts.len(), 50, "body: {body}");
    assert!(first_page.has_more, "body: {body}");
    assert!(first_page.next_cursor_created_at.is_some(), "body: {body}");
    assert!(first_page.next_cursor_post_id.is_some(), "body: {body}");
    assert!(
        first_page.posts.iter().all(|post| post
            .permalink
            .as_ref()
            .is_some_and(|p| p.starts_with("/~author/"))),
        "body: {body}"
    );
    assert!(
        first_page.posts.iter().all(|post| post
            .title
            .as_deref()
            .is_none_or(|title| !title.contains("Draft"))),
        "body: {body}"
    );

    let (status, body) = list_user_posts_form(
        Arc::clone(&state),
        "author",
        first_page.next_cursor_created_at,
        first_page.next_cursor_post_id,
        50,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(second_page.posts.len(), 1, "body: {body}");
    assert!(!second_page.has_more, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_rejects_invalid_username(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = list_user_posts_form(state, "Invalid Name", None, None, 50, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("username"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_local_timeline_returns_published_posts_with_cursor_pagination(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let other_id = state
        .users
        .create_user(
            &"other".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );
    storage::test_support::seed_posts(&state, author_id, 26, true).await;
    storage::test_support::seed_posts(&state, other_id, 26, true).await;

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "private",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "gone",
        "markdown",
        None,
        true,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let deleted: CreatePostResult = serde_json::from_str(&body).unwrap();
    state.posts.soft_delete_post(deleted.post_id).await.unwrap();

    let (status, body) = list_local_timeline_form(Arc::clone(&state), None, None, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(first_page.posts.len(), 50, "body: {body}");
    assert!(first_page.has_more, "body: {body}");
    assert!(first_page.next_cursor_created_at.is_some(), "body: {body}");
    assert!(first_page.next_cursor_post_id.is_some(), "body: {body}");
    assert!(
        first_page
            .posts
            .iter()
            .any(|post| post.username == "author"),
        "body: {body}"
    );
    assert!(
        first_page.posts.iter().any(|post| post.username == "other"),
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

    let (status, body) = list_local_timeline_form(
        Arc::clone(&state),
        first_page.next_cursor_created_at,
        first_page.next_cursor_post_id,
        50,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(second_page.posts.len(), 2, "body: {body}");
    assert!(!second_page.has_more, "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_home_feed_returns_authenticated_users_published_posts_only(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let other_id = state
        .users
        .create_user(
            &"other".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );
    let other_cookie = session_cookie(
        &state
            .sessions
            .create_session(other_id, "test session")
            .await
            .unwrap(),
    );

    storage::test_support::seed_posts(&state, author_id, 51, true).await;

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "private",
        "markdown",
        None,
        false,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    for i in 0..3 {
        let request_body = format!("# Post {i}\n\nbody");
        let (status, body) = create_post_json(
            Arc::clone(&state),
            &request_body,
            "markdown",
            None,
            true,
            Some(&other_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create body: {body}");
    }

    let (status, body) =
        list_home_feed_form(Arc::clone(&state), None, None, 50, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let first_page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(first_page.posts.len(), 50, "body: {body}");
    assert!(first_page.has_more, "body: {body}");
    assert!(first_page.next_cursor_created_at.is_some(), "body: {body}");
    assert!(first_page.next_cursor_post_id.is_some(), "body: {body}");
    assert!(
        first_page
            .posts
            .iter()
            .all(|post| post.username == "author"),
        "body: {body}"
    );
    assert!(
        first_page.posts.iter().all(|post| post
            .title
            .as_deref()
            .is_none_or(|title| { !title.contains("Other") && !title.contains("Draft") })),
        "body: {body}"
    );

    let (status, body) = list_home_feed_form(
        Arc::clone(&state),
        first_page.next_cursor_created_at,
        first_page.next_cursor_post_id,
        50,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second_page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(second_page.posts.len(), 1, "body: {body}");
    assert!(!second_page.has_more, "body: {body}");
}

async fn delete_post_form(
    state: Arc<storage::AppState>,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_form(
        state,
        "/api/delete_post",
        format!("post_id={post_id}"),
        cookie,
    )
    .await
}

#[apply(backends)]
#[tokio::test]
async fn delete_post_soft_deletes_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "gone",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = delete_post_form(Arc::clone(&state), created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // The post should now be gone from storage (deleted_at is set)
    let post = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(post.deleted_at.is_some(), "expected deleted_at to be set");
}

#[apply(backends)]
#[tokio::test]
async fn delete_post_rejects_non_author(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );
    let stranger_cookie = session_cookie(
        &state
            .sessions
            .create_session(stranger_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "mine",
        "markdown",
        None,
        true,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) =
        delete_post_form(Arc::clone(&state), created.post_id, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn delete_post_rejects_unauthenticated(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = delete_post_form(state, created.post_id, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn delete_post_returns_not_found_for_already_deleted_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = delete_post_form(Arc::clone(&state), created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "first delete body: {body}");

    let (status, body) = delete_post_form(state, created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn deleted_post_excluded_from_timelines_and_returns_404_at_permalink(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "# Deletable Post

body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    let permalink = String::from(created.permalink);

    // Verify post appears in user timeline before deletion
    let (status, body) =
        list_user_posts_form(Arc::clone(&state), "author", None, None, 10, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Deletable Post"), "expected post in timeline");

    let (status, body) = delete_post_form(Arc::clone(&state), created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");

    // Verify excluded from user timeline
    let (status, body) =
        list_user_posts_form(Arc::clone(&state), "author", None, None, 10, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        !body.contains("Deletable Post"),
        "expected post excluded from timeline: {body}"
    );

    // Verify excluded from local timeline
    let (status, body) = list_local_timeline_form(Arc::clone(&state), None, None, 10, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        !body.contains("Deletable Post"),
        "expected post excluded from local timeline: {body}"
    );

    // Extract year/month/day/slug from permalink for get_post call
    // permalink format: /~username/year/month/day/slug
    let parts: Vec<&str> = permalink.trim_start_matches('/').split('/').collect();
    // parts: ["~author", "year", "month", "day", "slug"]
    let year: i32 = parts[1].parse().unwrap();
    let month: u32 = parts[2].parse().unwrap();
    let day: u32 = parts[3].parse().unwrap();
    let slug = parts[4];

    let (status, body) =
        get_post_form(Arc::clone(&state), "author", year, month, day, slug, None).await;
    assert_eq!(StatusCode::NOT_FOUND, status, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn unpublish_post_reverts_published_post_to_draft(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "# Unpublish Me

body",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert!(created.published_at.is_some(), "should be published");

    let (status, body) =
        unpublish_post_form(Arc::clone(&state), created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "unpublish body: {body}");

    // Should no longer appear in the user timeline
    let (status, body) =
        list_user_posts_form(Arc::clone(&state), "author", None, None, 10, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        !body.contains("Unpublish Me"),
        "expected post removed from timeline: {body}"
    );

    // Should appear in drafts
    let (status, body) = list_drafts_form(Arc::clone(&state), None, None, 50, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        body.contains("unpublish-me"),
        "expected post in drafts: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn unpublish_post_rejects_non_author(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let other_id = state
        .users
        .create_user(
            &"other".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let author_cookie = session_cookie(
        &state
            .sessions
            .create_session(author_id, "test session")
            .await
            .unwrap(),
    );
    let other_cookie = session_cookie(
        &state
            .sessions
            .create_session(other_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "# Others Post

body",
        "markdown",
        None,
        true,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = unpublish_post_form(state, created.post_id, Some(&other_cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_carries_tags_per_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(user_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "# Tagged Post\n\nbody",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    // Apply two tags via the storage layer (the create_post tags param lands
    // in tags.5; here we just verify the timeline surface threads them
    // through).
    state
        .posts
        .tag_post(created.post_id, &"Rust".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    state
        .posts
        .tag_post(created.post_id, &"web".parse::<TagLabel>().unwrap())
        .await
        .unwrap();

    let (status, body) =
        list_user_posts_form(Arc::clone(&state), "author", None, None, 50, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "list body: {body}");
    let page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(page.posts.len(), 1);
    let post = &page.posts[0];
    let slugs: Vec<&str> = post.tags.iter().map(|t| t.slug.as_ref()).collect();
    assert_eq!(slugs, vec!["rust", "web"]);
    // Display casing is preserved (author-provided).
    assert!(post.tags.iter().any(|t| t.display == "Rust"));
}

#[apply(backends)]
#[tokio::test]
async fn get_post_carries_tags(#[case] backend: Backend) {
    use chrono::Datelike;

    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(user_id, "test session")
            .await
            .unwrap(),
    );

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "# Tagged Post\n\nbody",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    state
        .posts
        .tag_post(created.post_id, &"Performance".parse::<TagLabel>().unwrap())
        .await
        .unwrap();

    let published_at = state
        .posts
        .get_post_by_id(
            created.post_id,
            &common::visibility::ViewerIdentity::Anonymous,
        )
        .await
        .unwrap()
        .unwrap()
        .published_at
        .unwrap();

    let (status, body) = get_post_form(
        Arc::clone(&state),
        "author",
        published_at.year(),
        published_at.month(),
        published_at.day(),
        &created.slug,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get body: {body}");
    let response: web::posts::PostResponse = serde_json::from_str(&body).unwrap();
    assert_eq!(response.tags.len(), 1);
    assert_eq!(response.tags[0].slug, "performance");
    assert_eq!(response.tags[0].display, "Performance");
}

async fn login_and_state(backend: Backend) -> (TestBase, Arc<storage::AppState>, String) {
    let TestEnv { state, base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let cookie = session_cookie(
        &state
            .sessions
            .create_session(user_id, "test session")
            .await
            .unwrap(),
    );
    (base, state, cookie)
}

#[apply(backends)]
#[tokio::test]
async fn create_post_applies_tags_from_param(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    let payload = serde_json::json!({
        "args": {
            "body": "# Tagged via API\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": true,
            "tags": ["Rust", "web-dev"],
        }
    });
    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/create_post",
        payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let stored_tags = state
        .posts
        .get_tags_for_post(created.post_id)
        .await
        .unwrap();
    let slugs: Vec<&str> = stored_tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(slugs, vec!["rust", "web-dev"]);
    assert!(stored_tags.iter().any(|t| t.tag_display == "Rust"));
}

#[apply(backends)]
#[tokio::test]
async fn create_post_rejects_invalid_tag_token(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    let payload = serde_json::json!({
        "args": {
            "body": "# Bad Tag\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": true,
            "tags": ["rust", "not a valid tag!"],
        }
    });
    let (status, body) = post_json(state, "/api/create_post", payload, Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    // The invalid token is now rejected at the wire→TagLabel parse, surfacing
    // InvalidTagLabel's own message (the single validation source) rather than the
    // retired TagValidationError::Invalid.
    assert!(body.contains("tag must be non-empty"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn create_post_rejects_more_than_25_tags(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;
    let many: Vec<String> = (0..26).map(|n| format!("tag{n}")).collect();

    let payload = serde_json::json!({
        "args": {
            "body": "# Too Many\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": true,
            "tags": many,
        }
    });
    let (status, body) = post_json(state, "/api/create_post", payload, Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("too many tags"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_post_applies_tag_set_diff(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    // Create with two tags.
    let create_payload = serde_json::json!({
        "args": {
            "body": "# Diff Me\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": false,
            "tags": ["rust", "old-tag"],
        }
    });
    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/create_post",
        create_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    // Update: replace old-tag with new-tag, keep rust.
    let update_payload = serde_json::json!({
        "args": {
            "post_id": created.post_id,
            "body": "# Diff Me\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": false,
            "tags": ["rust", "new-tag"],
        }
    });
    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/update_post",
        update_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update body: {body}");

    let stored = state
        .posts
        .get_tags_for_post(created.post_id)
        .await
        .unwrap();
    let slugs: Vec<&str> = stored.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(slugs, vec!["new-tag", "rust"]);
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag_returns_matching_posts_from_all_users(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Two authors each post twice; only some posts get the target tag.
    let alice_id = state
        .users
        .create_user(
            &"alice".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let alice_cookie = session_cookie(
        &state
            .sessions
            .create_session(alice_id, "test session")
            .await
            .unwrap(),
    );
    let bob_id = state
        .users
        .create_user(
            &"bob".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let bob_cookie = session_cookie(
        &state
            .sessions
            .create_session(bob_id, "test session")
            .await
            .unwrap(),
    );

    let create = |cookie: String, body: &'static str, tags: serde_json::Value| {
        let state = Arc::clone(&state);
        async move {
            let payload = serde_json::json!({
                "args": {
                    "body": body,
                    "format": "markdown",
                    "slug_override": null,
                    "publish": true,
                    "tags": tags,
                }
            });
            let (status, body) = post_json(state, "/api/create_post", payload, Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK, "create body: {body}");
            serde_json::from_str::<CreatePostResult>(&body).unwrap()
        }
    };

    create(
        alice_cookie.clone(),
        "# Alice A\n\nbody",
        serde_json::json!(["rust", "web"]),
    )
    .await;
    create(
        alice_cookie,
        "# Alice B\n\nbody",
        serde_json::json!(["rust"]),
    )
    .await;
    create(
        bob_cookie.clone(),
        "# Bob A\n\nbody",
        serde_json::json!(["rust", "perf"]),
    )
    .await;
    create(
        bob_cookie,
        "# Bob B\n\nbody",
        serde_json::json!(["javascript"]),
    )
    .await;

    let (status, body) = list_posts_by_tag_form(Arc::clone(&state), "rust", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: TimelinePage = serde_json::from_str(&body).unwrap();
    // Three posts carry the "rust" tag, across both authors.
    assert_eq!(page.posts.len(), 3);
    let usernames: std::collections::HashSet<&str> =
        page.posts.iter().map(|p| p.username.as_ref()).collect();
    assert!(usernames.contains("alice"));
    assert!(usernames.contains("bob"));
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag_returns_empty_for_unknown_tag(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = list_posts_by_tag_form(state, "rust", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert!(page.posts.is_empty());
    assert!(!page.has_more);
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_scopes_to_user(#[case] backend: Backend) {
    let (_base, state, alice_cookie) = login_and_state(backend).await;
    let bob_id = state
        .users
        .create_user(
            &"bob".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let bob_cookie = session_cookie(
        &state
            .sessions
            .create_session(bob_id, "test session")
            .await
            .unwrap(),
    );

    // Alice ("author") + Bob each post with shared tag.
    let create = |cookie: String, body: &'static str| {
        let state = Arc::clone(&state);
        async move {
            let payload = serde_json::json!({
                "args": {
                    "body": body,
                    "format": "markdown",
                    "slug_override": null,
                    "publish": true,
                    "tags": ["shared"],
                }
            });
            let (status, body) = post_json(state, "/api/create_post", payload, Some(&cookie)).await;
            assert_eq!(status, StatusCode::OK, "create body: {body}");
        }
    };
    create(alice_cookie, "# Author Post\n\nbody").await;
    create(bob_cookie, "# Bob Post\n\nbody").await;

    let (status, body) =
        list_user_posts_by_tag_form(Arc::clone(&state), "author", "shared", None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(page.posts.len(), 1);
    assert_eq!(page.posts[0].username, "author");
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_unknown_user_returns_not_found(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = list_user_posts_by_tag_form(state, "nobody", "rust", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("user"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_post_with_tags_unset_leaves_existing_tags_alone(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    // Create with one tag.
    let create_payload = serde_json::json!({
        "args": {
            "body": "# Untouched\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": false,
            "tags": ["keep"],
        }
    });
    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/create_post",
        create_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    // Update without including the tags key (None on the server side).
    let update_payload = serde_json::json!({
        "args": {
            "post_id": created.post_id,
            "body": "# Untouched edited\n\nbody",
            "format": "markdown",
            "slug_override": null,
            "publish": false,
        }
    });
    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/update_post",
        update_payload,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update body: {body}");

    let stored = state
        .posts
        .get_tags_for_post(created.post_id)
        .await
        .unwrap();
    let slugs: Vec<&str> = stored.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(slugs, vec!["keep"]);
}

#[apply(backends)]
#[tokio::test]
async fn get_default_post_format_returns_markdown_by_default(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/get_default_post_format",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get body: {body}");
    assert_eq!(
        body, "\"markdown\"",
        "expected default format to be markdown"
    );
}

#[apply(backends)]
#[tokio::test]
async fn set_default_post_format_persists_and_retrieves_markdown(#[case] backend: Backend) {
    let (_base, state, cookie) = login_and_state(backend).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/set_default_post_format",
        "format=markdown",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "set body: {body}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/get_default_post_format",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "get body: {body}");
    assert_eq!(
        body, "\"markdown\"",
        "expected format to be markdown after setting"
    );
}

// ---------------------------------------------------------------------------
// Content visibility — Layer A (Task 16): timeline reads thread the real
// viewer (viewer_identity) through the store resolution filter instead of the
// Anonymous stopgap. These are server-fn-level tests; the exhaustive storage
// resolution matrix lives in `storage.rs`.
// ---------------------------------------------------------------------------

/// Creates a published post for `author` with the given audience targeting,
/// directly through the store (the web create path is Public-only in Layer A).
async fn create_targeted_post(
    state: &Arc<storage::AppState>,
    author: UserId,
    slug: &str,
    audiences: Vec<common::visibility::AudienceTarget>,
) -> PostId {
    state
        .posts
        .create_post(&storage::CreatePostInput {
            user_id: author,
            title: Some(format!("Post {slug}").into()),
            slug: slug.parse().unwrap(),
            body: "body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
            published_at: Some(chrono::Utc::now()),
            summary: None,
            audiences,
            idempotency_key: None,
        })
        .await
        .unwrap()
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

    let author = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let subscriber = state
        .users
        .create_user(
            &"subby".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let stranger = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();

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

    create_targeted_post(&state, author, "public-post", vec![AudienceTarget::Public]).await;
    create_targeted_post(
        &state,
        author,
        "subscribers-post",
        vec![AudienceTarget::Subscribers],
    )
    .await;
    create_targeted_post(
        &state,
        author,
        "named-post",
        vec![AudienceTarget::Named(friends)],
    )
    .await;
    create_targeted_post(&state, author, "private-post", vec![]).await;

    let author_cookie = session_cookie(&state.sessions.create_session(author, "s").await.unwrap());
    let subscriber_cookie = session_cookie(
        &state
            .sessions
            .create_session(subscriber, "s")
            .await
            .unwrap(),
    );
    let stranger_cookie =
        session_cookie(&state.sessions.create_session(stranger, "s").await.unwrap());

    // Anonymous viewer: only the Public post.
    let (status, body) = list_local_timeline_form(Arc::clone(&state), None, None, 50, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let anon: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(
        timeline_slugs(&anon),
        ["public-post".to_string()].into_iter().collect(),
        "anonymous viewer sees only Public; body: {body}"
    );

    // Author: sees all of their own posts, including the private one.
    let (status, body) =
        list_local_timeline_form(Arc::clone(&state), None, None, 50, Some(&author_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let authored: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(
        timeline_slugs(&authored),
        [
            "public-post".to_string(),
            "subscribers-post".to_string(),
            "named-post".to_string(),
            "private-post".to_string(),
        ]
        .into_iter()
        .collect(),
        "author sees own posts regardless of audience; body: {body}"
    );

    // Active subscriber + named member: Public + Subscribers + Named (not Private).
    let (status, body) =
        list_local_timeline_form(Arc::clone(&state), None, None, 50, Some(&subscriber_cookie))
            .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let sub: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(
        timeline_slugs(&sub),
        [
            "public-post".to_string(),
            "subscribers-post".to_string(),
            "named-post".to_string(),
        ]
        .into_iter()
        .collect(),
        "subscriber sees Public + Subscribers + admitted Named; body: {body}"
    );
    assert!(
        sub.posts.iter().all(|p| !p.is_author),
        "subscriber is not the author; body: {body}"
    );

    // Authed non-subscriber: only the Public post (same reach as anonymous,
    // proving viewer_identity yields a Channel viewer that is correctly *not*
    // admitted to subscriber/named content).
    let (status, body) =
        list_local_timeline_form(Arc::clone(&state), None, None, 50, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let stranger_page: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(
        timeline_slugs(&stranger_page),
        ["public-post".to_string()].into_iter().collect(),
        "authed non-subscriber sees only Public; body: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn single_post_permalink_hides_subscribers_post_from_anonymous(#[case] backend: Backend) {
    use common::visibility::AudienceTarget;

    let TestEnv { state, base: _base } = backend.setup().await;
    let author = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let subscriber = state
        .users
        .create_user(
            &"subby".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();

    let local = state.subscriptions.local_channel_id().await.unwrap();
    state
        .subscriptions
        .subscribe(author, local, &i64::from(subscriber).to_string())
        .await
        .unwrap();

    let post_id = create_targeted_post(
        &state,
        author,
        "subs-only",
        vec![AudienceTarget::Subscribers],
    )
    .await;
    let post = state
        .posts
        .get_post_by_id(
            post_id,
            &common::visibility::ViewerIdentity::local(author, local),
        )
        .await
        .unwrap()
        .unwrap();
    let published = post.published_at.unwrap();
    let (y, m, d) = (published.year(), published.month(), published.day());

    // Anonymous → 404 (the resolution filter hides the subscribers-only post).
    let (status, _body) =
        get_post_form(Arc::clone(&state), "author", y, m, d, "subs-only", None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "anonymous must not see subscribers-only post"
    );

    // Active subscriber → 200.
    let subscriber_cookie = session_cookie(
        &state
            .sessions
            .create_session(subscriber, "s")
            .await
            .unwrap(),
    );
    let (status, body) = get_post_form(
        Arc::clone(&state),
        "author",
        y,
        m,
        d,
        "subs-only",
        Some(&subscriber_cookie),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "subscriber must see subscribers-only post; body: {body}"
    );
}

// ── Audience-picker server fns ────────────────────────────────

/// Creates `author` and returns a session cookie for the audience-picker tests.
async fn author_with_cookie(state: &Arc<storage::AppState>) -> String {
    user_with_cookie(state, "author").await
}

/// Creates a user with `username` and returns a session cookie.
async fn user_with_cookie(state: &Arc<storage::AppState>, username: &str) -> String {
    let user_id = state
        .users
        .create_user(
            &username.parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    session_cookie(&token)
}

#[apply(backends)]
#[tokio::test]
async fn default_audience_selection_returns_public_by_default(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = author_with_cookie(&state).await;

    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/default_audience_selection",
        serde_json::json!({}),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let selection: AudienceSelection = serde_json::from_str(&body).unwrap();
    assert_eq!(selection.base, AudienceBase::Public);
    assert!(selection.named.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn default_audience_selection_rejects_unauthenticated(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = post_json(
        Arc::clone(&state),
        "/api/default_audience_selection",
        serde_json::json!({}),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn post_audience_selection_returns_public_for_new_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = author_with_cookie(&state).await;

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "Hello",
        "markdown",
        None,
        true,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/post_audience_selection",
        format!("post_id={}", created.post_id),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let selection: AudienceSelection = serde_json::from_str(&body).unwrap();
    // A post created with no audience field defaults to Public.
    assert_eq!(selection.base, AudienceBase::Public);
    assert!(selection.named.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn post_audience_selection_rejects_missing_post(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = author_with_cookie(&state).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/post_audience_selection",
        "post_id=99999".to_string(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn post_audience_selection_rejects_non_owner(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let author_cookie = user_with_cookie(&state, "author").await;
    let other_cookie = user_with_cookie(&state, "intruder").await;

    let (status, body) = create_post_json(
        Arc::clone(&state),
        "Hello",
        "markdown",
        None,
        true,
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    // A different user must not learn another author's targeting.
    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/post_audience_selection",
        format!("post_id={}", created.post_id),
        Some(&other_cookie),
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}
