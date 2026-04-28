mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::Datelike;
use common::storage::PostFormat;
use tempfile::TempDir;
use tower::ServiceExt;
use web::posts::{
    CreatePostResult, DraftSummary, PublishPostResult, TimelinePage, UpdatePostResult,
};

async fn unpublish_post_form(
    state: Arc<jaunder::storage::AppState>,
    post_id: i64,
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

use helpers::{ensure_server_fns_registered, test_options, test_state};

async fn post_form(
    state: Arc<jaunder::storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    ensure_server_fns_registered();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder.body(Body::from(body.into())).unwrap();

    let app = jaunder::create_router(test_options(), state, true);
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();

    (status, body_str)
}

async fn get_post_form(
    state: Arc<jaunder::storage::AppState>,
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
    state: Arc<jaunder::storage::AppState>,
    post_id: i64,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let body = format!("post_id={post_id}");
    post_form(state, "/api/get_post_preview", body, cookie).await
}

#[tokio::test]
async fn create_post_persists_rendered_published_post() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    // Title embedded as # heading in the body (verbatim storage)
    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=%23+Hello+World%0A%0A%2A%2Abold%2A%2A&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "hello-world");
    assert!(created.published_at.is_some());
    assert_eq!(
        created.preview_url,
        format!("/draft/{}/preview", created.post_id)
    );
    assert!(created.permalink.is_some());

    let record = state
        .posts
        .get_post_by_id(created.post_id)
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title.as_deref(), Some("Hello World"));
    assert_eq!(record.slug.to_string(), "hello-world");
    assert_eq!(record.format, PostFormat::Markdown);
    assert!(record.published_at.is_some());
    assert!(
        record.rendered_html.contains("<strong>bold</strong>"),
        "rendered_html: {}",
        record.rendered_html
    );
    let published_at = record.published_at.expect("published post");
    let expected_permalink = format!(
        "/~author/{:04}/{:02}/{:02}/{}",
        published_at.year(),
        published_at.month(),
        published_at.day(),
        record.slug.as_str()
    );
    assert_eq!(
        created.permalink.as_deref(),
        Some(expected_permalink.as_str())
    );
}

#[tokio::test]
async fn create_post_retries_slug_conflicts_for_same_user_and_date() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    // Title embedded as # heading; two posts with same heading produce conflicting slugs
    let (first_status, first_body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=%23+Repeated+Title%0A%0Afirst&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;
    assert_eq!(first_status, StatusCode::OK, "body: {first_body}");

    let (second_status, second_body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=%23+Repeated+Title%0A%0Asecond&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;

    assert_eq!(second_status, StatusCode::OK, "body: {second_body}");
    let created: CreatePostResult = serde_json::from_str(&second_body).unwrap();
    assert_eq!(created.slug, "repeated-title-2");
}

#[tokio::test]
async fn create_post_rejects_requests_without_authentication() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = post_form(
        state,
        "/api/create_post",
        "title=Unauthorized&body=body&format=markdown&publish=false",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[tokio::test]
async fn create_post_accepts_slug_override_and_saves_draft() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Draft+Post&body=%2Abold%2A&format=org&slug_override=Custom-Slug&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "custom-slug");
    assert!(created.published_at.is_none());
    assert_eq!(
        created.preview_url,
        format!("/draft/{}/preview", created.post_id)
    );
    assert!(created.permalink.is_none());

    let record = state
        .posts
        .get_post_by_id(created.post_id)
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.slug.to_string(), "custom-slug");
    assert_eq!(record.format, PostFormat::Org);
    assert!(record.published_at.is_none());
    assert!(record.rendered_html.contains("<b>bold</b>"));
}

#[tokio::test]
async fn create_post_accepts_titleless_body() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=&body=Titleless+note&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "titleless-note");
    let record = state
        .posts
        .get_post_by_id(created.post_id)
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title, None);
    assert_eq!(record.body, "Titleless note");
}

