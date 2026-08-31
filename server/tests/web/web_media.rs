use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use server_fn::ServerFn;
use tempfile::TempDir;
use tower::ServiceExt;
use web::media::{Item, MediaDeletion, UsageData};

use common::time::UtcInstant;
use host::config_key::SiteConfigKey;
use rstest::*;
use rstest_reuse::*;
use storage::{
    CreateMediaError, ForeignEvidenceSink, InstanceId, MediaRecord, MediaReferenceEvidence,
    MediaReferenceOwnershipResolver, PersistedMediaReference, WriteScopeError,
};

use crate::helpers::{
    ForeignReferenceResolver, MultipartFile, create_user_and_session, make_app, post_form,
    post_multipart, post_server_fn, post_server_fn_with_media_ownership_resolver,
    setup_with_base_url,
};
use common::media::{MaxFileSize, MediaReferenceForm, MediaSource, UploadedMedia, UserQuota};
use common::test_support::{
    parse_byte_size, parse_content_hash, parse_content_type, parse_filename, parse_post_body,
};
use storage::test_support::{
    Backend, SeedRawPost, TestEnv, backends, backends_matrix, noop_mailer,
};

async fn create_media(state: &storage::AppState, record: &MediaRecord) {
    let media = state.media.clone();
    let record = record.clone();
    let outcome = match state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move { media.create_media(transaction, &record).await })
        })
        .await
    {
        Ok(outcome) => outcome,
        Err(WriteScopeError::Operation(CreateMediaError::AlreadyExists)) => return,
        Err(error) => unreachable!("create_media returned an unexpected error: {error}"),
    };
    storage::test_support::confirmed_for(outcome, "fixture media creation");
}

fn confirmed_media_deletion(body: &str) -> MediaDeletion {
    storage::test_support::confirmed_for(
        serde_json::from_str(body).expect("response should be a valid mutation outcome"),
        "test fixture media deletion",
    )
}

fn confirmed_upload(body: &str) -> UploadedMedia {
    storage::test_support::confirmed_for(
        serde_json::from_str(body).expect("response should be a valid mutation outcome"),
        "test fixture media upload",
    )
}

