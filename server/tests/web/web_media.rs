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

use common::pagination::{PageOffset, RowLimit};
use common::time::UtcInstant;
use rstest::*;
use rstest_reuse::*;
use storage::{
    CreateMediaError, ForeignEvidenceSink, InstanceId, LocalMediaSink, MediaRecord,
    MediaReferenceEvidence, MediaReferenceOwnershipResolver, PersistedMediaReference,
    ProvenLocalMediaRefs, WriteScopeError,
};

use crate::helpers::image_privacy::{jpeg_oracle_bytes, jpeg_privacy_tags};
use crate::helpers::{
    ForeignReferenceResolver, MultipartFile, create_user_and_session, make_app, post_form,
    post_multipart, post_server_fn, post_server_fn_with_media_ownership_resolver,
};
use common::media::{
    MaxFileSize, MediaReference, MediaReferenceForm, MediaSource, UploadedMedia, UserQuota,
};
use common::test_support::{
    parse_byte_size, parse_content_hash, parse_content_type, parse_filename, parse_post_body,
};
use storage::test_support::{
    Backend, SeedRawPost, backends, backends_matrix, noop_mailer, seed_media,
};

const CLEAN_JPEG: &[u8] = include_bytes!("../fixtures/image-privacy/jpeg-clean.jpg");

async fn create_media(
    media: std::sync::Arc<dyn storage::MediaStorage>,
    write_scope: storage::WriteScope,
    record: &MediaRecord,
) {
    let record = record.clone();
    let outcome = match write_scope
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

    async fn resolve_local(
        &self,
        _: &[MediaReference],
        _: &InstanceId,
        _: Option<&common::tagged_url::BaseUrl>,
        local: LocalMediaSink,
    ) -> ProvenLocalMediaRefs {
        local.finish()
    }
}
// ─── media_usage ──────────────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn media_usage_returns_defaults_for_authenticated_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_form(
        app.clone(),
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

// ─── get_uploads_enabled ──────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn get_uploads_enabled_defaults_to_true_for_authenticated_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_server_fn(
        app.clone(),
        &web::media::GetUploadsEnabled {},
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        serde_json::from_str::<bool>(&body).expect("response should be a boolean"),
        "an absent media-upload setting defaults to enabled"
    );
}

#[apply(backends)]
#[tokio::test]
async fn get_uploads_enabled_reports_an_explicitly_disabled_capability(#[case] backend: Backend) {
    let env = backend.setup().media_uploads_enabled(false).await;
    let app = make_app!(&env, &env.base);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_server_fn(
        app.clone(),
        &web::media::GetUploadsEnabled {},
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert!(
        !serde_json::from_str::<bool>(&body).expect("response should be a boolean"),
        "the authenticated media page receives the disabled upload capability as an advisory read"
    );
}

// Shape B — every media server-fn refuses an unauthenticated request the same
// way (Leptos server fn → INTERNAL_SERVER_ERROR + "unauthorized"). Typed inputs
// keep this gate test independent of hand-encoded transport syntax.
#[apply(backends)]
#[tokio::test]
async fn media_endpoints_reject_unauthenticated_requests(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);

    let (usage_status, usage_body) =
        post_server_fn(app.clone(), &web::media::GetUsage {}, None).await;
    let (uploads_enabled_status, uploads_enabled_body) =
        post_server_fn(app.clone(), &web::media::GetUploadsEnabled {}, None).await;

    let (list_status, list_body) = post_server_fn(
        app.clone(),
        &web::media::ListMine {
            source: None,
            limit: None,
            offset: None,
        },
        None,
    )
    .await;
    let (delete_status, delete_body) = post_server_fn(
        app.clone(),
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
        (
            "get_uploads_enabled",
            uploads_enabled_status,
            uploads_enabled_body,
        ),
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

#[apply(backends)]
#[tokio::test]
async fn media_server_function_auth_rejection_does_not_advertise_basic(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let request_body =
        serde_qs::to_string(&web::media::GetUsage {}).expect("serialize server-function input");

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(<web::media::GetUsage as ServerFn>::PATH)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(request_body))
                .expect("build unauthenticated server-function request"),
        )
        .await
        .expect("server-function request");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(header::WWW_AUTHENTICATE).is_none());
}