#[tokio::test]
async fn create_post_extracts_markdown_heading_title() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=%23+Extracted+Title%0A%0ABody+text&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(created.slug, "extracted-title");
    let record = state
        .posts
        .get_post_by_id(created.post_id)
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title.as_deref(), Some("Extracted Title"));
    // Body is stored verbatim including the heading
    assert_eq!(record.body, "# Extracted Title\n\nBody text");
    // Rendered HTML contains the heading because body is rendered verbatim
    assert!(record.rendered_html.contains("<h1>Extracted Title</h1>"));
}

#[tokio::test]
async fn create_post_rejects_empty_post() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        state,
        "/api/create_post",
        "body=&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("post body is required"), "body: {body}");
}

#[tokio::test]
async fn create_post_rejects_post_without_slug_source() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        state,
        "/api/create_post",
        "title=%2B%2B%2B&body=%2B%2B%2B&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(
        body.contains("post must contain at least one ASCII letter or digit for its slug"),
        "body: {body}"
    );
}

#[tokio::test]
async fn create_post_rejects_invalid_format() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        state,
        "/api/create_post",
        "title=Bad+Format&body=body&format=html&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("post format must be"), "body: {body}");
}

#[tokio::test]
async fn create_post_rejects_invalid_slug_override() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        state,
        "/api/create_post",
        "title=Invalid+Slug&body=body&format=markdown&slug_override=Not Valid&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("slug must be non-empty"), "body: {body}");
}

#[tokio::test]
async fn create_post_rejects_title_without_ascii_slug_characters() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    // Heading with only em-dashes passes the empty check but cannot produce a slug
    let (status, body) = post_form(
        state,
        "/api/create_post",
        "body=%23+%E2%80%94%E2%80%94%E2%80%94%0A%0Abody&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(
        body.contains("post must contain at least one ASCII letter or digit for its slug"),
        "body: {body}"
    );
}

#[tokio::test]
async fn get_post_returns_published_post() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=%23+Permalink%0A%0A%2A%2Abold%2A%2A&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let record = state
        .posts
        .get_post_by_id(created.post_id)
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

#[tokio::test]
async fn get_post_returns_draft_to_author_only() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );
    let stranger_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(stranger_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=%23+Draft%0A%0Adraft&format=markdown&publish=false",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    let record = state
        .posts
        .get_post_by_id(created.post_id)
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

#[tokio::test]
async fn get_post_preview_shows_draft_to_author_only() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );
    let stranger_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(stranger_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=%23+Preview+Draft%0A%0Adraft&format=markdown&publish=false",
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

#[tokio::test]
async fn get_post_hides_drafts_from_guests() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Draft&body=draft&format=markdown&publish=false",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    let record = state
        .posts
        .get_post_by_id(created.post_id)
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

#[tokio::test]
async fn get_post_rejects_invalid_username() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = get_post_form(state, "Invalid Name", 2024, 1, 1, "missing", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("username"), "body: {body}");
}

#[tokio::test]
async fn get_post_rejects_invalid_slug() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = get_post_form(state, "author", 2024, 1, 1, "Invalid Slug", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("slug"), "body: {body}");
}

#[tokio::test]
async fn get_post_returns_not_found_for_missing_post() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = get_post_form(state, "author", 2024, 1, 1, "missing", None).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

async fn update_post_form(
    state: Arc<jaunder::storage::AppState>,
    post_id: i64,
    extra_params: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let body = format!("post_id={}&{}", post_id, extra_params);
    post_form(state, "/api/update_post", body, cookie).await
}

async fn list_drafts_form(
    state: Arc<jaunder::storage::AppState>,
    cursor_created_at: Option<&str>,
    cursor_post_id: Option<i64>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut parts = vec![format!("limit={limit}")];
    if let (Some(created_at), Some(post_id)) = (cursor_created_at, cursor_post_id) {
        parts.push(format!(
            "cursor_created_at={}",
            created_at.replace('+', "%2B")
        ));
        parts.push(format!("cursor_post_id={post_id}"));
    }
    post_form(state, "/api/list_drafts", parts.join("&"), cookie).await
}

