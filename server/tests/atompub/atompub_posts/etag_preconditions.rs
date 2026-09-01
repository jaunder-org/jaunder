use axum::{
    body::Body,
    http::{Method, StatusCode, header},
};
use common::root_relative_url::RootRelativeUrl;
use common::test_support::parse_post_body;
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{
    SeededSession, atompub, atompub_at, atompub_get, atompub_location, atompub_post_xml,
    atompub_put_xml, body_string, create_user_and_session, make_app, setup_with_base_url,
};
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};

use super::fixtures::{entry_xml, etag_of};

#[apply(backends)]
#[tokio::test]
async fn update_with_stale_if_match_returns_412(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    let post = session.seed_post().seed(&state).await;

    let app = make_app(&state, &base);

    let xml = entry_xml("New", "text", "new body");
    let response = app
        .oneshot(
            atompub(&session, Method::PUT, &format!("posts/{}", post.post_id))
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, "\"0\"") // Wrong ETag
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
}

/// A stale Org `JAUNDER_SYNCED` is an independent `AtomPub` precondition: even a
/// matching HTTP `If-Match` cannot make it apply, and the stored member remains
/// unchanged.
#[apply(backends)]
#[tokio::test]
async fn stale_org_synced_returns_412_despite_matching_if_match_without_mutation(
    #[case] backend: Backend,
) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let post = session
        .seed_post()
        .body(parse_post_body("Original body"))
        .seed(&state)
        .await;
    let member = format!("posts/{}", post.post_id);
    let initial = make_app(&state, &base)
        .oneshot(atompub_get(&session, &member))
        .await
        .unwrap();
    assert_eq!(initial.status(), StatusCode::OK);
    let current_etag = etag_of(&initial);
    let xml = entry_xml(
        "Replacement",
        "text/org",
        &format!(
            "#+PROPERTY: JAUNDER_ID {}\n#+PROPERTY: JAUNDER_SYNCED \"stale\"\n\nReplacement body",
            post.post_id
        ),
    );

    let response = make_app(&state, &base)
        .oneshot(
            atompub(&session, Method::PUT, &member)
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, current_etag)
                .body(Body::from(xml))
                .expect("build matching If-Match PUT"),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
    let response = make_app(&state, &base)
        .oneshot(atompub_get(&session, &member))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("Original body"), "body: {body}");
    assert!(!body.contains("Replacement body"), "body: {body}");
}

