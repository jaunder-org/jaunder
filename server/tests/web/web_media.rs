use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use server_fn::ServerFn;
use tempfile::TempDir;
use tower::ServiceExt;
use web::media::{DeleteResult, Item, UsageData};

use chrono::Utc;
use storage::{CreateMediaError, MediaRecord};

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_user_and_session, make_app, post_form, post_multipart, MultipartFile};
use common::media::{MaxFileSize, MediaSource, UploadResponse, UserQuota};
use common::test_support::{
    parse_byte_size, parse_content_hash, parse_content_type, parse_filename,
};
use storage::test_support::{
    backends, backends_matrix, noop_mailer, Backend, SeedRawPost, TestEnv,
};

// ─── media_usage ──────────────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn media_usage_returns_defaults_for_authenticated_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = post_form(
        &state,
        <web::media::GetUsage as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let usage: UsageData = serde_json::from_str(&body).expect("response should be valid JSON");
    assert_eq!(usage.used_bytes, parse_byte_size("0"));
    // No media config is set, so the getters return the type defaults (1 GiB / 50 MiB),
    // carried unchanged across the wire by the transparent-i64 serde bridge.
    assert_eq!(usage.quota_bytes, UserQuota::default());
    assert_eq!(usage.max_file_size_bytes, MaxFileSize::default());
}

// Shape B — every media server-fn refuses an unauthenticated request the same
// way (Leptos server fn → INTERNAL_SERVER_ERROR + "unauthorized"); only the
// endpoint and request body vary.
#[apply(backends_matrix)]
#[case::media_usage(<web::media::GetUsage as ServerFn>::PATH, "")]
#[case::list_my_media(<web::media::ListMine as ServerFn>::PATH, "")]
#[case::delete_media(<web::media::Delete as ServerFn>::PATH, "sha256=deadbeef00000000000000000000000000000000000000000000000000000000&filename=test.png&source=upload")]
#[tokio::test]
async fn media_endpoint_rejects_unauthenticated_request(
    backend: Backend,
    #[case] uri: &str,
    #[case] body: &str,
) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = post_form(&state, uri, body, None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

// ─── list_my_media ────────────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn list_my_media_returns_empty_for_new_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let (status, body) = post_form(
        &state,
        <web::media::ListMine as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let items: Vec<Item> = serde_json::from_str(&body).expect("response should be valid JSON");
    assert!(items.is_empty(), "expected no media items for new user");
}

