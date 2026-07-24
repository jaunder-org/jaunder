use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, StatusCode},
};
use tempfile::TempDir;
use tower::ServiceExt;

use rstest::*;
use rstest_reuse::*;

use storage::test_support::{backends, backends_matrix, Backend, TestEnv};

use crate::helpers::{
    atompub_authed, atompub_xml, body_string, create_user_and_session, make_app,
    setup_with_base_url,
};

const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

#[apply(backends)]
#[tokio::test]
async fn upload_returns_201_and_media_link_entry(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state, "alice").await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let response = app
        .oneshot(
            atompub_authed("POST", "/atompub/alice/media", "alice", &session.token)
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
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

    assert!(loc.starts_with("https://example.com/atompub/alice/media/"));

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
async fn reupload_identical_returns_200(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state, "alice").await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let _resp1 = app
        .clone()
        .oneshot(
            atompub_authed("POST", "/atompub/alice/media", "alice", &session.token)
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    // Second upload (identical)
    let resp2 = app
        .oneshot(
            atompub_authed("POST", "/atompub/alice/media", "alice", &session.token)
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp2.status(), StatusCode::OK);
}

#[apply(backends)]
#[tokio::test]
async fn get_media_member_returns_entry(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state, "alice").await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let resp = app
        .clone()
        .oneshot(
            atompub_authed("POST", "/atompub/alice/media", "alice", &session.token)
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    let loc = resp
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let get_resp = app
        .oneshot(atompub_xml("GET", &loc, "alice", &session.token, None))
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK);
    let body = body_string(get_resp).await;
    assert!(body.contains("rel=\"edit-media\""), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_unknown_media_returns_404(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state, "alice").await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let response = app
        .oneshot(atompub_xml(
            "GET",
            // A well-formed but never-uploaded hash: the typed extractor accepts it,
            // and the handler returns 404 for the absent record (a *malformed* hash
            // would be a pre-handler 400 — see member_rejects_malformed_segment).
            "/atompub/alice/media/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/none.png",
            "alice",
            &session.token,
            None,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn delete_media_member_returns_204_then_404(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state, "alice").await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let resp = app
        .clone()
        .oneshot(
            atompub_authed("POST", "/atompub/alice/media", "alice", &session.token)
                .header(header::CONTENT_TYPE, "image/png")
                .header("slug", "pic.png")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    let loc = resp
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let del_resp = app
        .clone()
        .oneshot(atompub_xml("DELETE", &loc, "alice", &session.token, None))
        .await
        .unwrap();

    assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

    // Second delete (should be 404)
    let del_resp2 = app
        .oneshot(atompub_xml("DELETE", &loc, "alice", &session.token, None))
        .await
        .unwrap();

    assert_eq!(del_resp2.status(), StatusCode::NOT_FOUND);
}

#[apply(backends)]
#[tokio::test]
async fn upload_forbids_other_user(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state, "alice").await;

    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    let response = app
        .oneshot(
            atompub_authed("POST", "/atompub/bob/media", "alice", &session.token)
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
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state, "alice").await;
    let storage = TempDir::new().unwrap();
    let app = make_app(state, &storage);

    // ".." sanitizes to an empty filename.
    let response = app
        .oneshot(
            atompub_authed("POST", "/atompub/alice/media", "alice", &session.token)
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
#[case::get("GET")]
#[case::delete("DELETE")]
#[tokio::test]
async fn member_forbids_other_user(backend: Backend, #[case] method: &str) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state, "alice").await;
    let storage = TempDir::new().unwrap();
    let app = make_app(state, &storage);

    let response = app
        .oneshot(atompub_xml(
            method,
            // A well-formed hash so the typed extractor passes and the wrong-user
            // check (alice authenticated, bob's namespace) is what yields 403.
            "/atompub/bob/media/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/pic.png",
            "alice",
            &session.token,
            None,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// A malformed `{sha}` or `{filename}` segment on the authenticated member routes is
// rejected by the typed `Path<(Username, ContentHash, Filename)>` extractor as a
// pre-handler 400 (the URL is one we minted, so a bad segment is the caller's fault) —
// distinct from a well-formed-but-absent resource, which is 404 above.
#[apply(backends)]
#[tokio::test]
async fn member_rejects_malformed_segment_returns_400(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state, "alice").await;
    let storage = TempDir::new().unwrap();
    let app = make_app(Arc::clone(&state), &storage);

    // Malformed hash segment (`deadbeef` is not 64 hex) → ContentHash parse fails → 400.
    let bad_hash = app
        .clone()
        .oneshot(atompub_xml(
            "GET",
            "/atompub/alice/media/deadbeef/pic.png",
            "alice",
            &session.token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(bad_hash.status(), StatusCode::BAD_REQUEST);

    // Non-canonical filename segment (`a%5Cb.png` decodes to `a\b.png`, not a safe leaf)
    // → Filename parse fails → 400.
    let bad_name = app
        .oneshot(atompub_xml(
            "GET",
            "/atompub/alice/media/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/a%5Cb.png",
            "alice",
            &session.token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(bad_name.status(), StatusCode::BAD_REQUEST);
}