async fn publish_post_form(
    state: Arc<jaunder::storage::AppState>,
    post_id: i64,
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
    state: Arc<jaunder::storage::AppState>,
    username: &str,
    cursor_created_at: Option<&str>,
    cursor_post_id: Option<i64>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut parts = vec![format!("username={username}"), format!("limit={limit}")];
    if let (Some(created_at), Some(post_id)) = (cursor_created_at, cursor_post_id) {
        parts.push(format!(
            "cursor_created_at={}",
            created_at.replace('+', "%2B")
        ));
        parts.push(format!("cursor_post_id={post_id}"));
    }
    post_form(state, "/api/list_user_posts", parts.join("&"), cookie).await
}

async fn list_local_timeline_form(
    state: Arc<jaunder::storage::AppState>,
    cursor_created_at: Option<&str>,
    cursor_post_id: Option<i64>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut parts = vec![format!("limit={limit}")];
    if let (Some(created_at), Some(post_id)) = (cursor_created_at, cursor_post_id) {
        parts.push(format!(
            "cursor_created_at={}",
            created_at.replace('+', "%2B")
        ));
        parts.push(format!("cursor_post_id={post_id}"));
    }
    post_form(state, "/api/list_local_timeline", parts.join("&"), cookie).await
}

async fn list_home_feed_form(
    state: Arc<jaunder::storage::AppState>,
    cursor_created_at: Option<&str>,
    cursor_post_id: Option<i64>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut parts = vec![format!("limit={limit}")];
    if let (Some(created_at), Some(post_id)) = (cursor_created_at, cursor_post_id) {
        parts.push(format!(
            "cursor_created_at={}",
            created_at.replace('+', "%2B")
        ));
        parts.push(format!("cursor_post_id={post_id}"));
    }
    post_form(state, "/api/list_home_feed", parts.join("&"), cookie).await
}

#[tokio::test]
async fn update_post_updates_draft_content_and_slug() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=original&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    let post_id = created.post_id;

    // Title embedded as # heading; slug_override takes precedence over the derived slug
    let (status, body) = update_post_form(
        Arc::clone(&state),
        post_id,
        "body=%23+Updated+Title%0A%0A%2A%2Anew+body%2A%2A&format=markdown&slug_override=updated-slug&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let updated: UpdatePostResult = serde_json::from_str(&body).unwrap();
    assert_eq!(updated.slug, "updated-slug");
    assert!(updated.published_at.is_none());

    let record = state
        .posts
        .get_post_by_id(post_id)
        .await
        .unwrap()
        .expect("post should exist");
    assert_eq!(record.title.as_deref(), Some("Updated Title"));
    assert_eq!(record.slug.to_string(), "updated-slug");
    assert!(record.rendered_html.contains("<strong>new body</strong>"));
}

