mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use tempfile::TempDir;
use tower::ServiceExt;
use web::media::{DeleteMediaResult, MediaItem, MediaUsageData};

use chrono::Utc;
use storage::{CreateMediaError, MediaRecord, MediaSource};

use helpers::{ensure_server_fns_registered, test_options, test_state};

async fn post_form(
    state: Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    ensure_server_fns_registered();
    server_fn::axum::register_explicit::<web::media::ListMyMedia>();
    server_fn::axum::register_explicit::<web::media::MediaUsage>();
    server_fn::axum::register_explicit::<web::media::DeleteMedia>();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder
        .body(Body::from(body.into()))
        .expect("failed to build request");

    let app = jaunder::create_router(
        test_options(),
        state,
        helpers::noop_mailer(),
        true,
        helpers::tmp_storage_path(),
    );
    let response = app.oneshot(request).await.expect("router oneshot failed");

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_str = String::from_utf8(bytes.to_vec()).expect("response body is not UTF-8");

    (status, body_str)
}

// ─── media_usage ──────────────────────────────────────────────

#[tokio::test]
async fn media_usage_returns_defaults_for_authenticated_user() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
            false,
        )
        .await
        .expect("create_user failed");
    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create_session failed");
    let cookie = format!("session={token}");

    let (status, body) = post_form(Arc::clone(&state), "/api/media_usage", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let usage: MediaUsageData = serde_json::from_str(&body).expect("response should be valid JSON");
    assert_eq!(usage.used_bytes, 0);
    assert!(usage.quota_bytes > 0, "quota_bytes should be positive");
    assert!(
        usage.max_file_size_bytes > 0,
        "max_file_size_bytes should be positive"
    );
}

#[tokio::test]
async fn media_usage_rejects_unauthenticated_request() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;

    let (status, body) = post_form(state, "/api/media_usage", "", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

// ─── list_my_media ────────────────────────────────────────────

#[tokio::test]
async fn list_my_media_returns_empty_for_new_user() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"bob".parse().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
            false,
        )
        .await
        .expect("create_user failed");
    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create_session failed");
    let cookie = format!("session={token}");

    let (status, body) =
        post_form(Arc::clone(&state), "/api/list_my_media", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let items: Vec<MediaItem> = serde_json::from_str(&body).expect("response should be valid JSON");
    assert!(items.is_empty(), "expected no media items for new user");
}

#[tokio::test]
async fn list_my_media_rejects_unauthenticated_request() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;

    let (status, body) = post_form(state, "/api/list_my_media", "", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[tokio::test]
async fn list_my_media_returns_inserted_item() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"dave".parse().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
            false,
        )
        .await
        .expect("create_user failed");

    let record = MediaRecord {
        user_id,
        sha256: "aabbccdd11223344".to_string(),
        filename: "photo.jpg".to_string(),
        source: MediaSource::Upload,
        content_type: "image/jpeg".to_string(),
        size_bytes: 1024,
        source_url: None,
        created_at: Utc::now(),
    };
    match state.media.create_media(&record).await {
        Ok(()) | Err(CreateMediaError::AlreadyExists) => {}
        Err(e) => panic!("create_media failed: {e}"),
    }

    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create_session failed");
    let cookie = format!("session={token}");

    let (status, body) =
        post_form(Arc::clone(&state), "/api/list_my_media", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let items: Vec<MediaItem> = serde_json::from_str(&body).expect("response should be valid JSON");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].filename, "photo.jpg");
    assert!(
        items[0].url.contains("/media/upload/"),
        "url: {}",
        items[0].url
    );
}