struct BlockingOwnershipResolver {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl BlockingOwnershipResolver {
    fn new() -> Self {
        Self {
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        }
    }
}

#[async_trait]
impl MediaReferenceOwnershipResolver for BlockingOwnershipResolver {
    async fn resolve(
        &self,
        _: &[PersistedMediaReference],
        _: &InstanceId,
        _: Option<&common::tagged_url::BaseUrl>,
        foreign: ForeignEvidenceSink,
    ) -> MediaReferenceEvidence {
        self.started.notify_one();
        self.release.notified().await;
        foreign.finish()
    }
}
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
// way (Leptos server fn → INTERNAL_SERVER_ERROR + "unauthorized"). Typed inputs
// keep this gate test independent of hand-encoded transport syntax.
#[apply(backends)]
#[tokio::test]
async fn media_endpoints_reject_unauthenticated_requests(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (usage_status, usage_body) = post_server_fn(&state, &web::media::GetUsage {}, None).await;
    let (list_status, list_body) = post_server_fn(
        &state,
        &web::media::ListMine {
            source: None,
            limit: None,
            offset: None,
        },
        None,
    )
    .await;
    let (delete_status, delete_body) = post_server_fn(
        &state,
        &web::media::Delete {
            request: web::media::DeleteMediaRequest {
                sha256: parse_content_hash(
                    "deadbeef00000000000000000000000000000000000000000000000000000000",
                ),
                filename: parse_filename("test.png"),
                source: MediaSource::Upload,
                force: None,
            },
        },
        None,
    )
    .await;

    for (endpoint, status, body) in [
        ("get_usage", usage_status, usage_body),
        ("list_mine", list_status, list_body),
        ("delete", delete_status, delete_body),
    ] {
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{endpoint}: {body}"
        );
        assert!(body.contains("unauthorized"), "{endpoint}: {body}");
    }
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
        created_at: UtcInstant::now(),
    };
    create_media(&state, &record).await;

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
        created_at: UtcInstant::now(),
    };
    create_media(&state, &record).await;

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
async fn delete_nested_request_maps_identity_without_force(#[case] backend: Backend) {
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
        created_at: UtcInstant::now(),
    };
    create_media(&state, &record).await;

    let cookie = session.cookie();

    let (status, body_str) = post_server_fn(
        &state,
        &web::media::Delete {
            request: web::media::DeleteMediaRequest {
                sha256: record.sha256.clone(),
                filename: record.filename.clone(),
                source: record.source,
                force: None,
            },
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body_str}");
    let result = confirmed_media_deletion(&body_str);
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
async fn delete_nested_request_refuses_referenced_without_force(#[case] backend: Backend) {
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
        created_at: UtcInstant::now(),
    };
    create_media(&state, &record).await;

    let post = SeedRawPost::new(user_id)
        .body(parse_post_body(&format!("![inline]({media_url})")))
        .seed(&state)
        .await;

    let cookie = session.cookie();

    let (status, body_str) = post_server_fn(
        &state,
        &web::media::Delete {
            request: web::media::DeleteMediaRequest {
                sha256: record.sha256.clone(),
                filename: record.filename.clone(),
                source: record.source,
                force: None,
            },
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body_str}");
    let result = confirmed_media_deletion(&body_str);
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
#[apply(backends)]
#[tokio::test]
async fn delete_uses_one_global_live_ownership_snapshot(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let owner = create_user_and_session(&state).await;
    let stranger = create_user_and_session(&state).await;
    let sha256 =
        parse_content_hash("deadbeef99999998000000000000000000000000000000000000000000000000");
    let filename = parse_filename("live-evidence.png");
    let media = MediaRecord {
        user_id: owner.user_id,
        sha256: sha256.clone(),
        filename: filename.clone(),
        source: MediaSource::Upload,
        content_type: parse_content_type("image/png"),
        size_bytes: parse_byte_size("42"),
        source_url: None,
        created_at: UtcInstant::now(),
    };
    create_media(&state, &media).await;
    let media_url = common::media::media_url(&media.source, &sha256, &filename);
    let foreign_form: MediaReferenceForm = format!("https://foreign.example{media_url}")
        .parse()
        .expect("valid media reference form");
    let resolver = Arc::new(ForeignReferenceResolver::new([foreign_form.clone()]));

    let owned = SeedRawPost::new(owner.user_id)
        .body(parse_post_body(&format!(
            "<img src=\"https://owned.example{media_url}\">"
        )))
        .seed(&state)
        .await;
    let (status, body) = post_server_fn_with_media_ownership_resolver(
        &state,
        resolver.clone(),
        &web::media::Delete {
            request: web::media::DeleteMediaRequest {
                sha256: sha256.clone(),
                filename: filename.clone(),
                source: MediaSource::Upload,
                force: None,
            },
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let refused = confirmed_media_deletion(&body);
    assert!(!refused.deleted);
    assert_eq!(refused.referenced_in_posts, vec![owned.post_id]);
    assert_eq!(
        resolver.calls().len(),
        1,
        "one resolution feeds report and guard"
    );

    let _foreign = SeedRawPost::new(owner.user_id)
        .body(parse_post_body(&format!("<img src=\"{foreign_form}\">")))
        .seed(&state)
        .await;
    let _unknown = SeedRawPost::new(stranger.user_id)
        .body(parse_post_body(&format!(
            "<img src=\"https://unknown.example{media_url}\">"
        )))
        .seed(&state)
        .await;
    let (status, body) = post_server_fn_with_media_ownership_resolver(
        &state,
        resolver.clone(),
        &web::media::Delete {
            request: web::media::DeleteMediaRequest {
                sha256,
                filename,
                source: MediaSource::Upload,
                force: Some(true),
            },
        },
        Some(&owner.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let refused = confirmed_media_deletion(&body);
    assert!(!refused.deleted, "unknown foreign ownership fails closed");
    assert_eq!(refused.referenced_in_posts, vec![owned.post_id]);
    let calls = resolver.calls();
    assert_eq!(
        calls.len(),
        2,
        "force resolves once before its storage guard"
    );
    assert_eq!(
        calls[1].len(),
        3,
        "resolver receives global cross-user rows"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_refusal_reports_the_reference_snapshot_despite_a_concurrent_post(
    #[case] backend: Backend,
) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let sha256 =
        parse_content_hash("deadbeef99999997000000000000000000000000000000000000000000000000");
    let filename = parse_filename("snapshot.png");
    let media = MediaRecord {
        user_id: session.user_id,
        sha256: sha256.clone(),
        filename: filename.clone(),
        source: MediaSource::Upload,
        content_type: parse_content_type("image/png"),
        size_bytes: parse_byte_size("42"),
        source_url: None,
        created_at: UtcInstant::now(),
    };
    create_media(&state, &media).await;
    let media_url = common::media::media_url(&media.source, &sha256, &filename);
    let original = SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!("<img src=\"{media_url}\">")))
        .seed(&state)
        .await;
    let resolver = Arc::new(BlockingOwnershipResolver::new());
    let started = resolver.started.notified();
    let request = web::media::Delete {
        request: web::media::DeleteMediaRequest {
            sha256,
            filename,
            source: MediaSource::Upload,
            force: None,
        },
    };
    let deleting = tokio::spawn({
        let state = Arc::clone(&state);
        let resolver = Arc::clone(&resolver);
        let cookie = session.cookie();
        async move {
            post_server_fn_with_media_ownership_resolver(&state, resolver, &request, Some(&cookie))
                .await
        }
    });
    started.await;

    let later = SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!("<img src=\"{media_url}\">")))
        .seed(&state)
        .await;
    resolver.release.notify_one();
    let (status, body) = deleting.await.expect("delete task does not panic");
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let result = confirmed_media_deletion(&body);
    assert!(!result.deleted);
    assert_eq!(
        result.referenced_in_posts,
        vec![original.post_id],
        "the refusal explains the pre-lock reference snapshot, not a later query"
    );
    assert_ne!(original.post_id, later.post_id);
}

#[apply(backends)]
#[tokio::test]
async fn delete_nested_request_force_can_break_owner_retained_history(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let user_id = session.user_id;
    let sha256 =
        parse_content_hash("feedface99999999000000000000000000000000000000000000000000000000");
    let filename = parse_filename("forced.png");
    let media_url = common::media::media_url(&MediaSource::Upload, &sha256, &filename);
    let record = MediaRecord {
        user_id,
        sha256: sha256.clone(),
        filename: filename.clone(),
        source: MediaSource::Upload,
        content_type: parse_content_type("image/png"),
        size_bytes: parse_byte_size("43"),
        source_url: None,
        created_at: UtcInstant::now(),
    };
    create_media(&state, &record).await;
    SeedRawPost::new(user_id)
        .body(parse_post_body(&format!("![forced]({media_url})")))
        .seed(&state)
        .await;

    let (status, body_str) = post_server_fn(
        &state,
        &web::media::Delete {
            request: web::media::DeleteMediaRequest {
                sha256: sha256.clone(),
                filename: filename.clone(),
                source: MediaSource::Upload,
                force: Some(true),
            },
        },
        Some(&session.cookie()),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body_str}");
    let result = confirmed_media_deletion(&body_str);
    assert!(
        result.deleted,
        "explicit force may knowingly break the owner's retained history"
    );
    assert!(result.referenced_in_posts.is_empty());
    assert!(
        state
            .media
            .get_media(user_id, &sha256, &filename, &MediaSource::Upload)
            .await
            .unwrap()
            .is_none(),
        "forced deletion removes the owner's final media identity"
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

    // The server fn returns 200 with a confirmed mutation outcome.
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let resp = confirmed_upload(&body);
    assert_eq!(resp.filename, "photo.jpg");
    assert_eq!(resp.content_type, "image/jpeg");
    assert!(resp.url.contains("/media/upload/"), "url: {}", resp.url);
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_detects_content_type_when_field_omits_it(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();
    let storage = TempDir::new().unwrap();
    let boundary = "----testboundary1234";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.jpg\"\r\n\r\nfake jpeg data\r\n--{boundary}--\r\n"
    );
    let response = make_app(&state, &storage)
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

    assert_eq!(response.status(), StatusCode::OK);
    let body = crate::helpers::body_string(response).await;
    let response = confirmed_upload(&body);
    assert_eq!(response.content_type, "image/jpeg");
}

#[apply(backends)]
#[tokio::test]
async fn upload_then_serve_round_trips_a_filename_needing_encoding(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();
    let storage = TempDir::new().unwrap();

    // A space is a *legal* `Filename` — `sanitize_filename` permits it — so this is an
    // ordinary upload, not a hostile one. The derived URL must carry it encoded:
    // `RootRelativeUrl` cannot even represent a raw space (#675).
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
    let resp = confirmed_upload(&body);

    // The wire field carries the *canonical* encoded spelling (#720), because it is a
    // lookup key rather than a display value — `atompub::media::collection_post` passes it
    // straight to `get_media`. Rendering surfaces decode it; this one does not.
    assert_eq!(resp.filename, "my%20photo.jpg");
    assert_eq!(resp.filename.decoded(), "my photo.jpg");
    assert!(resp.url.contains("my%20photo.jpg"), "url: {}", resp.url);
    assert!(!resp.url.contains(' '), "url: {}", resp.url);

    // The third spelling, read straight off the filesystem rather than inferred from a
    // successful serve (#720). Serving proves the reader and writer agree with each other;
    // only this proves they agree with the *stored column*. Walk to the leaf so the
    // assertion is about the directory entry's real name, not a path we reconstructed.
    let leaf = {
        let mut dir = storage.path().join("media").join("upload");
        // `<p1>/<p2>/<sha256>/` — three machine-generated levels, one entry each here.
        for _ in 0..3 {
            let entry = std::fs::read_dir(&dir)
                .expect("media tree should exist")
                .next()
                .expect("exactly one entry at each hash level")
                .expect("readable dir entry");
            dir = entry.path();
        }
        std::fs::read_dir(&dir)
            .expect("hash directory should exist")
            .next()
            .expect("the stored file")
            .expect("readable dir entry")
            .file_name()
    };
    assert_eq!(leaf.to_string_lossy(), "my%20photo.jpg");
    assert_eq!(
        leaf.to_string_lossy(),
        resp.filename.as_ref(),
        "the on-disk leaf and the stored column must be byte-identical"
    );

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
    // 255-byte per-component limit. It must be rejected before the file write, not fail
    // there with an opaque 500 (#708); the name is otherwise perfectly legal.
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
    let resp = confirmed_upload(&body);

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
    crate::helpers::set_site_config(&state, SiteConfigKey::MediaMaxFileSizeBytes, "5")
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
    crate::helpers::set_site_config(&state, SiteConfigKey::MediaUserQuotaBytes, "5")
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
        storage::InstanceId::new(),
        noop_mailer(),
        true,
        crate::helpers::tmp_storage_path(),
    )
    .expect("canonical instance identity is an HTTP header");
    app.oneshot(request)
        .await
        .expect("router oneshot failed")
        .status()
}

// The strict route extractor rejects malformed hashes as 400 before the handler can slice
// or read from storage.
#[apply(backends_matrix)]
#[case::short_hash("/media/upload/a/a/a/file.txt".to_owned())]
#[case::non_hex(format!("/media/upload/zz/zz/{}/file.txt", "z".repeat(64)))]
#[tokio::test]
async fn serve_handler_rejects_malformed_hash(backend: Backend, #[case] uri: String) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let status = media_serve_get(&state, &uri).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}