/// Matching update bookkeeping uses the target ID and the pre-write content
/// `ETag`; it is accepted independently of the optional HTTP conditional header.
#[apply(backends)]
#[tokio::test]
async fn matching_org_id_and_synced_bookkeeping_updates_member(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let post = session.seed_post().seed(&state).await;
    let member = format!("posts/{}", post.post_id);
    let initial = make_app(&state, &base)
        .oneshot(atompub_get(&session, &member))
        .await
        .unwrap();
    assert_eq!(initial.status(), StatusCode::OK);
    let etag = etag_of(&initial);
    let xml = entry_xml(
        "Updated title",
        "text/org",
        &format!(
            "#+PROPERTY: JAUNDER_ID {}\n#+PROPERTY: JAUNDER_SYNCED {etag}\n#+PROPERTY: JAUNDER_SLUG {}\n#+PROPERTY: JAUNDER_FORMAT org\n#+PROPERTY: JAUNDER_STATUS draft\n\nUpdated body",
            post.post_id, post.slug
        ),
    );

    let response = make_app(&state, &base)
        .oneshot(atompub_put_xml(&session, &member, &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("Updated body"), "body: {body}");
    assert!(body.contains(post.slug.as_ref()), "body: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_with_matching_if_match_succeeds(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <content type="text">body</content>
</entry>"#;

    let created = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", xml))
        .await
        .unwrap();
    let location = atompub_location(
        created
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );
    let etag = created
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    // A matching If-Match passes the precondition and the update proceeds.
    let updated = app
        .oneshot(
            atompub_at(&session, Method::PUT, &location)
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, etag)
                .body(Body::from(xml))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
}

const ETAG_POST_XML: &str = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <category term="rust"/>
  <content type="text">body</content>
</entry>"#;

/// POST `ETAG_POST_XML` as alice; return the create response's (`Location`, `ETag`).
async fn create_location_etag(
    app: axum::Router,
    session: &SeededSession,
) -> (RootRelativeUrl, String) {
    let created = app
        .oneshot(atompub_post_xml(session, "posts", ETAG_POST_XML))
        .await
        .unwrap();
    let location = atompub_location(
        created
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );
    let etag = created
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    (location, etag)
}

/// POST `ETAG_POST_XML` as alice and return the create response's `ETag`.
async fn create_etag(app: axum::Router, session: &SeededSession) -> String {
    create_location_etag(app, session).await.1
}

/// GET `location` as alice, returning the response status.
async fn get_status(
    app: axum::Router,
    session: &SeededSession,
    location: &RootRelativeUrl,
) -> StatusCode {
    app.oneshot(
        atompub_at(session, Method::GET, location)
            .body(Body::empty())
            .expect("failed to build atompub GET request"),
    )
    .await
    .unwrap()
    .status()
}

/// How a DELETE request carries (or omits) `If-Match`. `MatchingEtag` uses the
/// post's current `ETag` (captured at creation); the literals model a stale
/// precondition (`"0"`) and the `*` wildcard.
enum DeleteIfMatch {
    Absent,
    Literal(&'static str),
    MatchingEtag,
}

// AC7: `If-Match` on DELETE. A stale precondition is refused (412) and the post
// survives; a matching ETag, an absent header, and the `*` wildcard each delete
// unconditionally (204) and the post is then gone. Uses `backends_matrix` so each
// precondition case runs on both backends.
#[apply(backends_matrix)]
#[case::stale(DeleteIfMatch::Literal("\"0\""), StatusCode::PRECONDITION_FAILED, true)]
#[case::matching(DeleteIfMatch::MatchingEtag, StatusCode::NO_CONTENT, false)]
#[case::absent(DeleteIfMatch::Absent, StatusCode::NO_CONTENT, false)]
#[case::wildcard(DeleteIfMatch::Literal("*"), StatusCode::NO_CONTENT, false)]
#[tokio::test]
async fn delete_if_match_precondition(
    backend: Backend,
    #[case] if_match: DeleteIfMatch,
    #[case] expected_status: StatusCode,
    #[case] post_survives: bool,
) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let (location, etag) = create_location_etag(app.clone(), &session).await;

    let builder = atompub_at(&session, Method::DELETE, &location);
    let builder = match if_match {
        DeleteIfMatch::Absent => builder,
        DeleteIfMatch::Literal(value) => builder.header(header::IF_MATCH, value),
        DeleteIfMatch::MatchingEtag => builder.header(header::IF_MATCH, etag),
    };
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), expected_status);

    let expected_after = if post_survives {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    };
    assert_eq!(get_status(app, &session, &location).await, expected_after);
}

#[apply(backends)]
#[tokio::test]
async fn editing_content_via_put_changes_etag(#[case] backend: Backend) {
    // AC4 (HTTP): a PUT that changes the body changes the ETag end-to-end.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let (location, e1) = create_location_etag(app.clone(), &session).await;

    let edited = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <category term="rust"/>
  <content type="text">a different body</content>
</entry>"#;
    let updated = app
        .oneshot(
            atompub_at(&session, Method::PUT, &location)
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, &e1)
                .body(Body::from(edited))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    let e2 = updated
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_ne!(e1, e2);
}

#[apply(backends)]
#[tokio::test]
async fn etag_is_content_hash_format(#[case] backend: Backend) {
    // AC1: the emitted ETag is a strong, quoted "sha256-<64 lowercase hex>" token.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let etag = create_etag(app, &session).await;
    let hex = etag
        .strip_prefix("\"sha256-")
        .and_then(|s| s.strip_suffix('"'))
        .expect("ETag is a quoted sha256- token");
    assert_eq!(hex.len(), 64);
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    );
}

#[apply(backends)]
#[tokio::test]
async fn identical_posts_share_etag(#[case] backend: Backend) {
    // AC2: two distinct posts with identical content get the same ETag — the
    // per-post id / tag ids / slug are excluded from the hash.
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let e1 = create_etag(app.clone(), &session).await;
    let e2 = create_etag(app, &session).await;
    assert_eq!(e1, e2);
}

#[apply(backends)]
#[tokio::test]
async fn idempotent_reput_keeps_etag(#[case] backend: Backend) {
    // AC3 + AC5: re-PUT byte-identical content → the ETag is unchanged (a
    // timestamp ETag would have bumped on the write).
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let created = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", ETAG_POST_XML))
        .await
        .unwrap();
    let location = atompub_location(
        created
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );
    let e1 = created
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let updated = app
        .oneshot(
            atompub_at(&session, Method::PUT, &location)
                .header(header::CONTENT_TYPE, "application/atom+xml")
                .header(header::IF_MATCH, &e1)
                .body(Body::from(ETAG_POST_XML))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    let e2 = updated
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(e1, e2);
}