// ─── list_my_media ────────────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn list_my_media_returns_empty_for_new_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    let (status, body) = post_form(
        app.clone(),
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
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    // `limit=999` is outside PageSize's `1..=50`; the typed wire arg rejects it on
    // deserialization instead of fetching an unbounded page.
    let (status, _body) = post_form(
        app.clone(),
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
    let env = backend.setup().media_uploads_enabled(false).await;
    let app = make_app!(&env, &env.base);
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    seed_media(
        std::sync::Arc::clone(&env.media()),
        env.write_scope(),
        session.user_id,
        "photo.jpg",
    )
    .await;

    let cookie = session.cookie();

    let (status, body) = post_form(
        app.clone(),
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
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    seed_media(
        std::sync::Arc::clone(&env.media()),
        env.write_scope(),
        session.user_id,
        "clip.mp4",
    )
    .await;

    let cookie = session.cookie();

    let (status, body) = post_form(
        app.clone(),
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
    let env = backend.setup().media_uploads_enabled(false).await;
    let app = make_app!(&env, &env.base);
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let media = seed_media(
        std::sync::Arc::clone(&env.media()),
        env.write_scope(),
        session.user_id,
        "test.png",
    )
    .await;

    let cookie = session.cookie();

    let (status, body_str) = post_server_fn(
        app.clone(),
        &web::media::Delete {
            request: web::media::DeleteMediaRequest {
                sha256: media.sha256.clone(),
                filename: media.filename.clone(),
                source: media.source,
                force: None,
            },
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body_str}");
    assert_eq!(
        confirmed_media_deletion(&body_str),
        MediaDeletion::Deleted,
        "delete of existing item should report its confirmed deleted state"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_nested_request_refuses_referenced_without_force(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let user_id = session.user_id;

    let media = seed_media(
        std::sync::Arc::clone(&env.media()),
        env.write_scope(),
        user_id,
        "inline.png",
    )
    .await;
    let media_url = common::media::url(&media.source, &media.sha256, &media.filename);

    let post = SeedRawPost::new(user_id)
        .body(parse_post_body(&format!("![inline]({media_url})")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    let cookie = session.cookie();

    let (status, body_str) = post_server_fn(
        app.clone(),
        &web::media::Delete {
            request: web::media::DeleteMediaRequest {
                sha256: media.sha256.clone(),
                filename: media.filename.clone(),
                source: media.source,
                force: None,
            },
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "body: {body_str}");
    assert_eq!(
        confirmed_media_deletion(&body_str),
        MediaDeletion::OwnerRetainedHistory {
            post_ids: vec![post.post_id],
            theme_reference_count: 0,
        },
        "delete without force should report the referencing post"
    );
}
#[apply(backends)]
#[tokio::test]
async fn delete_uses_one_global_live_ownership_snapshot(#[case] backend: Backend) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
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
    create_media(env.media(), env.write_scope(), &media).await;
    let media_url = common::media::url(&media.source, &sha256, &filename);
    let foreign_form: MediaReferenceForm = format!("https://foreign.example{media_url}")
        .parse()
        .expect("valid media reference form");
    let resolver = Arc::new(ForeignReferenceResolver::new([foreign_form.clone()]));
    let app = make_app!(
        &env, &env.base;
        instance_id = InstanceId::new(),
        mailer = noop_mailer(),
        secure_cookies = false,
        resolver = resolver.clone()
    );
    // The router owns this resolver for the full request lifecycle.

    let owned = SeedRawPost::new(owner.user_id)
        .body(parse_post_body(&format!(
            "<img src=\"https://owned.example{media_url}\">"
        )))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;
    let (status, body) = post_server_fn_with_media_ownership_resolver(
        app.clone(),
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
    assert_eq!(
        confirmed_media_deletion(&body),
        MediaDeletion::OwnerRetainedHistory {
            post_ids: vec![owned.post_id],
            theme_reference_count: 0,
        }
    );
    assert_eq!(
        resolver.calls().len(),
        1,
        "one resolution feeds report and guard"
    );

    let _foreign = SeedRawPost::new(owner.user_id)
        .body(parse_post_body(&format!("<img src=\"{foreign_form}\">")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;
    let _unknown = SeedRawPost::new(stranger.user_id)
        .body(parse_post_body(&format!(
            "<img src=\"https://unknown.example{media_url}\">"
        )))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;
    let (status, body) = post_server_fn_with_media_ownership_resolver(
        app.clone(),
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
    assert_eq!(
        confirmed_media_deletion(&body),
        MediaDeletion::GlobalSafety {
            theme_reference_count: 0,
        },
        "unknown foreign ownership fails closed"
    );
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
async fn delete_refusal_reports_locked_classification_including_concurrent_post(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let resolver = Arc::new(BlockingOwnershipResolver::new());
    let app = make_app!(
        &env, &env.base;
        instance_id = InstanceId::new(),
        mailer = noop_mailer(),
        secure_cookies = false,
        resolver = resolver.clone()
    );
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
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
    create_media(env.media(), env.write_scope(), &media).await;
    let media_url = common::media::url(&media.source, &sha256, &filename);
    let original = SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!("<img src=\"{media_url}\">")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;
    // The router owns this resolver for the full request lifecycle.
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
        let app = app.clone();
        let cookie = session.cookie();
        async move {
            post_server_fn_with_media_ownership_resolver(app.clone(), &request, Some(&cookie)).await
        }
    });
    started.await;

    let later = SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!("<img src=\"{media_url}\">")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;
    resolver.release.notify_one();
    let (status, body) = deleting.await.expect("delete task does not panic");
    let mut expected = vec![original.post_id, later.post_id];
    expected.sort_unstable_by_key(|post_id| i64::from(*post_id));
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        confirmed_media_deletion(&body),
        MediaDeletion::OwnerRetainedHistory {
            post_ids: expected,
            theme_reference_count: 0,
        },
        "classification under the delete lock includes the concurrent retained Post"
    );
    assert_ne!(original.post_id, later.post_id);
}

#[apply(backends)]
#[tokio::test]
async fn delete_nested_request_force_can_break_owner_retained_history(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let user_id = session.user_id;
    let sha256 =
        parse_content_hash("feedface99999999000000000000000000000000000000000000000000000000");
    let filename = parse_filename("forced.png");
    let media_url = common::media::url(&MediaSource::Upload, &sha256, &filename);
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
    create_media(env.media(), env.write_scope(), &record).await;
    SeedRawPost::new(user_id)
        .body(parse_post_body(&format!("![forced]({media_url})")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    let (status, body_str) = post_server_fn(
        app.clone(),
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
    assert_eq!(
        confirmed_media_deletion(&body_str),
        MediaDeletion::Deleted,
        "explicit force may knowingly break the owner's retained history"
    );
    assert!(
        env.media()
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
async fn upload_then_serve_removes_ordinary_jpeg_metadata(#[case] backend: Backend) {
    let original = include_bytes!("../fixtures/image-privacy/jpeg-original.jpg");
    let before = jpeg_privacy_tags(original);
    let private_tags = [
        "IFD0:Make",
        "IFD0:Model",
        "IFD0:Artist",
        "IFD0:ImageDescription",
        "ExifIFD:DateTimeOriginal",
        "GPS:GPSLatitude",
        "GPS:GPSLongitude",
        "ExifIFD:UserComment",
        "IPTC:Caption-Abstract",
        "XMP-dc:Description",
        "IFD1:ThumbnailOffset",
    ];
    for tag in private_tags {
        assert!(before.contains_key(tag), "BEFORE witness absent: {tag}");
    }
    let env = backend.setup().await;
    let storage = TempDir::new().expect("media root");
    let app = make_app!(&env, &storage);
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "photo.jpg",
            content_type: "image/jpeg",
            bytes: original,
        },
        Some(&session.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload failed: {body}");
    let uploaded = confirmed_upload(&body);
    let response = app
        .oneshot(
            Request::builder()
                .uri(uploaded.url.as_ref())
                .body(Body::empty())
                .expect("public GET"),
        )
        .await
        .expect("public response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/jpeg"
    );
    assert_eq!(
        response
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap(),
        host::etag::from_content_hash(&uploaded.sha256).as_ref()
    );
    let served = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("public bytes");
    {
        use sha2::Digest as _;
        let actual = common::media::ContentHash::from_digest(sha2::Sha256::digest(&served).into());
        assert_eq!(
            uploaded.sha256, actual,
            "stored identity describes public bytes"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = storage
            .path()
            .join(uploaded.url.as_ref().trim_start_matches('/'));
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
    let after = jpeg_privacy_tags(&served);
    for tag in private_tags {
        assert!(
            !after.contains_key(tag),
            "public JPEG retains private tag: {tag}"
        );
    }
    let profile = jpeg_oracle_bytes(original, &["-b", "-ICC_Profile"]);
    assert!(!profile.is_empty(), "BEFORE needs a real ICC witness");
    assert!(
        profile == jpeg_oracle_bytes(&served, &["-b", "-ICC_Profile"]),
        "ICC bytes changed"
    );
    // The independent editor removes metadata from copies only; equal normalized
    // JPEGs witness identical encoded content without a new JPEG grammar in tests.
    assert!(
        jpeg_oracle_bytes(original, &["-all=", "-o", "-"])
            == jpeg_oracle_bytes(&served, &["-all=", "-o", "-"]),
        "encoded JPEG content changed"
    );
    let app = make_app!(&env, &storage);
    for bytes in [original.as_slice(), served.as_ref()] {
        let (status, body) = post_multipart(
            app.clone(),
            <web::media::Upload as ServerFn>::PATH,
            MultipartFile {
                filename: "photo.jpg",
                content_type: "image/jpeg",
                bytes,
            },
            Some(&session.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "repeat upload failed: {body}");
        let repeated = confirmed_upload(&body);
        assert_eq!(
            repeated.sha256, uploaded.sha256,
            "same input and clean reupload keep identity"
        );
        assert_eq!(repeated.size_bytes, uploaded.size_bytes);
    }
    assert_eq!(
        uploaded.size_bytes.value(),
        i64::try_from(served.len()).expect("served image size fits i64")
    );
    for content_type in ["image/jpeg", "IMAGE/JPEG", "text/plain"] {
        let (status, body) = post_multipart(
            app.clone(),
            <web::media::Upload as ServerFn>::PATH,
            MultipartFile {
                filename: "invalid.jpg",
                content_type,
                bytes: b"not an image",
            },
            Some(&session.cookie()),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::OK,
            "invalid image-labelled raster must not publish: {body}"
        );
    }
    let (status, body) =
        post_server_fn(app, &web::media::GetUsage {}, Some(&session.cookie())).await;
    assert_eq!(status, StatusCode::OK);
    let usage: UsageData = serde_json::from_str(&body).expect("usage");
    assert_eq!(
        usage.used_bytes, uploaded.size_bytes,
        "rejection and dedup charge no extra quota"
    );
    assert!(
        std::fs::read_dir(storage.path().join("media/tmp"))
            .expect("private spools")
            .next()
            .is_none(),
        "completed and rejected attempts leave no private files"
    );
}

// Each case has independently observed positive BEFORE witnesses. The oracle's
// metadata-only rewrite compares encoded content, not a sanitized golden or a
// list of descriptive tags; rendering controls are checked separately.
#[apply(backends_matrix)]
#[case::png(
    include_bytes!("../fixtures/image-privacy/png-original.png").as_slice(),
    "image/png", true,
    &["IFD0:Make", "IFD0:Model", "IFD0:Artist", "IFD0:ImageDescription", "IFD0:ModifyDate", "GPS:GPSLatitude", "GPS:GPSLongitude", "PNG:Author", "PNG:Comment", "PNG:XMLcomadobexmp", "PNG:ModifyDate"],
    &[("PNG:ImageWidth", serde_json::json!(32)), ("PNG:ImageHeight", serde_json::json!(24)), ("IFD0:Orientation", serde_json::json!(6)), ("PNG:ColorType", serde_json::json!(6))]
)]
#[case::png_color(
    include_bytes!("../fixtures/image-privacy/png-color-original.png").as_slice(),
    "image/png", false, &["PNG:Author", "PNG:Comment", "PNG:XMLcomadobexmp", "PNG:ModifyDate", "GPS:GPSLatitude"],
    &[("PNG:Gamma", serde_json::json!(2.2)), ("PNG:SRGBRendering", serde_json::json!(0)), ("IFD0:Orientation", serde_json::json!(6))]
)]
#[case::apng_default(
    include_bytes!("../fixtures/image-privacy/apng-default-original.png").as_slice(),
    "image/png", true, &["PNG:Author", "PNG:Comment", "PNG:XMLcomadobexmp", "PNG:ModifyDate", "GPS:GPSLatitude"],
    &[("PNG:AnimationFrames", serde_json::json!(3)), ("PNG:AnimationPlays", serde_json::json!(2)), ("IFD0:Orientation", serde_json::json!(6)), ("PNG:ColorType", serde_json::json!(6))]
)]
#[case::apng_first(
    include_bytes!("../fixtures/image-privacy/apng-first-original.png").as_slice(),
    "image/png", true, &["PNG:Author", "PNG:Comment", "PNG:XMLcomadobexmp", "PNG:ModifyDate", "GPS:GPSLatitude"],
    &[("PNG:AnimationFrames", serde_json::json!(3)), ("PNG:AnimationPlays", serde_json::json!(0)), ("IFD0:Orientation", serde_json::json!(6)), ("PNG:ColorType", serde_json::json!(6))]
)]
#[case::gif_animation(
    include_bytes!("../fixtures/image-privacy/gif-animation-original.gif").as_slice(),
    "image/gif", true, &["File:Comment"],
    &[("GIF:ImageWidth", serde_json::json!(12)), ("GIF:ImageHeight", serde_json::json!(10)), ("GIF:TransparentColor", serde_json::json!(0)), ("GIF:FrameCount", serde_json::json!(3)), ("GIF:AnimationIterations", serde_json::json!(2)), ("GIF:Duration", serde_json::json!(0.35))]
)]
#[case::webp_animation(
    include_bytes!("../fixtures/image-privacy/webp-animation-original.webp").as_slice(),
    "image/webp", true, &["IFD0:Make", "IFD0:Model", "IFD0:Artist", "IFD0:ImageDescription", "IFD0:ModifyDate", "GPS:GPSLatitude", "GPS:GPSLongitude", "XMP-dc:Description"],
    &[("RIFF:ImageWidth", serde_json::json!(32)), ("RIFF:ImageHeight", serde_json::json!(24)), ("IFD0:Orientation", serde_json::json!(6)), ("RIFF:AnimationLoopCount", serde_json::json!(2)), ("RIFF:Duration", serde_json::json!(0.35)), ("RIFF:BackgroundColor", serde_json::json!("29 19 9 39"))]
)]
#[case::heic(
    include_bytes!("../fixtures/image-privacy/heic-original.heic").as_slice(),
    "image/heif", false, &["IFD0:Make", "IFD0:Model", "IFD0:Artist", "IFD0:ImageDescription", "ExifIFD:DateTimeOriginal", "GPS:GPSLatitude", "GPS:GPSLongitude", "XMP-dc:Description", "XMP-exif:GPSLatitude", "XMP-exif:GPSLongitude"],
    &[("File:FileType", serde_json::json!("HEIC")), ("File:ImageWidth", serde_json::json!(64)), ("File:ImageHeight", serde_json::json!(64)), ("QuickTime:CleanAperture", serde_json::json!("32 24 -16 -20")), ("QuickTime:ColorPrimaries", serde_json::json!(1)), ("QuickTime:TransferCharacteristics", serde_json::json!(13)), ("QuickTime:MatrixCoefficients", serde_json::json!(6)), ("QuickTime:VideoFullRangeFlag", serde_json::json!(1))]
)]
#[case::hdr_signal(
    include_bytes!("../fixtures/image-privacy/hdr-signal.png").as_slice(),
    "image/png", false, &["PNG:Comment"],
    &[("PNG:ImageWidth", serde_json::json!(2)), ("PNG:ImageHeight", serde_json::json!(2)), ("PNG-cICP:ColorPrimaries", serde_json::json!(9)), ("PNG-cICP:TransferCharacteristics", serde_json::json!(16)), ("PNG-cICP:MatrixCoefficients", serde_json::json!(0)), ("PNG-cICP:VideoFullRangeFlag", serde_json::json!(1))]
)]
#[tokio::test]
async fn upload_then_serve_preserves_image_fidelity(
    backend: Backend,
    #[case] original: &[u8],
    #[case] mime: &str,
    #[case] has_profile: bool,
    #[case] private: &[&str],
    #[case] rendering: &[(&str, serde_json::Value)],
) {
    use crate::helpers::image_privacy::{oracle_tags, rgba_frames};
    use sha2::{Digest, Sha256};
    let before = oracle_tags(original, &[]);
    for tag in private {
        assert!(
            before.contains_key(*tag),
            "BEFORE privacy witness absent: {tag}"
        );
    }
    for (tag, expected) in rendering {
        assert_eq!(
            before.get(*tag),
            Some(expected),
            "BEFORE rendering witness: {tag}"
        );
    }
    let profile = jpeg_oracle_bytes(original, &["-b", "-ICC_Profile"]);
    assert_eq!(!profile.is_empty(), has_profile, "BEFORE ICC witness");
    // HEIC container offsets and metadata extents can differ after two valid
    // metadata rewrites. Ask the independent oracle for its actual image-data
    // digest there, rather than treating container layout as encoded content.
    let encoded_arguments: &[&str] = if mime == "image/heif" {
        &["-b", "-api", "ImageHashType=SHA256", "-ImageDataHash"]
    } else {
        &["-all=", "-o", "-"]
    };
    let encoded = jpeg_oracle_bytes(original, encoded_arguments);
    assert!(
        !encoded.is_empty() && encoded.len() < original.len(),
        "independent encoded-content witness absent"
    );
    if mime == "image/heif" {
        assert_eq!(
            encoded.as_slice(),
            b"7e182f80de852f3219b82edb65159e3aab4388b184b7c6c3b43bf74ebd20e525",
            "BEFORE independent SHA256 image-data witness"
        );
        // This one owned fixture's oracle identifies hvc1 at 0xfcf, length 63.
        // Guard its identity and a byte inside that payload before corrupting a
        // disposable copy: a constant/empty hash must not pass preservation.
        assert_eq!(
            common::media::ContentHash::from_digest(Sha256::digest(original).into()).as_ref(),
            "32ef665e9e89f18e91985c79d8a1984b5375eec33e82be20137637ebb109c994"
        );
        assert_eq!(original[0xfd7], 0x80);
        let mut changed = original.to_vec();
        changed[0xfd7] ^= 1;
        assert!(
            encoded != jpeg_oracle_bytes(&changed, encoded_arguments),
            "image-data observer missed a payload change"
        );
    }
    let pixels = rgba_frames(original, mime);
    if let Some(frames) = &pixels {
        assert!(!frames.is_empty(), "BEFORE decoded frames absent");
        if has_profile {
            assert!(
                frames
                    .iter()
                    .flat_map(|f| f.buffer().pixels())
                    .any(|p| p.0[3] < 255),
                "BEFORE needs actual transparent samples"
            );
            assert!(
                frames
                    .iter()
                    .flat_map(|f| f.buffer().pixels())
                    .any(|p| p.0[3] > 0),
                "BEFORE must not be entirely transparent"
            );
        }
        if mime == "image/webp" {
            assert_eq!(frames.len(), 3, "BEFORE animated frames");
        }
    }
    if before.contains_key("PNG:AnimationFrames") {
        assert_eq!(pixels.as_ref().expect("APNG frames").len(), 3);
    }
    let env = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let filename = "misleading.jpg";
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename,
            content_type: "application/octet-stream",
            bytes: original,
        },
        Some(&session.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "upload failed: {body}");
    let uploaded = confirmed_upload(&body);
    assert_eq!(uploaded.filename.as_ref(), filename);
    assert_eq!(uploaded.content_type.as_ref(), mime);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uploaded.url.as_ref())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get(header::CONTENT_TYPE).unwrap(), mime);
    assert_eq!(
        response.headers().get(header::ETAG).unwrap(),
        host::etag::from_content_hash(&uploaded.sha256).as_ref()
    );
    let served = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let hash = common::media::ContentHash::from_digest(Sha256::digest(&served).into());
    assert_eq!(uploaded.sha256, hash);
    assert_eq!(
        uploaded.size_bytes.value(),
        i64::try_from(served.len()).expect("served image size fits i64")
    );
    if let Some(before_frames) = pixels {
        let after_frames = rgba_frames(&served, mime).expect("same public format decoder");
        assert_eq!(
            before_frames.len(),
            after_frames.len(),
            "decoded frame count changed"
        );
        for (before, after) in before_frames.iter().zip(&after_frames) {
            assert_eq!(
                before.buffer().dimensions(),
                after.buffer().dimensions(),
                "decoded dimensions changed"
            );
            assert!(before.buffer() == after.buffer(), "decoded RGBA changed");
            assert_eq!(before.delay(), after.delay(), "frame timing changed");
            assert_eq!(
                (before.left(), before.top()),
                (after.left(), after.top()),
                "frame placement changed"
            );
        }
    }
    let after = oracle_tags(&served, &[]);
    for tag in private {
        assert!(
            !after.contains_key(*tag),
            "public bytes retain ordinary metadata: {tag}"
        );
    }
    for (tag, expected) in rendering {
        assert_eq!(
            after.get(*tag),
            Some(expected),
            "public rendering control changed: {tag}"
        );
    }
    assert!(
        profile == jpeg_oracle_bytes(&served, &["-b", "-ICC_Profile"]),
        "ICC bytes changed"
    );
    assert!(
        encoded == jpeg_oracle_bytes(&served, encoded_arguments),
        "encoded image/frame content changed"
    );
    let record = env
        .media()
        .get_media(
            session.user_id,
            &hash,
            &uploaded.filename,
            &MediaSource::Upload,
        )
        .await
        .unwrap()
        .expect("sanitized Media Record");
    assert_eq!(record.size_bytes, uploaded.size_bytes);
    assert_eq!(record.content_type.as_ref(), mime);
    for bytes in [original, served.as_ref()] {
        let (status, body) = post_multipart(
            app.clone(),
            <web::media::Upload as ServerFn>::PATH,
            MultipartFile {
                filename,
                content_type: "image/jpeg",
                bytes,
            },
            Some(&session.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "reupload failed: {body}");
        let repeated = confirmed_upload(&body);
        assert_eq!(repeated.sha256, hash);
        assert_eq!(repeated.url, uploaded.url);
        assert_eq!(repeated.size_bytes, uploaded.size_bytes);
    }
    assert_eq!(
        env.media()
            .get_user_upload_usage(session.user_id)
            .await
            .unwrap(),
        uploaded.size_bytes
    );
    let original_hash = common::media::ContentHash::from_digest(Sha256::digest(original).into());
    assert_ne!(
        original_hash, hash,
        "positive privacy witness must change identity"
    );
    let old_url = common::media::url(&MediaSource::Upload, &original_hash, &uploaded.filename);
    assert_eq!(
        app.oneshot(
            Request::builder()
                .uri(old_url.as_ref())
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap()
        .status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        std::fs::read_dir(storage.path().join("media/tmp"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[apply(backends)]
#[tokio::test]
async fn upload_non_image_keeps_bytes_and_supplied_type(#[case] backend: Backend) {
    let env = backend.setup().await;
    let storage = TempDir::new().expect("media root");
    let app = make_app!(&env, &storage);
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    // A detected non-image stays outside the raster policy even under a misleading
    // image label. This is not document validation or a new attachment allowlist.
    let bytes = b"%PDF-1.7\nsynthetic non-personal document\n";
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "document.pdf",
            content_type: "image/jpeg",
            bytes,
        },
        Some(&session.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "non-image upload failed: {body}");
    let uploaded = confirmed_upload(&body);
    assert_eq!(uploaded.content_type, "image/jpeg");
    let response = app
        .oneshot(
            Request::builder()
                .uri(uploaded.url.as_ref())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/jpeg"
    );
    let served = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&served[..], bytes);
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_stores_file_and_returns_metadata(#[case] backend: Backend) {
    let env = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    // A real writable root so the upload lands on disk (separate from the DB backend).
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "photo.jpg",
            content_type: "image/jpeg",
            bytes: CLEAN_JPEG,
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
    let env = backend.setup().await;
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    let storage = TempDir::new().unwrap();
    let boundary = "----testboundary1234";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.jpg\"\r\n\r\n"
    ).into_bytes();
    body.extend_from_slice(CLEAN_JPEG);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let response = make_app!(&env, &storage)
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
    let env = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    // A space is a *legal* `Filename` — `sanitize_filename` permits it — so this is an
    // ordinary upload, not a hostile one. The derived URL must carry it encoded:
    // `RootRelativeUrl` cannot even represent a raw space (#675).
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "my photo.jpg",
            content_type: "image/jpeg",
            bytes: CLEAN_JPEG,
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
    let app = make_app!(&env, &storage);
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
    assert_eq!(&served[..], CLEAN_JPEG);
}

#[apply(backends)]
#[tokio::test]
async fn upload_then_serve_survives_a_name_too_long_to_store(#[case] backend: Backend) {
    let env = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    // 200 `ä` is 400 raw bytes and ~1200 once percent-encoded — far past the filesystem's
    // 255-byte per-component limit. It must be rejected before the file write, not fail
    let long_name = format!("{}.jpg", "ä".repeat(200));
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: &long_name,
            content_type: "image/jpeg",
            bytes: CLEAN_JPEG,
        },
        Some(&cookie),
    )
    .await;
    let resp = confirmed_upload(&body);
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // Truncated, not rejected — and the extension survived, so the detected content type is
    // still an image rather than octet-stream.
    assert!(resp.filename.len() < long_name.len(), "must truncate");
    assert!(resp.filename.ends_with(".jpg"), "{}", resp.filename);
    assert_eq!(resp.content_type, "image/jpeg");

    // The point of the test: the file actually landed and is served back at the URL handed
    // to the client.
    let app = make_app!(&env, &storage);
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
    assert_eq!(&served[..], CLEAN_JPEG);
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_rejects_unauthenticated_request(#[case] backend: Backend) {
    let env = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let (status, body) = post_multipart(
        app.clone(),
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
async fn disabled_upload_is_forbidden_without_media_mutation(#[case] backend: Backend) {
    let env = backend.setup().media_uploads_enabled(false).await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let cookie = session.cookie();
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "blocked.png",
            content_type: "image/png",
            bytes: b"blocked image",
        },
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "body: {body}");
    assert_eq!(
        body,
        r#"{"forbidden":{"message":"media uploads are disabled"}}"#
    );
    assert!(
        std::fs::read_dir(storage.path().join("media").join("tmp"))
            .unwrap()
            .next()
            .is_none(),
        "disabled upload must not create temporary media"
    );
    assert!(
        std::fs::read_dir(storage.path().join("media").join("upload"))
            .unwrap()
            .next()
            .is_none(),
        "disabled upload must not create durable media"
    );
    assert!(
        env.media()
            .list_media(
                session.user_id,
                None,
                RowLimit::at_most(100),
                PageOffset::default(),
            )
            .await
            .unwrap()
            .is_empty(),
        "disabled upload must not create a media row"
    );
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_rejects_invalid_filename(#[case] backend: Backend) {
    let env = backend.setup().await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    // `..` sanitizes to empty → `MediaError::BadRequest`, exercising `map_media_error`'s
    // BadRequest arm (projected to `WebError::Validation`).
    let (status, body) = post_multipart(
        app.clone(),
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
    // A valid image exceeds a positive received-byte cap by exactly one byte;
    // the typed result must be size policy, not an unrelated image failure.
    let env = backend
        .setup()
        .media_limits(
            (CLEAN_JPEG.len() - 1).to_string().parse().unwrap(),
            UserQuota::default(),
        )
        .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let (status, body) = post_multipart(
        app,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "big.jpg",
            content_type: "image/jpeg",
            bytes: CLEAN_JPEG,
        },
        Some(&session.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"validation":{"message":"payload too large"}})
    );
    assert_eq!(
        env.media()
            .get_user_upload_usage(session.user_id)
            .await
            .unwrap()
            .value(),
        0
    );
    assert!(
        env.media()
            .list_media(
                session.user_id,
                None,
                RowLimit::at_most(10),
                PageOffset::default()
            )
            .await
            .unwrap()
            .is_empty()
    );
    for directory in ["media/upload", "media/tmp"] {
        assert!(
            std::fs::read_dir(storage.path().join(directory))
                .unwrap()
                .next()
                .is_none()
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_rejects_over_quota_file(#[case] backend: Backend) {
    // A positive quota smaller than a valid already-clean image must reach the
    // actual stored-byte accounting boundary, not fail native image validation.
    let env = backend
        .setup()
        .media_limits(MaxFileSize::default(), "5".parse().unwrap())
        .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let (status, body) = post_multipart(
        app,
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "big.jpg",
            content_type: "image/jpeg",
            bytes: CLEAN_JPEG,
        },
        Some(&session.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"validation":{"message":"insufficient storage"}})
    );
    assert_eq!(
        env.media()
            .get_user_upload_usage(session.user_id)
            .await
            .unwrap()
            .value(),
        0
    );
    assert!(
        env.media()
            .list_media(
                session.user_id,
                None,
                RowLimit::at_most(10),
                PageOffset::default()
            )
            .await
            .unwrap()
            .is_empty()
    );
    for directory in ["media/upload", "media/tmp"] {
        assert!(
            std::fs::read_dir(storage.path().join(directory))
                .unwrap()
                .next()
                .is_none()
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_rejects_missing_file_field(#[case] backend: Backend) {
    let env = backend.setup().await;
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();

    // An empty multipart body (a closing boundary with no field) yields
    // `next_field() == None`, exercising the "no file field" guard.
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
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

async fn media_serve_get(app: axum::Router, uri: &str) -> StatusCode {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("failed to build request");

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
    let env = backend.setup().await;
    let storage = TempDir::new().expect("test storage directory");
    let app = make_app!(
        &env,
        &storage;
        instance_id = storage::InstanceId::new(),
        mailer = noop_mailer(),
        secure_cookies = true,
        resolver = std::sync::Arc::new(
            jaunder::media_ownership::LiveMediaReferenceOwnershipResolver::new(),
        )
    );

    let status = media_serve_get(app, &uri).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}
