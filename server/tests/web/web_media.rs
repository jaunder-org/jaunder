use std::sync::Arc;

const JPEG: &[u8] = include_bytes!("../../../host/src/image_sanitizer_fixtures/jpeg-sanitized.jpg");

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

// Explicit pre-policy fixture placement is restricted to this disposable rehearsal.
#[apply(backends)]
#[tokio::test]
async fn historical_image_replacement_rehearsal(#[case] backend: Backend) {
    use crate::helpers::{atompub_get, atompub_post_xml, atompub_put_xml, body_string};
    use common::ids::PostId;
    use common::media::ContentHash;
    use sha2::{Digest, Sha256};

    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let owner = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let other = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let cases: [(&str, &str, &[u8], &[u8]); 2] = [
        (
            "historical-private.png",
            "image/png",
            include_bytes!("../../../host/src/image_sanitizer_fixtures/png-original.png"),
            include_bytes!("../../../host/src/image_sanitizer_fixtures/png-sanitized.png"),
        ),
        (
            "historical-private.jpg",
            "image/jpeg",
            include_bytes!("../../../host/src/image_sanitizer_fixtures/jpeg-original.jpg"),
            JPEG,
        ),
    ];
    for (index, (name, mime, original, golden)) in cases.into_iter().enumerate() {
        let old_hash = ContentHash::from_digest(Sha256::digest(original).into());
        let new_hash = ContentHash::from_digest(Sha256::digest(golden).into());
        assert_ne!(old_hash, new_hash);
        let filename = parse_filename(name);
        let old_url = common::media::url(&MediaSource::Upload, &old_hash, &filename);
        let old_path = env.base.path().join("media").join(common::media::path(
            &MediaSource::Upload,
            &old_hash,
            &filename,
        ));
        std::fs::create_dir_all(old_path.parent().unwrap()).unwrap();
        std::fs::write(&old_path, original).unwrap();
        create_media(
            env.media(),
            env.write_scope(),
            &MediaRecord {
                user_id: owner.user_id,
                sha256: old_hash.clone(),
                filename: filename.clone(),
                source: MediaSource::Upload,
                content_type: parse_content_type(mime),
                size_bytes: parse_byte_size(&original.len().to_string()),
                source_url: None,
                created_at: UtcInstant::now(),
            },
        )
        .await;
        let entry = |url: &str| {
            format!(
                r#"<entry xmlns="http://www.w3.org/2005/Atom"><title>Replacement rehearsal</title><content type="html">&lt;img src=&quot;{url}&quot;&gt;</content></entry>"#
            )
        };
        let created = app
            .clone()
            .oneshot(atompub_post_xml(&owner, "posts", &entry(old_url.as_ref())))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let post_id = PostId::from(
            created.headers()[header::LOCATION]
                .to_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .parse::<i64>()
                .unwrap(),
        );
        let suffix = format!("posts/{post_id}");
        let member = app
            .clone()
            .oneshot(atompub_get(&owner, &suffix))
            .await
            .unwrap();
        assert_eq!(member.status(), StatusCode::OK);
        let etag = member.headers()[header::ETAG].clone();
        // Supported ingress, never an in-place rewrite of historical public bytes.
        let (status, body) = post_multipart(
            app.clone(),
            <web::media::Upload as ServerFn>::PATH,
            MultipartFile {
                filename: name,
                content_type: mime,
                bytes: original,
            },
            Some(&owner.cookie()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let uploaded = confirmed_upload(&body);
        assert!(uploaded.url.contains(new_hash.as_ref()));
        assert_eq!(uploaded.filename, name);
        assert_eq!(uploaded.content_type, mime);
        let new_path = env.base.path().join("media").join(common::media::path(
            &MediaSource::Upload,
            &new_hash,
            &filename,
        ));
        assert_eq!(std::fs::read(&new_path).unwrap(), golden);
        assert_eq!(std::fs::read(&old_path).unwrap(), original);
        let mut update = atompub_put_xml(&owner, &suffix, &entry(&uploaded.url));
        update.headers_mut().insert(header::IF_MATCH, etag);
        let updated = app.clone().oneshot(update).await.unwrap();
        assert_eq!(
            updated.status(),
            StatusCode::OK,
            "{}",
            body_string(updated).await
        );
        let current = env.current_post_media(post_id).await;
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].0.sha256, new_hash);
        // Current references changed, but ordinary deletion still protects revisions.
        let request = |force| web::media::Delete {
            request: web::media::DeleteMediaRequest {
                sha256: old_hash.clone(),
                filename: filename.clone(),
                source: MediaSource::Upload,
                force,
            },
        };
        let (status, body) =
            post_server_fn(app.clone(), &request(None), Some(&owner.cookie())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            confirmed_media_deletion(&body),
            MediaDeletion::OwnerRetainedHistory {
                post_ids: vec![post_id],
                theme_reference_count: 0,
            }
        );
        if index == 0 {
            // A supported cross-user Post create materializes an independent
            // record. It retains public bytes, not control over the source row.
            let shared = app
                .clone()
                .oneshot(atompub_post_xml(&other, "posts", &entry(old_url.as_ref())))
                .await
                .unwrap();
            assert_eq!(shared.status(), StatusCode::CREATED);
            let shared_post_id = PostId::from(
                shared.headers()[header::LOCATION]
                    .to_str()
                    .unwrap()
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap(),
            );
            let source_record = env
                .media()
                .get_media(owner.user_id, &old_hash, &filename, &MediaSource::Upload)
                .await
                .unwrap()
                .unwrap();
            let independent = env
                .media()
                .get_media(other.user_id, &old_hash, &filename, &MediaSource::Upload)
                .await
                .unwrap()
                .expect("supported sharing must materialize its author's record");
            assert_eq!(independent.content_type, source_record.content_type);
            assert_eq!(independent.size_bytes, source_record.size_bytes);
            assert_eq!(independent.created_at, source_record.created_at);
            assert_eq!(independent.source_url, source_record.source_url);
            let (status, body) =
                post_server_fn(app.clone(), &request(None), Some(&other.cookie())).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(
                confirmed_media_deletion(&body),
                MediaDeletion::OwnerRetainedHistory {
                    post_ids: vec![shared_post_id],
                    theme_reference_count: 0,
                }
            );
        }
        if index == 1 {
            // Legacy retained sharing without a materialized independent record:
            // no owner's history override can make global reclaim safe.
            SeedRawPost::new(other.user_id)
                .body(parse_post_body(&format!("<img src=\"{old_url}\">")))
                .seed(env.posts(), env.write_scope())
                .await;
        }
        let (status, body) =
            post_server_fn(app.clone(), &request(Some(true)), Some(&owner.cookie())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let expected = if index == 0 {
            MediaDeletion::Deleted
        } else {
            MediaDeletion::GlobalSafety {
                theme_reference_count: 0,
            }
        };
        assert_eq!(confirmed_media_deletion(&body), expected);
        let served = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(old_url.as_ref())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(std::fs::read(&old_path).unwrap(), original);
        // Record removal is not a public-byte erasure promise: the storage
        // reclaim guard still protects retained history, even for the owner.
        let old_member = app
            .clone()
            .oneshot(atompub_get(&owner, &format!("media/{old_hash}/{filename}")))
            .await
            .unwrap();
        assert_eq!(
            old_member.status(),
            if index == 0 {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            }
        );
        if index == 0 {
            let shared_member = app
                .clone()
                .oneshot(atompub_get(&other, &format!("media/{old_hash}/{filename}")))
                .await
                .unwrap();
            assert_eq!(
                shared_member.status(),
                StatusCode::OK,
                "source-owner record deletion must preserve the independent owner's record"
            );
            assert!(
                env.media()
                    .get_media(other.user_id, &old_hash, &filename, &MediaSource::Upload)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        assert_eq!(std::fs::read(&new_path).unwrap(), golden);
    }
}

// ─── upload_media ─────────────────────────────────────────────

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
            bytes: JPEG,
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
    body.extend_from_slice(JPEG);
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
            bytes: JPEG,
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
    assert_eq!(&served[..], JPEG);
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
            bytes: JPEG,
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
    assert_eq!(&served[..], JPEG);
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
            bytes: JPEG,
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
            bytes: JPEG,
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
async fn rejected_raster_upload_preserves_empty_durable_state(#[case] backend: Backend) {
    for over_quota in [false, true] {
        let quota = if over_quota {
            "5".parse().unwrap()
        } else {
            UserQuota::default()
        };
        let env = backend
            .setup()
            .media_limits(MaxFileSize::default(), quota)
            .await;
        let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
        let storage = TempDir::new().unwrap();
        let app = make_app!(&env, &storage);
        let bytes: &[u8] = if over_quota {
            include_bytes!("../../../host/src/image_sanitizer_fixtures/png-original.png")
        } else {
            b"malformed raster"
        };
        let (status, body) = post_multipart(
            app,
            <web::media::Upload as ServerFn>::PATH,
            MultipartFile {
                filename: "rejected.png",
                content_type: "application/octet-stream",
                bytes,
            },
            Some(&session.cookie()),
        )
        .await;
        // The existing Leptos server-fn transport carries typed validation
        // errors in a 500 envelope; AtomPub retains its REST 400/507 statuses.
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "web validation envelope: {body}"
        );
        let error: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(
            error.get("validation").is_some(),
            "expected client validation, not infrastructure failure"
        );
        assert!(body.contains(if over_quota {
            "insufficient storage"
        } else {
            "Invalid image upload"
        }));
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
                    RowLimit::at_most(100),
                    PageOffset::default()
                )
                .await
                .unwrap()
                .is_empty()
        );
        let mut directories = vec![storage.path().join("media")];
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                assert!(
                    entry.file_type().unwrap().is_dir(),
                    "no private original, output or public file may survive"
                );
                directories.push(entry.path());
            }
        }
        assert!(
            std::fs::read_dir(storage.path().join("media/tmp"))
                .unwrap()
                .next()
                .is_none()
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn upload_media_rejects_oversized_file(#[case] backend: Backend) {
    // Cap the max file size at 5 bytes so a 14-byte upload trips PayloadTooLarge,
    // exercising `map_media_error`'s PayloadTooLarge arm.
    let env = backend
        .setup()
        .media_limits("5".parse().unwrap(), UserQuota::default())
        .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "big.jpg",
            content_type: "image/jpeg",
            bytes: JPEG,
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
    // A 5-byte user quota with a 14-byte upload trips InsufficientStorage, exercising
    // `map_media_error`'s InsufficientStorage arm.
    let env = backend
        .setup()
        .media_limits(MaxFileSize::default(), "5".parse().unwrap())
        .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    let (status, body) = post_multipart(
        app.clone(),
        <web::media::Upload as ServerFn>::PATH,
        MultipartFile {
            filename: "big.jpg",
            content_type: "image/jpeg",
            bytes: JPEG,
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
