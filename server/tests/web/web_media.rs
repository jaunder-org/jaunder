use common::visibility::AudienceTarget;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;
use web::media::{DeleteMediaResult, MediaItem, MediaUsageData};

use chrono::Utc;
use storage::{CreateMediaError, MediaRecord, MediaSource};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{post_form, session_cookie, test_options};
use common::test_support::parse_content_hash;
use storage::test_support::{backends, backends_matrix, noop_mailer, Backend, TestEnv};

// ─── media_usage ──────────────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn media_usage_returns_defaults_for_authenticated_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
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
        .create_session(user_id, "test session")
        .await
        .expect("create_session failed");
    let cookie = session_cookie(&token);

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

// Shape B — every media server-fn refuses an unauthenticated request the same
// way (Leptos server fn → INTERNAL_SERVER_ERROR + "unauthorized"); only the
// endpoint and request body vary.
#[apply(backends_matrix)]
#[case::media_usage("/api/media_usage", "")]
#[case::list_my_media("/api/list_my_media", "")]
#[case::delete_media("/api/delete_media", "sha256=deadbeef00000000000000000000000000000000000000000000000000000000&filename=test.png&source=upload")]
#[tokio::test]
async fn media_endpoint_rejects_unauthenticated_request(
    backend: Backend,
    #[case] uri: &str,
    #[case] body: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = post_form(state, uri, body, None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

// ─── list_my_media ────────────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn list_my_media_returns_empty_for_new_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
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
        .create_session(user_id, "test session")
        .await
        .expect("create_session failed");
    let cookie = session_cookie(&token);

    let (status, body) =
        post_form(Arc::clone(&state), "/api/list_my_media", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let items: Vec<MediaItem> = serde_json::from_str(&body).expect("response should be valid JSON");
    assert!(items.is_empty(), "expected no media items for new user");
}

#[apply(backends)]
#[tokio::test]
async fn list_my_media_returns_inserted_item(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
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
        sha256: parse_content_hash(
            "aabbccdd11223344000000000000000000000000000000000000000000000000",
        ),
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
        .create_session(user_id, "test session")
        .await
        .expect("create_session failed");
    let cookie = session_cookie(&token);

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

#[apply(backends)]
#[tokio::test]
async fn list_my_media_with_source_filter(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
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
        sha256: parse_content_hash(
            "ff00ee11dd22cc33000000000000000000000000000000000000000000000000",
        ),
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
        .create_session(user_id, "test session")
        .await
        .expect("create_session failed");
    let cookie = session_cookie(&token);

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

#[apply(backends)]
#[tokio::test]
async fn delete_media_succeeds_for_existing_item(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
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
        sha256: parse_content_hash(
            "deadbeef01234567000000000000000000000000000000000000000000000000",
        ),
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
        .create_session(user_id, "test session")
        .await
        .expect("create_session failed");
    let cookie = session_cookie(&token);

    let body = "sha256=deadbeef01234567000000000000000000000000000000000000000000000000&filename=test.png&source=upload&force=false";
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

#[apply(backends)]
#[tokio::test]
async fn delete_media_reports_referencing_posts_when_not_forced(#[case] backend: Backend) {
    use common::slug::Slug;
    use storage::{CreatePostInput, PostFormat, RenderedHtml};

    let TestEnv { state, base: _base } = backend.setup().await;
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

    let media_url = common::media::media_url(
        "upload",
        &parse_content_hash("deadbeef99999999000000000000000000000000000000000000000000000000"),
        "inline.png",
    );
    let record = MediaRecord {
        user_id,
        sha256: parse_content_hash(
            "deadbeef99999999000000000000000000000000000000000000000000000000",
        ),
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
            title: Some("with media".into()),
            slug: "with-media".parse::<Slug>().expect("valid slug"),
            body: format!("![inline]({media_url})").into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted(format!("<p><img src=\"{media_url}\"></p>")),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("create_post failed");

    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .expect("create_session failed");
    let cookie = session_cookie(&token);

    let body = "sha256=deadbeef99999999000000000000000000000000000000000000000000000000&filename=inline.png&source=upload&force=false";
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

// ─── serve_handler hash validation (security: §2.2) ────────────

async fn media_serve_get(state: Arc<storage::AppState>, uri: &str) -> StatusCode {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("failed to build request");

    let app = jaunder::create_router(
        test_options(),
        state,
        noop_mailer(),
        true,
        crate::helpers::tmp_storage_path(),
    );
    app.oneshot(request)
        .await
        .expect("router oneshot failed")
        .status()
}

// Shape B — the serve handler must reject malformed hashes with 404 (not panic
// on `params.hash[2..]`, not accept non-hex). Identical setup + assertion; only
// the malformed URI varies.
//
// `short_hash`: a 1-byte hash historically panicked because the prefix check
// (`hash.starts_with(p1)`) passes and the slice runs off the end of the string.
// `non_hex`: 64 characters but not lowercase hex — not a canonical content hash.
#[apply(backends_matrix)]
#[case::short_hash("/media/upload/a/a/a/file.txt".to_owned())]
#[case::non_hex(format!("/media/upload/zz/zz/{}/file.txt", "z".repeat(64)))]
#[tokio::test]
async fn serve_handler_rejects_malformed_hash(backend: Backend, #[case] uri: String) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let status = media_serve_get(state, &uri).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}