#[tokio::test]
async fn list_my_media_with_source_filter() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"eve".parse().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
            false,
        )
        .await
        .expect("create_user failed");

    let record = MediaRecord {
        user_id,
        sha256: "ff00ee11dd22cc33".to_string(),
        filename: "clip.mp4".to_string(),
        source: MediaSource::Upload,
        content_type: "video/mp4".to_string(),
        size_bytes: 512,
        source_url: None,
        created_at: Utc::now(),
    };
    match state.media.create_media(&record).await {
        Ok(()) | Err(CreateMediaError::AlreadyExists) => {}
        Err(e) => panic!("create_media failed: {e}"),
    }

    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create_session failed");
    let cookie = format!("session={token}");

    let (status, body) = post_form(
        Arc::clone(&state),
        "/api/list_my_media",
        "source=upload",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let items: Vec<MediaItem> = serde_json::from_str(&body).expect("response should be valid JSON");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].source, "upload");
}

// ─── delete_media ─────────────────────────────────────────────

#[tokio::test]
async fn delete_media_succeeds_for_existing_item() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"carol".parse().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
            false,
        )
        .await
        .expect("create_user failed");

    // Insert a media record directly so delete has something to act on.
    let record = MediaRecord {
        user_id,
        sha256: "deadbeef01234567".to_string(),
        filename: "test.png".to_string(),
        source: MediaSource::Upload,
        content_type: "image/png".to_string(),
        size_bytes: 42,
        source_url: None,
        created_at: Utc::now(),
    };
    match state.media.create_media(&record).await {
        Ok(()) | Err(CreateMediaError::AlreadyExists) => {}
        Err(e) => panic!("create_media failed: {e}"),
    }

    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create_session failed");
    let cookie = format!("session={token}");

    let body = "sha256=deadbeef01234567&filename=test.png&source=upload&force=false";
    let (status, body_str) =
        post_form(Arc::clone(&state), "/api/delete_media", body, Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body_str}");
    let result: DeleteMediaResult =
        serde_json::from_str(&body_str).expect("response should be valid JSON");
    assert!(
        result.deleted,
        "delete of existing item should report deleted=true"
    );
    assert!(
        result.referenced_in_posts.is_empty(),
        "item not in any posts should have no post references"
    );
}

#[tokio::test]
async fn delete_media_reports_referencing_posts_when_not_forced() {
    use common::slug::Slug;
    use storage::{CreatePostInput, PostFormat};

    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;
    let user_id = state
        .users
        .create_user(
            &"darya".parse().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
            false,
        )
        .await
        .expect("create_user failed");

    let media_url = common::media::media_url("upload", "deadbeef99999999", "inline.png");
    let record = MediaRecord {
        user_id,
        sha256: "deadbeef99999999".to_string(),
        filename: "inline.png".to_string(),
        source: MediaSource::Upload,
        content_type: "image/png".to_string(),
        size_bytes: 42,
        source_url: None,
        created_at: Utc::now(),
    };
    match state.media.create_media(&record).await {
        Ok(()) | Err(CreateMediaError::AlreadyExists) => {}
        Err(e) => panic!("create_media failed: {e}"),
    }

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id,
            title: Some("with media".to_string()),
            slug: "with-media".parse::<Slug>().expect("valid slug"),
            body: format!("![inline]({media_url})"),
            format: PostFormat::Markdown,
            rendered_html: format!("<p><img src=\"{media_url}\"></p>"),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("create_post failed");

    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create_session failed");
    let cookie = format!("session={token}");

    let body = "sha256=deadbeef99999999&filename=inline.png&source=upload&force=false";
    let (status, body_str) =
        post_form(Arc::clone(&state), "/api/delete_media", body, Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body_str}");
    let result: DeleteMediaResult =
        serde_json::from_str(&body_str).expect("response should be valid JSON");
    assert!(
        !result.deleted,
        "delete without force should refuse when media is referenced by a post"
    );
    assert_eq!(
        result.referenced_in_posts,
        vec![post_id],
        "referenced_in_posts should list the referencing post"
    );
}

#[tokio::test]
async fn delete_media_rejects_unauthenticated_request() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;

    let body = "sha256=deadbeef&filename=test.png&source=upload";
    let (status, body_str) = post_form(state, "/api/delete_media", body, None).await;

    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {body_str}"
    );
    assert!(body_str.contains("unauthorized"), "body: {body_str}");
}