#[tokio::test]
async fn update_post_freezes_slug_when_published() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Published+Post&body=body&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    let post_id = created.post_id;
    let original_slug = created.slug.clone();

    let (status, body) = update_post_form(
        Arc::clone(&state),
        post_id,
        "title=Changed+Title&body=new+body&format=markdown&slug_override=new-slug&publish=true",
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

#[tokio::test]
async fn update_post_publishes_draft() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Draft+Post&body=draft+body&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert!(created.published_at.is_none());
    let post_id = created.post_id;

    let (status, body) = update_post_form(
        Arc::clone(&state),
        post_id,
        "title=Draft+Post&body=draft+body&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "update body: {body}");
    let updated: UpdatePostResult = serde_json::from_str(&body).unwrap();
    assert!(updated.published_at.is_some());
    assert!(updated.permalink.is_some());
}

#[tokio::test]
async fn update_post_rejects_non_author() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );
    let stranger_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(stranger_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Authors+Post&body=body&format=markdown&publish=false",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = update_post_form(
        Arc::clone(&state),
        created.post_id,
        "title=Stolen+Title&body=hacked&format=markdown&publish=false",
        Some(&stranger_cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[tokio::test]
async fn update_post_rejects_unauthenticated() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = update_post_form(
        state,
        42,
        "title=Unauthorized&body=body&format=markdown&publish=false",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[tokio::test]
async fn update_post_rejects_empty_post() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Original&body=body&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = update_post_form(
        state,
        created.post_id,
        "title=&body=&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(
        body.contains("post body or title is required"),
        "body: {body}"
    );
}

#[tokio::test]
async fn update_post_rejects_invalid_format() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Original&body=body&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = update_post_form(
        state,
        created.post_id,
        "title=Updated&body=body&format=html&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("post format must be"), "body: {body}");
}

#[tokio::test]
async fn update_post_returns_not_found_for_missing_post() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = update_post_form(
        state,
        99999,
        "title=Does+Not+Exist&body=body&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[tokio::test]
async fn update_post_returns_not_found_for_deleted_post() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Delete+Me&body=body&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    state.posts.soft_delete_post(created.post_id).await.unwrap();

    let (status, body) = update_post_form(
        state,
        created.post_id,
        "title=Delete+Me&body=body&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[tokio::test]
async fn update_post_rejects_title_without_ascii_slug_characters() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let token = state.sessions.create_session(user_id, None).await.unwrap();
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=original&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    // Heading with only em-dashes passes the empty check but cannot produce a slug
    let (status, body) = update_post_form(
        state,
        created.post_id,
        "body=%23+%E2%80%94%E2%80%94%E2%80%94%0A%0Abody&format=markdown&publish=false",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(
        body.contains("post must contain at least one ASCII letter or digit for its slug"),
        "body: {body}"
    );
}

#[tokio::test]
async fn list_drafts_returns_current_user_drafts_with_cursor_pagination() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );
    let stranger_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(stranger_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Draft+One&body=first&format=markdown&publish=false",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let first_draft: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Draft+Two&body=second&format=markdown&publish=false",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let second_draft: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Published&body=visible&format=markdown&publish=true",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Stranger+Draft&body=private&format=markdown&publish=false",
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
        Some(&first_entry.created_at),
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
    ids.sort_unstable();
    let mut expected_ids = vec![first_draft.post_id, second_draft.post_id];
    expected_ids.sort_unstable();
    assert_eq!(ids, expected_ids);
}

#[tokio::test]
async fn publish_post_publishes_draft_and_returns_permalink() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Publish+Me&body=draft+body&format=markdown&publish=false",
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
        .get_post_by_id(created.post_id)
        .await
        .unwrap()
        .unwrap();
    assert!(record.published_at.is_some());
}

#[tokio::test]
async fn publish_post_rejects_non_author() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );
    let stranger_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(stranger_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Private+Draft&body=secret&format=markdown&publish=false",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = publish_post_form(state, created.post_id, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}

#[tokio::test]
async fn list_drafts_rejects_unauthenticated_requests() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = list_drafts_form(state, None, None, 10, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[tokio::test]
async fn list_drafts_rejects_invalid_cursor_inputs() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let cookie = format!(
        "session={}",
        state.sessions.create_session(user_id, None).await.unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/list_drafts",
        "cursor_created_at=2026-04-16T10:11:12%2B00:00&limit=10",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("must be provided together"), "body: {body}");

    let (status, body) = post_form(
        state,
        "/api/list_drafts",
        "cursor_created_at=bad-time&cursor_post_id=10&limit=10",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("invalid cursor_created_at"), "body: {body}");
}

#[tokio::test]
async fn publish_post_rejects_unauthenticated_requests() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = publish_post_form(state, 99, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[tokio::test]
async fn publish_post_returns_not_found_for_missing_or_deleted_posts() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );

    let (status, body) = publish_post_form(Arc::clone(&state), 999_999, Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Delete+Before+Publish&body=body&format=markdown&publish=false",
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

#[tokio::test]
async fn get_post_finds_author_draft_across_multiple_pages() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );

    let mut first_post_id = None;
    for i in 0..55 {
        let (status, body) = post_form(
            Arc::clone(&state),
            "/api/create_post",
            format!("title=Draft+{i}&body=body&format=markdown&publish=false"),
            Some(&cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create body: {body}");
        let created: CreatePostResult = serde_json::from_str(&body).unwrap();
        if first_post_id.is_none() {
            first_post_id = Some(created.post_id);
        }
    }

    let first_post_id = first_post_id.expect("at least one draft should be created");
    let record = state
        .posts
        .get_post_by_id(first_post_id)
        .await
        .unwrap()
        .expect("first draft should exist");

    let (status, body) = get_post_form(
        state,
        "author",
        record.created_at.year(),
        record.created_at.month(),
        record.created_at.day(),
        record.slug.as_str(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("\"is_draft\":true"), "body: {body}");
}

#[tokio::test]
async fn list_user_posts_returns_published_posts_with_cursor_pagination() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let other_id = state
        .users
        .create_user(
            &"other".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );
    let other_cookie = format!(
        "session={}",
        state.sessions.create_session(other_id, None).await.unwrap()
    );

    for i in 0..51 {
        let (status, body) = post_form(
            Arc::clone(&state),
            "/api/create_post",
            format!("title=Author+Published+{i}&body=body&format=markdown&publish=true"),
            Some(&author_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create body: {body}");
    }

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Author+Draft&body=private&format=markdown&publish=false",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Other+Published&body=body&format=markdown&publish=true",
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
        first_page
            .posts
            .iter()
            .all(|post| post.permalink.starts_with("/~author/")),
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
        first_page.next_cursor_created_at.as_deref(),
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

#[tokio::test]
async fn list_user_posts_rejects_invalid_username() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = list_user_posts_form(state, "Invalid Name", None, None, 50, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("username"), "body: {body}");
}

#[tokio::test]
async fn list_user_posts_rejects_invalid_cursor_inputs() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/list_user_posts",
        "username=author&cursor_created_at=2026-04-16T10:11:12%2B00:00&limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("must be provided together"), "body: {body}");

    let (status, body) = post_form(
        state,
        "/api/list_user_posts",
        "username=author&cursor_created_at=bad-time&cursor_post_id=12&limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("invalid cursor_created_at"), "body: {body}");
}

#[tokio::test]
async fn list_local_timeline_returns_published_posts_with_cursor_pagination() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let other_id = state
        .users
        .create_user(
            &"other".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );
    let other_cookie = format!(
        "session={}",
        state.sessions.create_session(other_id, None).await.unwrap()
    );

    for i in 0..26 {
        let (status, body) = post_form(
            Arc::clone(&state),
            "/api/create_post",
            format!("title=Author+Timeline+{i}&body=body&format=markdown&publish=true"),
            Some(&author_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create body: {body}");

        let (status, body) = post_form(
            Arc::clone(&state),
            "/api/create_post",
            format!("title=Other+Timeline+{i}&body=body&format=markdown&publish=true"),
            Some(&other_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create body: {body}");
    }

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Timeline+Draft&body=private&format=markdown&publish=false",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Timeline+Deleted&body=gone&format=markdown&publish=true",
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
            .all(|post| post.permalink.starts_with("/~")),
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
        first_page.next_cursor_created_at.as_deref(),
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

#[tokio::test]
async fn list_local_timeline_rejects_invalid_cursor_inputs() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/list_local_timeline",
        "cursor_created_at=2026-04-16T10:11:12%2B00:00&limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("must be provided together"), "body: {body}");

    let (status, body) = post_form(
        state,
        "/api/list_local_timeline",
        "cursor_created_at=bad-time&cursor_post_id=12&limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("invalid cursor_created_at"), "body: {body}");
}

#[tokio::test]
async fn list_home_feed_returns_authenticated_users_published_posts_only() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let other_id = state
        .users
        .create_user(
            &"other".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );
    let other_cookie = format!(
        "session={}",
        state.sessions.create_session(other_id, None).await.unwrap()
    );

    for i in 0..51 {
        let (status, body) = post_form(
            Arc::clone(&state),
            "/api/create_post",
            format!("title=Home+Feed+{i}&body=body&format=markdown&publish=true"),
            Some(&author_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create body: {body}");
    }

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Author+Home+Draft&body=private&format=markdown&publish=false",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");

    for i in 0..3 {
        let (status, body) = post_form(
            Arc::clone(&state),
            "/api/create_post",
            format!("title=Other+Home+{i}&body=body&format=markdown&publish=true"),
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
        first_page.next_cursor_created_at.as_deref(),
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

#[tokio::test]
async fn list_home_feed_rejects_unauthenticated_requests() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, body) = list_home_feed_form(state, None, None, 50, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[tokio::test]
async fn list_home_feed_rejects_invalid_cursor_inputs() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/list_home_feed",
        "cursor_created_at=2026-04-16T10:11:12%2B00:00&limit=10",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("must be provided together"), "body: {body}");

    let (status, body) = post_form(
        state,
        "/api/list_home_feed",
        "cursor_created_at=bad-time&cursor_post_id=12&limit=10",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("invalid cursor_created_at"), "body: {body}");
}

async fn delete_post_form(
    state: Arc<jaunder::storage::AppState>,
    post_id: i64,
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

#[tokio::test]
async fn delete_post_soft_deletes_post() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=To+Delete&body=gone&format=markdown&publish=true",
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
        .get_post_by_id(created.post_id)
        .await
        .unwrap()
        .unwrap();
    assert!(post.deleted_at.is_some(), "expected deleted_at to be set");
}

#[tokio::test]
async fn delete_post_rejects_non_author() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let stranger_id = state
        .users
        .create_user(
            &"stranger".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );
    let stranger_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(stranger_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Owned+Post&body=mine&format=markdown&publish=true",
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

#[tokio::test]
async fn delete_post_rejects_unauthenticated() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Protected&body=body&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = delete_post_form(state, created.post_id, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[tokio::test]
async fn delete_post_returns_not_found_for_already_deleted_post() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "title=Once+Only&body=body&format=markdown&publish=true",
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

#[tokio::test]
async fn deleted_post_excluded_from_timelines_and_returns_404_at_permalink() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=%23+Deletable+Post%0A%0Abody&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    let permalink = created.permalink.unwrap();

    // Verify post appears in user timeline before deletion
    let (status, body) =
        list_user_posts_form(Arc::clone(&state), "author", None, None, 10, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(body.contains("Deletable Post"), "expected post in timeline");

    // Delete the post
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

#[tokio::test]
async fn unpublish_post_reverts_published_post_to_draft() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=%23+Unpublish+Me%0A%0Abody&format=markdown&publish=true",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();
    assert!(created.published_at.is_some(), "should be published");

    // Unpublish
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

#[tokio::test]
async fn unpublish_post_rejects_non_author() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    let author_id = state
        .users
        .create_user(
            &"author".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let other_id = state
        .users
        .create_user(
            &"other".parse().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let author_cookie = format!(
        "session={}",
        state
            .sessions
            .create_session(author_id, None)
            .await
            .unwrap()
    );
    let other_cookie = format!(
        "session={}",
        state.sessions.create_session(other_id, None).await.unwrap()
    );

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/create_post",
        "body=%23+Others+Post%0A%0Abody&format=markdown&publish=true",
        Some(&author_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create body: {body}");
    let created: CreatePostResult = serde_json::from_str(&body).unwrap();

    let (status, body) = unpublish_post_form(state, created.post_id, Some(&other_cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
}
