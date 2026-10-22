use async_trait::async_trait;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{HeaderValue, Method, StatusCode, header},
};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::helpers::{
    ForeignReferenceResolver, atompub, atompub_at, atompub_get, atompub_location, atompub_upload,
    body_string, create_user_and_session, make_app,
};
use common::pagination::{PageOffset, RowLimit};
use common::root_relative_url::RootRelativeUrl;
use common::test_support::{
    parse_content_hash, parse_filename, parse_post_body, parse_root_relative_url,
};
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{
    Backend, SeedRawPost, backends, backends_matrix, confirmed, noop_mailer, seed_media,
};
use url::Url;

const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

#[apply(backends_matrix)]
#[case::jpeg(
    include_bytes!("../fixtures/image-privacy/jpeg-original.jpg").as_slice(),
    "image/jpeg",
    &["IFD0:Make", "IFD0:Artist", "GPS:GPSLatitude", "GPS:GPSLongitude", "ExifIFD:DateTimeOriginal", "ExifIFD:UserComment", "IPTC:Caption-Abstract", "XMP-dc:Description", "IFD1:ThumbnailOffset"]
)]
#[case::png(
    include_bytes!("../fixtures/image-privacy/png-original.png").as_slice(),
    "image/png",
    &["IFD0:Make", "IFD0:Artist", "GPS:GPSLatitude", "GPS:GPSLongitude", "PNG:Author", "PNG:Comment", "PNG:XMLcomadobexmp", "PNG:ModifyDate"]
)]
#[tokio::test]
async fn atompub_image_preparation_serves_sanitized_identity_and_rejects_invalid_input(
    backend: Backend,
    #[case] original: &[u8],
    #[case] mime: &str,
    #[case] private_tags: &[&str],
) {
    use crate::helpers::image_privacy::{jpeg_oracle_bytes, oracle_tags, rgba_frames};
    use common::media::{ByteSize, ContentHash, MediaSource};
    use sha2::{Digest, Sha256};
    let before = oracle_tags(original, &[]);
    for tag in private_tags {
        assert!(before.contains_key(*tag), "BEFORE witness absent: {tag}");
    }
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let filename = parse_filename("misleading.png");
    let response = app
        .clone()
        .oneshot(
            atompub(&session, Method::POST, "media")
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header("slug", filename.as_ref())
                .body(Body::from(original.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let member = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let entry = body_string(response)
        .await
        .parse::<host::atompub::Entry>()
        .expect("media Entry");
    let content = entry.content().expect("authoritative content");
    assert_eq!(content.content_type(), Some(mime));
    let source = content
        .src()
        .expect("authoritative sanitized URL")
        .to_owned();
    let public_url = atompub_location(&source);
    assert_eq!(
        entry
            .links()
            .iter()
            .find(|link| link.rel() == "edit-media")
            .unwrap()
            .href(),
        source
    );
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(public_url.as_ref())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get(header::CONTENT_TYPE).unwrap(), mime);
    let etag = response
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let served = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let hash = ContentHash::from_digest(Sha256::digest(&served).into());
    assert_eq!(etag, host::etag::from_content_hash(&hash).as_ref());
    assert!(source.contains(hash.as_ref()) && member.contains(hash.as_ref()));
    assert!(
        source.ends_with(filename.as_ref()),
        "canonical filename must remain unchanged"
    );
    let after = oracle_tags(&served, &[]);
    for tag in private_tags {
        assert!(
            !after.contains_key(*tag),
            "public image retains private tag: {tag}"
        );
    }
    if mime == "image/png" {
        for (tag, expected) in [
            ("PNG:ImageWidth", serde_json::json!(32)),
            ("PNG:ImageHeight", serde_json::json!(24)),
            ("IFD0:Orientation", serde_json::json!(6)),
            ("PNG:ColorType", serde_json::json!(6)),
        ] {
            assert_eq!(before.get(tag), Some(&expected), "BEFORE control: {tag}");
            assert_eq!(after.get(tag), Some(&expected), "public control: {tag}");
        }
    }
    if let Some(frames) = rgba_frames(original, mime) {
        assert!(!frames.is_empty(), "BEFORE decoded frames absent");
        assert!(
            frames
                .iter()
                .flat_map(|f| f.buffer().pixels())
                .any(|p| p.0[3] < 255),
            "BEFORE transparency witness absent"
        );
        let public_frames = rgba_frames(&served, mime).expect("same public format");
        assert_eq!(frames.len(), public_frames.len());
        for (before, after) in frames.iter().zip(&public_frames) {
            assert!(before.buffer() == after.buffer(), "decoded RGBA changed");
            assert_eq!(before.delay(), after.delay(), "frame timing changed");
            assert_eq!((before.left(), before.top()), (after.left(), after.top()));
        }
    }
    let profile = jpeg_oracle_bytes(original, &["-b", "-ICC_Profile"]);
    assert!(!profile.is_empty(), "BEFORE ICC witness absent");
    assert!(
        profile == jpeg_oracle_bytes(&served, &["-b", "-ICC_Profile"]),
        "ICC bytes changed"
    );
    assert!(
        jpeg_oracle_bytes(original, &["-all=", "-o", "-"])
            == jpeg_oracle_bytes(&served, &["-all=", "-o", "-"]),
        "encoded image content changed"
    );
    let size = ByteSize::try_from(i64::try_from(served.len()).expect("served image size fits i64"))
        .unwrap();
    let record = env
        .media()
        .get_media(session.user_id, &hash, &filename, &MediaSource::Upload)
        .await
        .unwrap()
        .expect("persisted sanitized Media Record");
    assert_eq!(record.size_bytes, size);
    assert_eq!(record.content_type, mime);
    for bytes in [original, served.as_ref()] {
        let response = app
            .clone()
            .oneshot(
                atompub(&session, Method::POST, "media")
                    .header(header::CONTENT_TYPE, "image/png")
                    .header("slug", filename.as_ref())
                    .body(Body::from(bytes.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "same identity is an idempotent reupload"
        );
        let repeated = body_string(response)
            .await
            .parse::<host::atompub::Entry>()
            .unwrap();
        assert_eq!(repeated.content().unwrap().src(), Some(source.as_str()));
    }
    let original_hash = ContentHash::from_digest(Sha256::digest(original).into());
    assert_ne!(
        hash, original_hash,
        "planted metadata must change the stored identity"
    );
    let old_url = common::media::url(&MediaSource::Upload, &original_hash, &filename);
    assert_eq!(
        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(old_url.as_ref())
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    for (bytes, expected) in [
        (b"not an image".as_slice(), StatusCode::BAD_REQUEST),
        (
            b"\xff\xd8\xff\xe1\x00\x20broken".as_slice(),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                atompub(&session, Method::POST, "media")
                    .header(header::CONTENT_TYPE, "image/jpeg")
                    .header("slug", "rejected.jpg")
                    .body(Body::from(bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            expected,
            "invalid input must not publish an original"
        );
        assert!(response.headers().get(header::LOCATION).is_none());
        let rejected_hash = ContentHash::from_digest(Sha256::digest(bytes).into());
        assert!(
            env.media()
                .get_media(
                    session.user_id,
                    &rejected_hash,
                    &parse_filename("rejected.jpg"),
                    &MediaSource::Upload
                )
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        env.media()
            .get_user_upload_usage(session.user_id)
            .await
            .unwrap(),
        size
    );
    assert_eq!(
        env.media()
            .list_media(
                session.user_id,
                None,
                RowLimit::at_most(10),
                PageOffset::default()
            )
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        std::fs::read_dir(storage.path().join("media/tmp"))
            .unwrap()
            .next()
            .is_none()
    );
}

const OWNER_RETAINED_DETAIL: &str = "Media is referenced by retained Posts or revisions. Use Jaunder's web media library to review references before deleting.";
const GLOBAL_SAFETY_DETAIL: &str = "Media deletion is blocked because Jaunder cannot prove that removing this record would preserve referenced media.";

struct MatchingInstanceTransport {
    header: Vec<u8>,
}

#[async_trait]
impl jaunder::media_ownership::HeadTransport for MatchingInstanceTransport {
    async fn head(
        &self,
        _target: &Url,
    ) -> Result<jaunder::media_ownership::HeadResponse, jaunder::media_ownership::HeadTransportError>
    {
        Ok(jaunder::media_ownership::HeadResponse::new(vec![
            self.header.clone(),
        ]))
    }
}

async fn assert_media_delete_conflict(
    response: axum::response::Response,
    detail: &str,
    post_ids: &[i64],
) {
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/problem+json")
    );
    let post_ids = post_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(
        body_string(response).await,
        format!(
            r#"{{"type":"https://jaunder.org/problems/media-delete-conflict","title":"Media deletion refused","status":409,"detail":"{detail}","post_ids":[{post_ids}],"theme_reference_count":0}}"#
        )
    );
}

#[apply(backends)]
#[tokio::test]
async fn upload_returns_201_and_media_link_entry(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let response = app
        .oneshot(atompub_upload(&session, "pic.png", PNG))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let loc = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    assert!(loc.starts_with(&format!(
        "https://example.com/atompub/{}/media/",
        session.username
    )));

    let body = body_string(response).await;
    assert!(body.contains("rel=\"edit-media\""), "body: {body}");
    assert!(body.contains("type=\"image/png\""), "body: {body}");
    assert!(
        body.contains("https://example.com/media/upload/"),
        "body: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn member_delete_masks_closed_write_storage_and_retains_the_typed_handler_cause(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let base = &env.base;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().expect("temporary media root");
    let app = make_app!(&env, &storage);
    base.close_pool().await;
    let hash =
        parse_content_hash("0000000000000000000000000000000000000000000000000000000000000000");

    let response = app
        .oneshot(
            atompub(
                &session,
                Method::DELETE,
                &format!("media/{hash}/closed-storage.png"),
            )
            .body(Body::empty())
            .expect("valid AtomPub DELETE"),
        )
        .await
        .expect("handler response");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body_string(response).await.is_empty(),
        "the AtomPub boundary masks the typed storage failure"
    );
}

#[apply(backends)]
#[tokio::test]
async fn live_instance_proven_absolute_and_scheme_relative_posts_materialize_independent_records(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let source = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let author = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let absolute_media = seed_media(
        std::sync::Arc::clone(&env.media()),
        env.write_scope(),
        source.user_id,
        "live-proven-absolute.png",
    )
    .await;
    let scheme_relative_media = seed_media(
        std::sync::Arc::clone(&env.media()),
        env.write_scope(),
        source.user_id,
        "live-proven-scheme-relative.png",
    )
    .await;
    let storage = TempDir::new().expect("temporary media root");
    let instance_id = storage::InstanceId::new();
    let resolver = Arc::new(
        jaunder::media_ownership::LiveMediaReferenceOwnershipResolver::with_transport(
            MatchingInstanceTransport {
                header: instance_id.to_string().into_bytes(),
            },
        ),
    );
    std::fs::create_dir_all(storage.path().join("media").join("upload"))
        .expect("media upload directory");
    std::fs::create_dir_all(storage.path().join("media").join("cached"))
        .expect("media cache directory");
    std::fs::create_dir_all(storage.path().join("media").join("tmp"))
        .expect("media temporary directory");
    let app = make_app!(
        &env,
        &storage;
        instance_id = instance_id,
        mailer = noop_mailer(),
        secure_cookies = false,
        resolver = resolver
    );
    for (title, media, origin) in [
        ("Proven absolute", &absolute_media, "https://example.com"),
        (
            "Proven scheme relative",
            &scheme_relative_media,
            "//example.com",
        ),
    ] {
        let local_url = common::media::url(&media.source, &media.sha256, &media.filename);
        let reference = format!("{origin}{local_url}");
        let response = app
            .clone()
            .oneshot(
                atompub(&author, Method::POST, "posts")
                    .header("content-type", "application/atom+xml")
                    .body(Body::from(format!(
                        "<?xml version=\"1.0\"?><entry xmlns=\"http://www.w3.org/2005/Atom\"><title>{title}</title><content type=\"html\">&lt;img src=\"{reference}\"&gt;</content></entry>"
                    )))
                    .expect("valid AtomPub post request"),
            )
            .await
            .expect("post response");
        assert_eq!(response.status(), StatusCode::CREATED);
        assert!(
            env.media()
                .get_media(
                    author.user_id,
                    &media.sha256,
                    &media.filename,
                    &media.source
                )
                .await
                .expect("author media lookup")
                .is_some(),
            "{title} live-instance proof materializes an independent media record"
        );
    }
}
#[apply(backends)]
#[tokio::test]
async fn disabled_upload_is_forbidden_without_media_mutation(#[case] backend: Backend) {
    let env = backend.setup().media_uploads_enabled(false).await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let response = app
        .oneshot(atompub_upload(&session, "blocked.png", PNG))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_string(response).await, "media uploads are disabled");
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
async fn upload_above_two_mib_honors_configured_limit(#[case] backend: Backend) {
    use common::media::{MediaSource, UserQuota};
    let size = 2 * 1024 * 1024 + 1;
    let env = backend
        .setup()
        .media_limits(size.to_string().parse().unwrap(), UserQuota::default())
        .await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let bytes = vec![b'x'; size];
    let response = app
        .oneshot(
            atompub(&session, Method::POST, "media")
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header("slug", "large.bin")
                .body(Body::from(bytes.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(response.headers().get(header::LOCATION).is_some());
    let entry = body_string(response)
        .await
        .parse::<host::atompub::Entry>()
        .unwrap();
    assert_eq!(
        entry.content().unwrap().content_type(),
        Some("application/octet-stream")
    );
    assert!(
        entry
            .content()
            .unwrap()
            .src()
            .unwrap()
            .ends_with("/large.bin")
    );
    let records = env
        .media()
        .list_media(
            session.user_id,
            None,
            RowLimit::at_most(10),
            PageOffset::default(),
        )
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].size_bytes.value(), i64::try_from(size).unwrap());
    assert_eq!(records[0].source, MediaSource::Upload);
}

#[apply(backends_matrix)]
#[case::below_small_limit(1024, 1023, StatusCode::CREATED)]
#[case::at_small_limit(1024, 1024, StatusCode::CREATED)]
#[case::above_small_limit(1024, 1025, StatusCode::PAYLOAD_TOO_LARGE)]
#[case::above_large_limit(3 * 1024 * 1024, 3 * 1024 * 1024 + 1, StatusCode::PAYLOAD_TOO_LARGE)]
#[tokio::test]
async fn upload_enforces_configured_input_boundary(
    backend: Backend,
    #[case] limit: usize,
    #[case] size: usize,
    #[case] expected: StatusCode,
) {
    use common::media::UserQuota;
    let env = backend
        .setup()
        .media_limits(limit.to_string().parse().unwrap(), UserQuota::default())
        .await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let request = atompub(&session, Method::POST, "media").header("slug", "boundary.bin");
    // An oversized request must be rejected during bounded extraction, before
    // content-type parsing or the shared preparation/persistence path.
    let request = if size > limit {
        request.header(header::CONTENT_TYPE, "invalid")
    } else {
        request.header(header::CONTENT_TYPE, "application/octet-stream")
    };
    // Unknown-length chunks also obey the bound: extraction must stop at the
    // oversized chunk, without reading the subsequent broken transport chunk.
    let body = if size > limit {
        Body::from_stream(futures_util::stream::iter([
            Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![b'x'; size])),
            Err(std::io::Error::other("must not read past the limit")),
        ]))
    } else {
        Body::from(vec![b'x'; size])
    };
    let response = app.oneshot(request.body(body).unwrap()).await.unwrap();
    assert_eq!(response.status(), expected);
    if expected == StatusCode::PAYLOAD_TOO_LARGE {
        assert!(response.headers().get(header::LOCATION).is_none());
        assert_eq!(body_string(response).await, "");
    }
    let records = env
        .media()
        .list_media(
            session.user_id,
            None,
            RowLimit::at_most(10),
            PageOffset::default(),
        )
        .await
        .unwrap();
    if size <= limit {
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].size_bytes.value(), i64::try_from(size).unwrap());
    } else {
        assert!(records.is_empty());
        for dir in ["media/tmp", "media/upload"] {
            assert!(
                std::fs::read_dir(storage.path().join(dir))
                    .unwrap()
                    .next()
                    .is_none()
            );
        }
    }
}

#[apply(backends)]
#[tokio::test]
async fn upload_rejects_unauthorized_large_body_without_persistence(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method(Method::POST)
                .uri(format!("/atompub/{}/media", session.username))
                .body(Body::from(vec![b'x'; 2 * 1024 * 1024 + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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
}

#[apply(backends)]
#[tokio::test]
async fn upload_body_read_failure_is_not_a_size_error(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let body = Body::from_stream(futures_util::stream::once(async {
        Err::<axum::body::Bytes, _>(std::io::Error::other("broken upload transport"))
    }));
    let response = app
        .oneshot(atompub(&session, Method::POST, "media").body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
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
}

#[apply(backends)]
#[tokio::test]
async fn upload_accepts_pdf_content_type(#[case] backend: Backend) {
    let env = backend.setup().media_uploads_enabled(true).await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let response = app
        .oneshot(
            atompub(&session, Method::POST, "media")
                .header(header::CONTENT_TYPE, "application/pdf")
                .header("slug", "document.pdf")
                .body(Body::from("PDF-BYTES"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = body_string(response).await;
    assert!(body.contains("type=\"application/pdf\""), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn upload_without_content_type_detects_images_and_preserves_non_image_defaults(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    for (bytes, filename, expected) in [
        (PNG, "upload.bin", "image/png"),
        (
            b"%PDF-1.7\nsynthetic document\n".as_slice(),
            "document.bin",
            "application/octet-stream",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                atompub(&session, Method::POST, "media")
                    .header("slug", filename)
                    .body(Body::from(bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let entry = body_string(response)
            .await
            .parse::<host::atompub::Entry>()
            .unwrap();
        assert_eq!(entry.content().unwrap().content_type(), Some(expected));
    }
}

#[apply(backends)]
#[tokio::test]
async fn upload_rejects_invalid_present_content_type(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let response = app
        .oneshot(
            atompub(&session, Method::POST, "media")
                .header(header::CONTENT_TYPE, "text")
                .header("slug", "upload.txt")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[apply(backends)]
#[tokio::test]
async fn upload_rejects_opaque_present_content_type(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let response = app
        .oneshot(
            atompub(&session, Method::POST, "media")
                .header(
                    header::CONTENT_TYPE,
                    HeaderValue::from_bytes(&[0xff]).expect("opaque header value"),
                )
                .header("slug", "upload.bin")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[apply(backends)]
#[tokio::test]
async fn reupload_identical_returns_200(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let _resp1 = app
        .clone()
        .oneshot(atompub_upload(&session, "pic.png", PNG))
        .await
        .unwrap();

    // Second upload (identical)
    let resp2 = app
        .oneshot(atompub_upload(&session, "pic.png", PNG))
        .await
        .unwrap();

    assert_eq!(resp2.status(), StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn get_media_member_returns_entry(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let resp = app
        .clone()
        .oneshot(atompub_upload(&session, "pic.png", PNG))
        .await
        .unwrap();

    let loc = atompub_location(
        resp.headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    let get_resp = app
        .oneshot(
            atompub_at(&session, Method::GET, &loc)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK);
    let body = body_string(get_resp).await;
    assert!(body.contains("rel=\"edit-media\""), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn disabled_uploads_leave_existing_media_readable_and_deletable(#[case] backend: Backend) {
    let env = backend.setup().media_uploads_enabled(false).await;
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
        "existing.png",
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let location = parse_root_relative_url(&format!(
        "/atompub/{}/media/{}/{}",
        session.username, media.sha256, media.filename
    ));

    let get_response = app
        .clone()
        .oneshot(
            atompub_at(&session, Method::GET, &location)
                .body(Body::empty())
                .expect("failed to build AtomPub GET request"),
        )
        .await
        .unwrap();
    assert_eq!(get_response.status(), StatusCode::OK);

    let delete_response = app
        .oneshot(
            atompub_at(&session, Method::DELETE, &location)
                .body(Body::empty())
                .expect("failed to build AtomPub DELETE request"),
        )
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);
}

#[apply(backends)]
#[tokio::test]
async fn get_unknown_media_returns_404(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let response = app
        .oneshot(atompub_get(
            &session,
            // A well-formed but never-uploaded hash: the typed extractor accepts it,
            // and the handler returns 404 for the absent record (a *malformed* hash
            // would be a pre-handler 400 — see member_rejects_malformed_segment).
            "media/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/none.png",
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn delete_media_member_returns_204_then_404(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let resp = app
        .clone()
        .oneshot(atompub_upload(&session, "pic.png", PNG))
        .await
        .unwrap();

    let loc = atompub_location(
        resp.headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    let del_resp = app
        .clone()
        .oneshot(
            atompub_at(&session, Method::DELETE, &loc)
                .body(Body::empty())
                .expect("failed to build atompub request"),
        )
        .await
        .unwrap();

    assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

    // Second delete (should be 404)
    let del_resp2 = app
        .oneshot(
            atompub_at(&session, Method::DELETE, &loc)
                .body(Body::empty())
                .expect("failed to build atompub request"),
        )
        .await
        .unwrap();

    assert_eq!(del_resp2.status(), StatusCode::NOT_FOUND);
}
#[apply(backends)]
#[tokio::test]
async fn delete_media_member_reports_owner_live_post_and_preserves_media(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let loc = upload_and_member_url(&app, &session, "referenced.png").await;
    let sha256 = loc
        .as_ref()
        .rsplit('/')
        .nth(1)
        .map(parse_content_hash)
        .expect("member URL includes the content hash");
    let filename = parse_filename("referenced.png");
    let media_url = common::media::url(&common::media::MediaSource::Upload, &sha256, &filename);
    let post = SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!("![referenced]({media_url})")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    let response = app
        .clone()
        .oneshot(
            atompub_at(&session, Method::DELETE, &loc)
                .body(Body::empty())
                .expect("failed to build atompub DELETE request"),
        )
        .await
        .unwrap();

    assert_media_delete_conflict(response, OWNER_RETAINED_DETAIL, &[i64::from(post.post_id)]).await;
    assert_eq!(
        app.oneshot(
            atompub_at(&session, Method::GET, &loc)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap()
        .status(),
        StatusCode::OK,
        "a refused deletion preserves the media Member"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_media_member_reports_unique_ascending_owner_live_post_ids(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let loc = upload_and_member_url(&app, &session, "many-references.png").await;
    let sha256 = loc
        .as_ref()
        .rsplit('/')
        .nth(1)
        .map(parse_content_hash)
        .expect("member URL includes the content hash");
    let filename = parse_filename("many-references.png");
    let media_url = common::media::url(&common::media::MediaSource::Upload, &sha256, &filename);
    let first = SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!("![first]({media_url})")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;
    let second = SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!("<img src=\"{media_url}\">")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;
    let mut expected = [i64::from(first.post_id), i64::from(second.post_id)];
    expected.sort_unstable();

    let response = app
        .oneshot(
            atompub_at(&session, Method::DELETE, &loc)
                .body(Body::empty())
                .expect("failed to build atompub DELETE request"),
        )
        .await
        .unwrap();

    assert_media_delete_conflict(response, OWNER_RETAINED_DETAIL, &expected).await;
}

#[apply(backends)]
#[tokio::test]
async fn delete_media_member_reports_deleted_post_and_revision_once(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let loc = upload_and_member_url(&app, &session, "deleted-reference.png").await;
    let sha256 = loc
        .as_ref()
        .rsplit('/')
        .nth(1)
        .map(parse_content_hash)
        .expect("member URL includes the content hash");
    let filename = parse_filename("deleted-reference.png");
    let media_url = common::media::url(&common::media::MediaSource::Upload, &sha256, &filename);
    let post = SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!("![deleted]({media_url})")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;
    let outcome = storage::soft_delete_post(
        &env.write_scope(),
        Arc::clone(&env.posts()),
        Arc::clone(&env.feed_events()),
        post.post_id,
        session.user_id,
        common::time::UtcInstant::now(),
    )
    .await
    .expect("soft delete succeeds");
    confirmed(outcome);

    let response = app
        .clone()
        .oneshot(
            atompub_at(&session, Method::DELETE, &loc)
                .body(Body::empty())
                .expect("failed to build atompub DELETE request"),
        )
        .await
        .unwrap();

    assert_media_delete_conflict(response, OWNER_RETAINED_DETAIL, &[i64::from(post.post_id)]).await;
    assert_eq!(
        app.oneshot(
            atompub_at(&session, Method::GET, &loc)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap()
        .status(),
        StatusCode::OK,
        "Deleted Post and Revision references preserve the media Member"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_media_member_prefers_global_safety_over_owner_ids(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let other_owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let app = make_app!(&env, &storage);

    let loc = upload_and_member_url(&app, &session, "pic.png").await;
    let sha256 = loc
        .as_ref()
        .rsplit('/')
        .nth(1)
        .map(parse_content_hash)
        .expect("member URL includes the content hash");
    let filename = parse_filename("pic.png");
    let media_url = common::media::url(&common::media::MediaSource::Upload, &sha256, &filename);
    SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!("![referenced]({media_url})")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    SeedRawPost::new(other_owner.user_id)
        .body(parse_post_body(&format!(
            "<img src=\"https://unknown.example{media_url}\">"
        )))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;
    let del_resp = app
        .oneshot(
            atompub_at(&session, Method::DELETE, &loc)
                .body(Body::empty())
                .expect("failed to build atompub DELETE request"),
        )
        .await
        .unwrap();

    assert_media_delete_conflict(del_resp, GLOBAL_SAFETY_DETAIL, &[]).await;
}

#[apply(backends)]
#[tokio::test]
async fn delete_media_member_returns_409_for_another_owners_retained_reference(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let other_owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let loc = upload_and_member_url(&app, &owner, "retained.png").await;
    let sha256 = loc
        .as_ref()
        .rsplit('/')
        .nth(1)
        .map(parse_content_hash)
        .expect("member URL includes the content hash");
    let media_url = common::media::url(
        &common::media::MediaSource::Upload,
        &sha256,
        &parse_filename("retained.png"),
    );
    SeedRawPost::new(other_owner.user_id)
        .body(parse_post_body(&format!("<img src=\"{media_url}\">")))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    let response = app
        .clone()
        .oneshot(
            atompub_at(&owner, Method::DELETE, &loc)
                .body(Body::empty())
                .expect("failed to build atompub DELETE request"),
        )
        .await
        .unwrap();

    assert_media_delete_conflict(response, GLOBAL_SAFETY_DETAIL, &[]).await;
    let retained = app
        .oneshot(atompub_get(&owner, loc.as_ref()))
        .await
        .unwrap();
    assert_eq!(
        retained.status(),
        StatusCode::OK,
        "a refused delete retains the media row"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_media_member_has_no_force_override(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let foreign_session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let resolver = Arc::new(ForeignReferenceResolver::new([]));
    let app = make_app!(
        &env,
        &storage;
        instance_id = storage::InstanceId::new(),
        mailer = noop_mailer(),
        secure_cookies = false,
        resolver = resolver.clone()
    );
    let exact_member_url = upload_and_member_url(&app, &session, "exact-origin.png").await;
    let exact_hash = exact_member_url
        .rsplit('/')
        .nth(1)
        .map(parse_content_hash)
        .expect("member URL includes the content hash");
    let exact_filename = parse_filename("exact-origin.png");
    let exact_media_url = common::media::url(
        &common::media::MediaSource::Upload,
        &exact_hash,
        &exact_filename,
    );
    let exact_post = SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!(
            "<img src=\"https://example.com{exact_media_url}\">"
        )))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    let forced_member_url =
        parse_root_relative_url(&format!("{}?force=true", exact_member_url.as_ref()));
    let forced_delete = app
        .clone()
        .oneshot(
            atompub_at(&session, Method::DELETE, &forced_member_url)
                .header("x-jaunder-force", "true")
                .body(Body::empty())
                .expect("failed to build forced AtomPub DELETE request"),
        )
        .await
        .unwrap();
    assert_media_delete_conflict(
        forced_delete,
        OWNER_RETAINED_DETAIL,
        &[i64::from(exact_post.post_id)],
    )
    .await;

    let retry_delete = app
        .clone()
        .oneshot(
            atompub_at(&session, Method::DELETE, &exact_member_url)
                .body(Body::empty())
                .expect("failed to build repeated AtomPub DELETE request"),
        )
        .await
        .unwrap();
    assert_media_delete_conflict(
        retry_delete,
        OWNER_RETAINED_DETAIL,
        &[i64::from(exact_post.post_id)],
    )
    .await;
    assert_eq!(
        app.clone()
            .oneshot(atompub_get(&session, exact_member_url.as_ref()))
            .await
            .unwrap()
            .status(),
        StatusCode::OK,
        "query, header, and retry attempts cannot override guarded deletion"
    );

    let foreign_member_url = upload_and_member_url(&app, &session, "foreign-origin.png").await;
    let foreign_hash = foreign_member_url
        .rsplit('/')
        .nth(1)
        .map(parse_content_hash)
        .expect("member URL includes the content hash");
    let foreign_filename = parse_filename("foreign-origin.png");
    let foreign_media_url = common::media::url(
        &common::media::MediaSource::Upload,
        &foreign_hash,
        &foreign_filename,
    );
    SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!(
            "<img src=\"https://foreign.example{foreign_media_url}\">"
        )))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;

    resolver.insert_foreign_form(
        format!("https://foreign.example{foreign_media_url}")
            .parse()
            .expect("valid media reference form"),
    );
    let foreign_delete = app
        .clone()
        .oneshot(
            atompub_at(&session, Method::DELETE, &foreign_member_url)
                .body(Body::empty())
                .expect("failed to build atompub DELETE request"),
        )
        .await
        .unwrap();
    assert_eq!(
        foreign_delete.status(),
        StatusCode::NO_CONTENT,
        "foreign-origin reference does not guard deletion"
    );

    let unknown_member_url = upload_and_member_url(&app, &session, "unknown-origin.png").await;
    let unknown_hash = unknown_member_url
        .rsplit('/')
        .nth(1)
        .map(parse_content_hash)
        .expect("member URL includes the content hash");
    let unknown_media_url = common::media::url(
        &common::media::MediaSource::Upload,
        &unknown_hash,
        &parse_filename("unknown-origin.png"),
    );
    SeedRawPost::new(foreign_session.user_id)
        .body(parse_post_body(&format!(
            "<img src=\"https://unknown.example{unknown_media_url}\">"
        )))
        .seed(std::sync::Arc::clone(&env.posts()), env.write_scope())
        .await;
    let unknown_delete = app
        .clone()
        .oneshot(
            atompub_at(&session, Method::DELETE, &unknown_member_url)
                .body(Body::empty())
                .expect("failed to build atompub DELETE request"),
        )
        .await
        .unwrap();
    assert_media_delete_conflict(unknown_delete, GLOBAL_SAFETY_DETAIL, &[]).await;
    assert_eq!(
        app.oneshot(
            atompub_at(&session, Method::GET, &unknown_member_url)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap()
        .status(),
        StatusCode::OK,
        "global evidence uncertainty preserves the media Member"
    );
}

/// Replaces the trailing filename segment of a member URL, keeping everything before it.
/// Used to aim a request at a name the server never minted.
fn with_filename_segment(member_url: &RootRelativeUrl, segment: &str) -> RootRelativeUrl {
    let (prefix, _old) = member_url
        .as_ref()
        .rsplit_once('/')
        .expect("a member URL always has a trailing filename segment");
    parse_root_relative_url(&format!("{prefix}/{segment}"))
}

/// Uploads `slug` and returns the member URL the server minted for it.
async fn upload_and_member_url(
    app: &axum::Router,
    session: &crate::helpers::SeededSession,
    slug: &str,
) -> RootRelativeUrl {
    let resp = app
        .clone()
        .oneshot(atompub_upload(session, slug, PNG))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "uploading {slug:?}");
    atompub_location(
        resp.headers()
            .get(header::LOCATION)
            .expect("a created media member carries a Location")
            .to_str()
            .expect("Location is ASCII"),
    )
}

#[apply(backends)]
#[tokio::test]
async fn member_get_resolves_a_filename_needing_encoding(#[case] backend: Backend) {
    // The decoded-segment conversion proof for `member_get` (#720). Every other test in
    // this file uses `pic.png`, which encodes to itself — so none would fail if the
    // private member-address extractor skipped re-encoding. This one would: Axum decodes
    // the `my%20photo.jpg` segment to `my photo.jpg`, and only the conversion recovers
    // the stored spelling to match the row.
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let loc = upload_and_member_url(&app, &session, "my photo.jpg").await;
    assert!(
        loc.as_ref().ends_with("/my%20photo.jpg"),
        "minted URL: {loc}"
    );

    let get_resp = app
        .oneshot(
            atompub_at(&session, Method::GET, &loc)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK, "fetching {loc}");
    let body = body_string(get_resp).await;
    // The entry we got back is the one we stored: its member URL carries the canonical
    // spelling, byte-identical to the segment we requested. (The `<title>` is the decoded
    // display view — asserted separately, where that decode lands.)
    assert!(body.contains("/my%20photo.jpg\""), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn member_delete_resolves_a_filename_needing_encoding(#[case] backend: Backend) {
    // As above, for `member_delete`: the delete must match the stored row rather than
    // missing it, which the follow-up 404 confirms actually happened.
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let loc = upload_and_member_url(&app, &session, "my photo.jpg").await;

    let del_resp = app
        .clone()
        .oneshot(
            atompub_at(&session, Method::DELETE, &loc)
                .body(Body::empty())
                .expect("failed to build atompub request"),
        )
        .await
        .unwrap();
    assert_eq!(del_resp.status(), StatusCode::NO_CONTENT, "deleting {loc}");

    let get_resp = app
        .oneshot(
            atompub_at(&session, Method::GET, &loc)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn an_over_long_segment_does_not_truncate_onto_a_stored_name(#[case] backend: Backend) {
    // The discriminating test for "checks, never repairs" (#720, AC6). Asserting merely
    // that an over-long segment does not resolve would pass whether or not truncation was
    // removed — a name that never matched anything does not resolve either. So: store a
    // name sitting exactly at the budget, then request a *longer* one whose truncation
    // would land on it. If the decoded-segment conversion ever repaired instead of
    // rejecting, this would resolve to another user's file rather than missing.
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    let at_budget = "a".repeat(common::media::MAX_FILENAME_ENCODED_BYTES);
    let loc = upload_and_member_url(&app, &session, &at_budget).await;
    assert!(loc.as_ref().ends_with(&at_budget), "minted URL: {loc}");

    let over_budget = "a".repeat(common::media::MAX_FILENAME_ENCODED_BYTES + 1);
    let aimed = with_filename_segment(&loc, &over_budget);

    let resp = app
        .oneshot(
            atompub_at(&session, Method::GET, &aimed)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap();

    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "an over-long segment must never resolve onto the stored name"
    );
    // Rejected at the door, so it is a pre-handler 400 rather than a lookup 404.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[apply(backends)]
#[tokio::test]
async fn upload_forbids_other_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;

    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let uri = parse_root_relative_url("/atompub/bob/media");

    let response = app
        .oneshot(
            atompub_at(&session, Method::POST, &uri)
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[apply(backends)]
#[tokio::test]
async fn upload_rejects_empty_slug(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    // ".." sanitizes to an empty filename.
    let response = app
        .oneshot(
            atompub(&session, Method::POST, "media")
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "..")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// Shape B — accessing another user's media member is forbidden regardless of
// method. Identical setup (alice authenticated, bob's resource) + assertion;
// only the HTTP method varies.
#[apply(backends_matrix)]
#[case::get(Method::GET)]
#[case::delete(Method::DELETE)]
#[tokio::test]
async fn member_forbids_other_user(backend: Backend, #[case] method: Method) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);
    let uri = parse_root_relative_url(
        "/atompub/bob/media/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/pic.png",
    );

    let response = app
        .oneshot(
            atompub_at(
                &session, method,
                // A well-formed hash so the typed extractor passes and the wrong-user
                // check (alice authenticated, bob's namespace) is what yields 403.
                &uri,
            )
            .body(Body::empty())
            .expect("failed to build atompub request"),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// A malformed `{sha}` or `{filename}` segment on the authenticated member routes is
// rejected by the private member-address extractor as a pre-handler 400 (the URL is one
// we minted, so a bad segment is the caller's fault) — distinct from a
// well-formed-but-absent resource, which is 404 above.
//
// This test is unchanged by #720, and deliberately so: the decoded-segment conversion
// runs the safe-leaf oracle before re-encoding, so `a\b.png` is still rejected at the
// door. Had the oracle been dropped or moved after the encode, this would have quietly
// become a 404 lookup miss instead.
#[apply(backends)]
#[tokio::test]
async fn member_rejects_malformed_segment_returns_400(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let storage = TempDir::new().unwrap();
    let app = make_app!(&env, &storage);

    // Malformed hash segment (`deadbeef` is not 64 hex) → ContentHash parse fails → 400.
    let bad_hash = app
        .clone()
        .oneshot(atompub_get(&session, "media/deadbeef/pic.png"))
        .await
        .unwrap();
    assert_eq!(bad_hash.status(), StatusCode::BAD_REQUEST);

    // Non-canonical filename segment (`a%5Cb.png` decodes to `a\b.png`, not a safe leaf)
    // → decoded-segment conversion fails → 400.
    let bad_name = app
        .oneshot(atompub_get(
            &session,
            "media/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/a%5Cb.png",
        ))
        .await
        .unwrap();
    assert_eq!(bad_name.status(), StatusCode::BAD_REQUEST);
}
