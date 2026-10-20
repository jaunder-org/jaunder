//! Disposable operator rehearsal: preexisting bytes are fixture state, while every
//! replacement crosses the real mandatory upload worker and supported writes.
use std::path::Path;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use common::ids::UserId;
use common::media::{ByteSize, ContentHash, MediaRef, MediaSource};
use common::pagination::PageSize;
use common::test_support::{parse_filename, parse_post_body};
use common::time::UtcInstant;
use rstest::*;
use rstest_reuse::*;
use sha2::{Digest, Sha256};
use storage::MediaRecord;
use storage::test_support::{Backend, SeedRawPost, TestEnv, backends, confirmed};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::helpers::image_privacy::oracle_tags;
use crate::helpers::{
    SeededSession, atompub, atompub_at, atompub_location, body_string, confirmed_mutation,
    create_user_and_session, make_app, post_server_fn,
};

fn digest(bytes: &[u8]) -> ContentHash {
    ContentHash::from_digest(Sha256::digest(bytes).into())
}

// This simulates an immutable object that predates mandatory preparation. It is
// deliberately test-fixture state, not a new upload door or sanitizer substitute.
async fn preexisting_image(
    env: &TestEnv,
    root: &Path,
    owner: UserId,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> MediaRef {
    let media = MediaRef {
        source: MediaSource::Upload,
        sha256: digest(bytes),
        filename: parse_filename(name),
    };
    let path = root.join("media").join(common::media::path(
        &media.source,
        &media.sha256,
        &media.filename,
    ));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
    let record = MediaRecord {
        user_id: owner,
        sha256: media.sha256.clone(),
        filename: media.filename.clone(),
        source: media.source,
        content_type: mime.parse().unwrap(),
        size_bytes: ByteSize::try_from(
            i64::try_from(bytes.len()).expect("owned fixture size fits i64"),
        )
        .unwrap(),
        source_url: None,
        created_at: UtcInstant::now(),
    };
    let store = env.media();
    confirmed(
        env.write_scope()
            .run(move |transaction| {
                Box::pin(async move { store.create_media(transaction, &record).await })
            })
            .await
            .unwrap(),
    );
    media
}

async fn public_image(app: &Router, media: &MediaRef) -> Vec<u8> {
    let url = common::media::url(&media.source, &media.sha256, &media.filename);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(url.as_ref())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "rehearsal old public URL must be observed, not inferred from a Media Record"
    );
    assert_eq!(
        response.headers().get(header::ETAG).unwrap(),
        host::etag::from_content_hash(&media.sha256).as_ref()
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    assert_eq!(
        digest(&bytes),
        media.sha256,
        "public bytes disagree with their immutable identity"
    );
    bytes
}

fn entry_with_image(url: &str) -> String {
    format!(
        r#"<entry xmlns="http://www.w3.org/2005/Atom"><title>Disposable replacement rehearsal</title><content type="text">![owned image]({url})</content></entry>"#
    )
}

async fn create_post(
    app: &Router,
    session: &SeededSession,
    url: &str,
) -> common::root_relative_url::RootRelativeUrl {
    let response = app
        .clone()
        .oneshot(
            atompub(session, Method::POST, "posts")
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .body(Body::from(entry_with_image(url)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    atompub_location(
        response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    )
}

async fn web_delete(
    app: &Router,
    session: &SeededSession,
    media: &MediaRef,
    force: bool,
) -> web::media::MediaDeletion {
    let (status, body) = post_server_fn(
        app.clone(),
        &web::media::Delete {
            request: web::media::DeleteMediaRequest {
                sha256: media.sha256.clone(),
                filename: media.filename.clone(),
                source: media.source,
                force: Some(force),
            },
        },
        Some(&session.cookie()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    confirmed_mutation(&body)
}

#[apply(backends)]
#[tokio::test]
async fn replacement_rehearsal_moves_current_references_but_retains_old_public_bytes(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let root = TempDir::new().unwrap();
    let app = make_app!(&env, &root);
    let owner = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let fixtures: [(&str, &str, &[u8], &[&str]); 2] = [
        (
            "legacy.jpg",
            "image/jpeg",
            include_bytes!("../fixtures/image-privacy/jpeg-original.jpg"),
            &[
                "IFD0:Make",
                "GPS:GPSLatitude",
                "XMP-dc:Description",
                "IFD1:ThumbnailImage",
            ],
        ),
        (
            "legacy.webp",
            "image/webp",
            include_bytes!("../fixtures/image-privacy/webp-animation-original.webp"),
            &["IFD0:Make", "GPS:GPSLatitude", "XMP-dc:Description"],
        ),
    ];
    let mut expected_usage = 0i64;
    for (name, mime, original, private) in fixtures {
        let before = oracle_tags(original, &[]);
        for key in private {
            assert!(
                before.contains_key(*key),
                "positive original witness absent: {key}"
            );
        }
        let old = preexisting_image(&env, root.path(), owner.user_id, name, mime, original).await;
        assert_eq!(public_image(&app, &old).await, original);
        let old_url = common::media::url(&old.source, &old.sha256, &old.filename);
        let member = create_post(&app, &owner, old_url.as_ref()).await;
        let post_id = member.as_ref().rsplit('/').next().unwrap().parse().unwrap();
        let response = app
            .clone()
            .oneshot(
                atompub(&owner, Method::POST, "media")
                    .header(header::CONTENT_TYPE, mime)
                    .header("slug", name)
                    .body(Body::from(original.to_vec()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let replacement_entry = body_string(response)
            .await
            .parse::<host::atompub::Entry>()
            .unwrap();
        let replacement_url = replacement_entry.content().unwrap().src().unwrap();
        let reference = common::media::MediaRef {
            source: MediaSource::Upload,
            sha256: replacement_url.rsplit('/').nth(1).unwrap().parse().unwrap(),
            filename: old.filename.clone(),
        };
        assert_ne!(
            reference.sha256, old.sha256,
            "a renamed file alone is not a sanitized new identity"
        );
        assert_eq!(
            common::media::url(&reference.source, &reference.sha256, &reference.filename).as_ref(),
            atompub_location(replacement_url).as_ref()
        );
        let clean = public_image(&app, &reference).await;
        let after = oracle_tags(&clean, &[]);
        for key in private {
            assert!(
                !after.contains_key(*key),
                "replacement retains ordinary metadata: {key}"
            );
        }
        let replacement = env
            .media()
            .get_media(
                owner.user_id,
                &reference.sha256,
                &reference.filename,
                &reference.source,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            replacement.size_bytes.value(),
            i64::try_from(clean.len()).expect("replacement image size fits i64")
        );
        assert_eq!(replacement.content_type.as_ref(), mime);
        assert_eq!(
            env.media()
                .get_user_upload_usage(owner.user_id)
                .await
                .unwrap()
                .value(),
            expected_usage
                + i64::try_from(original.len()).expect("owned fixture size fits i64")
                + i64::try_from(clean.len()).expect("replacement image size fits i64")
        );
        let current = app
            .clone()
            .oneshot(
                atompub_at(&owner, Method::GET, &member)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let etag = current.headers().get(header::ETAG).unwrap().clone();
        let updated = app
            .clone()
            .oneshot(
                atompub_at(&owner, Method::PUT, &member)
                    .header(header::CONTENT_TYPE, "application/atom+xml")
                    .header(header::IF_MATCH, etag.clone())
                    .body(Body::from(entry_with_image(replacement_url)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(updated.status(), StatusCode::OK);
        assert_ne!(updated.headers().get(header::ETAG).unwrap(), &etag);
        let fetched = app
            .clone()
            .oneshot(
                atompub_at(&owner, Method::GET, &member)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(fetched.status(), StatusCode::OK);
        let current = body_string(fetched)
            .await
            .parse::<host::atompub::Entry>()
            .unwrap();
        let current_body = current.content().unwrap().value().unwrap();
        assert!(current_body.contains(replacement_url));
        assert!(!current_body.contains(old_url.as_ref()));
        let history = env
            .posts()
            .list_post_revision_history(owner.user_id, post_id, None, PageSize::default())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(history.revisions.len(), 1);
        let revision = env
            .posts()
            .get_post_revision_detail(owner.user_id, post_id, history.revisions[0].revision_id)
            .await
            .unwrap()
            .unwrap();
        assert!(revision.revision.body.as_ref().contains(old_url.as_ref()));
        let old_member = format!("media/{}/{}", old.sha256, old.filename);
        let refused = app
            .clone()
            .oneshot(
                atompub(&owner, Method::DELETE, &old_member)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::CONFLICT);
        let conflict: serde_json::Value =
            serde_json::from_str(&body_string(refused).await).unwrap();
        assert_eq!(
            conflict["post_ids"],
            serde_json::json!([i64::from(post_id)])
        );
        assert_eq!(public_image(&app, &old).await, original);
        assert_eq!(
            web_delete(&app, &owner, &old, true).await,
            web::media::MediaDeletion::Deleted
        );
        assert!(
            env.media()
                .get_media(owner.user_id, &old.sha256, &old.filename, &old.source)
                .await
                .unwrap()
                .is_none()
        );
        expected_usage += i64::try_from(clean.len()).expect("replacement image size fits i64");
        assert_eq!(
            env.media()
                .get_user_upload_usage(owner.user_id)
                .await
                .unwrap()
                .value(),
            expected_usage
        );
        // Retained history prevents byte reclamation. Public serving can still
        // open that immutable file even after the owner's final record is gone.
        assert_eq!(public_image(&app, &old).await, original);
        assert_eq!(public_image(&app, &reference).await, clean);
        assert_eq!(digest(original), old.sha256, "owned input was changed");
        println!(
            "disposable {name}: old GET 200 before replacement, after history refusal and after owner-record override; old_sha={} old_etag={} replacement_sha={} replacement_size={}",
            old.sha256,
            host::etag::from_content_hash(&old.sha256),
            reference.sha256,
            clean.len()
        );
    }
    assert!(
        std::fs::read_dir(root.path().join("media/tmp"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[apply(backends)]
#[tokio::test]
async fn replacement_rehearsal_preserves_independent_sharing_and_refuses_legacy_global_safety(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let root = TempDir::new().unwrap();
    let app = make_app!(&env, &root);
    let owner = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let other = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let jpeg = include_bytes!("../fixtures/image-privacy/jpeg-original.jpg");
    let shared = preexisting_image(
        &env,
        root.path(),
        owner.user_id,
        "shared.jpg",
        "image/jpeg",
        jpeg,
    )
    .await;
    let shared_url = common::media::url(&shared.source, &shared.sha256, &shared.filename);
    create_post(&app, &other, shared_url.as_ref()).await;
    let independent = env
        .media()
        .get_media(
            other.user_id,
            &shared.sha256,
            &shared.filename,
            &shared.source,
        )
        .await
        .unwrap()
        .expect("qualifying Post write creates independent ownership");
    assert_eq!(
        independent.size_bytes.value(),
        i64::try_from(jpeg.len()).expect("owned JPEG fixture size fits i64")
    );
    let other_usage = env
        .media()
        .get_user_upload_usage(other.user_id)
        .await
        .unwrap();
    assert_eq!(
        other_usage.value(),
        i64::try_from(jpeg.len()).expect("owned JPEG fixture size fits i64")
    );
    let removed = app
        .clone()
        .oneshot(
            atompub(
                &owner,
                Method::DELETE,
                &format!("media/{}/{}", shared.sha256, shared.filename),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    assert!(
        env.media()
            .get_media(
                owner.user_id,
                &shared.sha256,
                &shared.filename,
                &shared.source
            )
            .await
            .unwrap()
            .is_none()
    );
    let retained = env
        .media()
        .get_media(
            other.user_id,
            &shared.sha256,
            &shared.filename,
            &shared.source,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.size_bytes, independent.size_bytes);
    assert_eq!(retained.content_type, independent.content_type);
    assert_eq!(retained.created_at, independent.created_at);
    assert_eq!(
        env.media()
            .get_user_upload_usage(other.user_id)
            .await
            .unwrap(),
        other_usage
    );
    assert_eq!(public_image(&app, &shared).await, jpeg);
    let webp = include_bytes!("../fixtures/image-privacy/webp-animation-original.webp");
    let legacy = preexisting_image(
        &env,
        root.path(),
        owner.user_id,
        "unknown.webp",
        "image/webp",
        webp,
    )
    .await;
    let old_url = common::media::url(&legacy.source, &legacy.sha256, &legacy.filename);
    // The existing raw fixture models a legacy reference without a qualifying
    // independent Media Record, not a new supported Post-write path.
    SeedRawPost::new(other.user_id)
        .body(parse_post_body(&format!("![legacy]({old_url})")))
        .seed(env.posts(), env.write_scope())
        .await;
    assert!(
        env.media()
            .get_media(
                other.user_id,
                &legacy.sha256,
                &legacy.filename,
                &legacy.source
            )
            .await
            .unwrap()
            .is_none()
    );
    let usage = env
        .media()
        .get_user_upload_usage(owner.user_id)
        .await
        .unwrap();
    let refusal = app
        .clone()
        .oneshot(
            atompub(
                &owner,
                Method::DELETE,
                &format!("media/{}/{}", legacy.sha256, legacy.filename),
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refusal.status(), StatusCode::CONFLICT);
    let conflict: serde_json::Value = serde_json::from_str(&body_string(refusal).await).unwrap();
    assert_eq!(
        conflict["post_ids"],
        serde_json::json!([]),
        "global refusal must not disclose another owner's Post IDs"
    );
    assert_eq!(
        conflict,
        serde_json::json!({"type": "https://jaunder.org/problems/media-delete-conflict", "title": "Media deletion refused", "status": 409, "detail": "Media deletion is blocked because Jaunder cannot prove that removing this record would preserve referenced media.", "post_ids": [], "theme_reference_count": 0})
    );
    assert_eq!(
        web_delete(&app, &owner, &legacy, true).await,
        web::media::MediaDeletion::GlobalSafety {
            theme_reference_count: 0
        }
    );
    assert_eq!(
        env.media()
            .get_user_upload_usage(owner.user_id)
            .await
            .unwrap(),
        usage
    );
    assert_eq!(
        env.media()
            .get_user_upload_usage(other.user_id)
            .await
            .unwrap(),
        other_usage
    );
    assert!(
        env.media()
            .get_media(
                owner.user_id,
                &legacy.sha256,
                &legacy.filename,
                &legacy.source
            )
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(public_image(&app, &legacy).await, webp);
    assert_eq!(public_image(&app, &shared).await, jpeg);
    assert!(
        std::fs::read_dir(root.path().join("media/tmp"))
            .unwrap()
            .next()
            .is_none()
    );
    println!(
        "disposable sharing: old GET 200 after one record deletion; old_sha={} old_etag={}; unaccounted legacy refusal including web force leaves old GET 200 old_sha={} old_etag={} and both owners' accounting intact",
        shared.sha256,
        host::etag::from_content_hash(&shared.sha256),
        legacy.sha256,
        host::etag::from_content_hash(&legacy.sha256)
    );
}
