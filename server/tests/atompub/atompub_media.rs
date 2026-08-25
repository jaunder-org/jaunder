use axum::{
    body::Body,
    http::{HeaderValue, Method, StatusCode, header},
};
use tempfile::TempDir;
use tower::ServiceExt;

use common::root_relative_url::RootRelativeUrl;
use common::test_support::{
    parse_content_hash, parse_filename, parse_post_body, parse_root_relative_url,
};
use rstest::*;
use rstest_reuse::*;

use storage::test_support::{Backend, SeedRawPost, TestEnv, backends, backends_matrix};

use crate::helpers::{
    atompub, atompub_at, atompub_get, atompub_location, atompub_upload, body_string,
    create_user_and_session, make_app, setup_with_base_url,
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
    let session = create_user_and_session(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
async fn upload_accepts_pdf_content_type(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
async fn upload_without_content_type_defaults_to_octet_stream(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

    let response = app
        .oneshot(
            atompub(&session, Method::POST, "media")
                .header("slug", "upload.bin")
                .body(Body::from(PNG))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(
        body_string(response)
            .await
            .contains("type=\"application/octet-stream\"")
    );
}

#[apply(backends)]
#[tokio::test]
async fn upload_rejects_invalid_present_content_type(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
async fn get_unknown_media_returns_404(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
async fn delete_media_member_refuses_rowless_referenced_file(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

    let loc = upload_and_member_url(&app, &session, "pic.png").await;
    let sha256 = loc
        .as_ref()
        .rsplit('/')
        .nth(1)
        .map(parse_content_hash)
        .expect("member URL includes the content hash");
    let filename = parse_filename("pic.png");
    let media_url =
        common::media::media_url(&common::media::MediaSource::Upload, &sha256, &filename);
    SeedRawPost::new(session.user_id)
        .body(parse_post_body(&format!("![referenced]({media_url})")))
        .seed(&state)
        .await;

    let del_resp = app
        .oneshot(
            atompub_at(&session, Method::DELETE, &loc)
                .body(Body::empty())
                .expect("failed to build atompub DELETE request"),
        )
        .await
        .unwrap();

    assert_eq!(del_resp.status(), StatusCode::CONFLICT);
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
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
    let TestEnv { state, base: _base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;

    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);
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
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);
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
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let storage = TempDir::new().unwrap();
    let app = make_app(&state, &storage);

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