#[apply(backends)]
#[tokio::test]
async fn list_my_media_rejects_out_of_range_limit(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    // `limit=999` is outside PageSize's `1..=50`; the typed wire arg rejects it on
    // deserialization instead of fetching an unbounded page.
    let (status, _body) = post_form(
        &state,
        <web::media::ListMine as ServerFn>::PATH,
        "limit=999",
        Some(&cookie),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "out-of-range media limit must be rejected"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_my_media_returns_inserted_item(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    let record = MediaRecord {
        user_id: session.user_id,
        sha256: parse_content_hash(
            "aabbccdd11223344000000000000000000000000000000000000000000000000",
        ),
        filename: parse_filename("photo.jpg"),
        source: MediaSource::Upload,
        content_type: parse_content_type("image/jpeg"),
        size_bytes: parse_byte_size("1024"),
        source_url: None,
        created_at: Utc::now(),
    };
    match state.media.create_media(&record).await {
        Ok(()) | Err(CreateMediaError::AlreadyExists) => {}
        Err(e) => panic!("create_media failed: {e}"),
    }

    let cookie = session.cookie();

    let (status, body) = post_form(
        &state,
        <web::media::ListMine as ServerFn>::PATH,
        "",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let items: Vec<Item> = serde_json::from_str(&body).expect("response should be valid JSON");
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
    let session = create_user_and_session(&state).await;

    let record = MediaRecord {
        user_id: session.user_id,
        sha256: parse_content_hash(
            "ff00ee11dd22cc33000000000000000000000000000000000000000000000000",
        ),
        filename: parse_filename("clip.mp4"),
        source: MediaSource::Upload,
        content_type: parse_content_type("video/mp4"),
        size_bytes: parse_byte_size("512"),
        source_url: None,
        created_at: Utc::now(),
    };
    match state.media.create_media(&record).await {
        Ok(()) | Err(CreateMediaError::AlreadyExists) => {}
        Err(e) => panic!("create_media failed: {e}"),
    }

    let cookie = session.cookie();

    let (status, body) = post_form(
        &state,
        <web::media::ListMine as ServerFn>::PATH,
        "source=upload",
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let items: Vec<Item> = serde_json::from_str(&body).expect("response should be valid JSON");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].source, MediaSource::Upload);
}

// ─── delete_media ─────────────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn delete_media_succeeds_for_existing_item(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    // Insert a media record directly so delete has something to act on.
    let record = MediaRecord {
        user_id: session.user_id,
        sha256: parse_content_hash(
            "deadbeef01234567000000000000000000000000000000000000000000000000",
        ),
        filename: parse_filename("test.png"),
        source: MediaSource::Upload,
        content_type: parse_content_type("image/png"),
        size_bytes: parse_byte_size("42"),
        source_url: None,
        created_at: Utc::now(),
    };
    match state.media.create_media(&record).await {
        Ok(()) | Err(CreateMediaError::AlreadyExists) => {}
        Err(e) => panic!("create_media failed: {e}"),
    }

    let cookie = session.cookie();

    let body = "sha256=deadbeef01234567000000000000000000000000000000000000000000000000&filename=test.png&source=upload&force=false";
    let (status, body_str) = post_form(
        &state,
        <web::media::Delete as ServerFn>::PATH,
        body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body_str}");
    let result: DeleteResult =
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
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let user_id = session.user_id;

    let media_url = common::media::media_url(
        &MediaSource::Upload,
        &parse_content_hash("deadbeef99999999000000000000000000000000000000000000000000000000"),
        &parse_filename("inline.png"),
    );
    let record = MediaRecord {
        user_id,
        sha256: parse_content_hash(
            "deadbeef99999999000000000000000000000000000000000000000000000000",
        ),
        filename: parse_filename("inline.png"),
        source: MediaSource::Upload,
        content_type: parse_content_type("image/png"),
        size_bytes: parse_byte_size("42"),
        source_url: None,
        created_at: Utc::now(),
    };
    match state.media.create_media(&record).await {
        Ok(()) | Err(CreateMediaError::AlreadyExists) => {}
        Err(e) => panic!("create_media failed: {e}"),
    }

    let post = SeedRawPost::new(user_id)
        .body(format!("![inline]({media_url})"))
        .seed(&state)
        .await;

    let cookie = session.cookie();

    let body = "sha256=deadbeef99999999000000000000000000000000000000000000000000000000&filename=inline.png&source=upload&force=false";
    let (status, body_str) = post_form(
        &state,
        <web::media::Delete as ServerFn>::PATH,
        body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body_str}");
    let result: DeleteResult =
        serde_json::from_str(&body_str).expect("response should be valid JSON");
    assert!(
        !result.deleted,
        "delete without force should refuse when media is referenced by a post"
    );
    assert_eq!(
        result.referenced_in_posts,
        vec![post.post_id],
        "referenced_in_posts should list the referencing post"
    );
}

// ─── upload_media ─────────────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn upload_media_stores_file_and_returns_metadata(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    // A real writable root so the upload lands on disk (separate from the DB backend).
    let storage = TempDir::new().unwrap();
    let (status, body) = post_multipart(
        &state,
        &storage,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "photo.jpg",
            content_type: "image/jpeg",
            bytes: b"fake jpeg data",
        },
        Some(&cookie),
    )
    .await;

    // The server fn returns 200 with the bare `UploadResponse` JSON — not the old
    // `/media/upload` handler's 201.
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let resp: UploadResponse = serde_json::from_str(&body).expect("response should be valid JSON");
    assert_eq!(resp.filename, "photo.jpg");
    assert_eq!(resp.content_type, "image/jpeg");
    assert!(resp.url.contains("/media/upload/"), "url: {}", resp.url);
}

#[apply(backends)]
#[tokio::test]
async fn upload_then_serve_round_trips_a_filename_needing_encoding(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();
    let storage = TempDir::new().unwrap();

    // A space is a *legal* `Filename` — `sanitize_filename` permits it — so this is an
    // ordinary upload, not a hostile one. Before #675 the derived URL carried the raw
    // space, which `RootRelativeUrl` cannot even represent.
    let (status, body) = post_multipart(
        &state,
        &storage,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "my photo.jpg",
            content_type: "image/jpeg",
            bytes: b"fake jpeg data",
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let resp: UploadResponse = serde_json::from_str(&body).expect("response should be valid JSON");

    // The display name stays raw; only the URL/disk segment is encoded.
    assert_eq!(resp.filename, "my photo.jpg");
    assert!(resp.url.contains("my%20photo.jpg"), "url: {}", resp.url);
    assert!(!resp.url.contains(' '), "url: {}", resp.url);

    // The property that actually matters, and the one no unit test can reach: fetching the
    // URL we just handed the client returns the bytes we stored. It fails if the writer's
    // spelling of the name on disk and the reader's ever diverge again.
    let app = make_app(&state, &storage);
    let request = Request::builder()
        .method("GET")
        .uri(resp.url.to_string())
        .body(Body::empty())
        .expect("failed to build request");
    let response = app.oneshot(request).await.expect("router oneshot failed");
    assert_eq!(response.status(), StatusCode::OK, "serving {}", resp.url);
    let served = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    assert_eq!(&served[..], b"fake jpeg data");
}

#[apply(backends)]
#[tokio::test]
async fn upload_then_serve_survives_a_name_too_long_to_store(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();
    let storage = TempDir::new().unwrap();

    // 200 `ä` is 400 raw bytes and ~1200 once percent-encoded — far past the filesystem's
    // 255-byte per-component limit. Before #708 this reached the file write and failed with
    // an opaque 500; the name is otherwise perfectly legal.
    let long_name = format!("{}.jpg", "ä".repeat(200));
    let (status, body) = post_multipart(
        &state,
        &storage,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: &long_name,
            content_type: "image/jpeg",
            bytes: b"long name content",
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let resp: UploadResponse = serde_json::from_str(&body).expect("response should be valid JSON");

    // Truncated, not rejected — and the extension survived, so the detected content type is
    // still an image rather than octet-stream.
    assert!(resp.filename.len() < long_name.len(), "must truncate");
    assert!(resp.filename.ends_with(".jpg"), "{}", resp.filename);
    assert_eq!(resp.content_type, "image/jpeg");

    // The point of the test: the file actually landed and is served back at the URL handed
    // to the client.
    let app = make_app(&state, &storage);
    let request = Request::builder()
        .method("GET")
        .uri(resp.url.to_string())
        .body(Body::empty())
        .expect("failed to build request");
    let response = app.oneshot(request).await.expect("router oneshot failed");
    assert_eq!(response.status(), StatusCode::OK, "serving {}", resp.url);
    let served = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should be readable");
    assert_eq!(&served[..], b"long name content");
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_rejects_unauthenticated_request(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let storage = TempDir::new().unwrap();
    let (status, body) = post_multipart(
        &state,
        &storage,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "photo.jpg",
            content_type: "image/jpeg",
            bytes: b"fake jpeg data",
        },
        None,
    )
    .await;

    // Same shape as the sibling media fns: the Leptos server-fn auth-error path is
    // a 500 carrying "unauthorized", not a bare 401.
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(body.contains("unauthorized"), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_rejects_invalid_filename(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    let storage = TempDir::new().unwrap();
    // `..` sanitizes to empty → `MediaError::BadRequest`, exercising `map_media_error`'s
    // BadRequest arm (projected to `WebError::Validation`).
    let (status, body) = post_multipart(
        &state,
        &storage,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "..",
            content_type: "image/jpeg",
            bytes: b"fake jpeg data",
        },
        Some(&cookie),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "invalid filename must be rejected: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_rejects_oversized_file(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    // Cap the max file size at 5 bytes so a 14-byte upload trips PayloadTooLarge,
    // exercising `map_media_error`'s PayloadTooLarge arm.
    state
        .site_config
        .set(storage::MEDIA_MAX_FILE_SIZE_BYTES_KEY, "5")
        .await
        .unwrap();
    let cookie = create_user_and_session(&state).await.cookie();

    let storage = TempDir::new().unwrap();
    let (status, body) = post_multipart(
        &state,
        &storage,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "big.jpg",
            content_type: "image/jpeg",
            bytes: b"fake jpeg data",
        },
        Some(&cookie),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "oversized file must be rejected: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_rejects_over_quota_file(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    // A 5-byte user quota with a 14-byte upload trips InsufficientStorage, exercising
    // `map_media_error`'s InsufficientStorage arm.
    state
        .site_config
        .set(storage::MEDIA_USER_QUOTA_BYTES_KEY, "5")
        .await
        .unwrap();
    let cookie = create_user_and_session(&state).await.cookie();

    let storage = TempDir::new().unwrap();
    let (status, body) = post_multipart(
        &state,
        &storage,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "big.jpg",
            content_type: "image/jpeg",
            bytes: b"fake jpeg data",
        },
        Some(&cookie),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "over-quota file must be rejected: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_rejects_missing_file_field(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();

    // An empty multipart body (a closing boundary with no field) yields
    // `next_field() == None`, exercising the "no file field" guard.
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);
    let boundary = "----testboundary1234";
    let body = format!("--{boundary}--\r\n");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(<web::media::Upload as ServerFn>::PATH)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .header(header::COOKIE, cookie)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(
        response.status(),
        StatusCode::OK,
        "a multipart body with no file field must be rejected"
    );
}

// ─── serve_handler hash validation (security: §2.2) ────────────

async fn media_serve_get(state: &Arc<storage::AppState>, uri: &str) -> StatusCode {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("failed to build request");

    let app = jaunder::create_router(
        Arc::clone(state),
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

    let status = media_serve_get(&state, &uri).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}
