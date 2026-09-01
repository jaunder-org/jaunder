use axum::{
    body::Body,
    http::{Method, StatusCode, header},
};
use common::ids::PostId;
use common::test_support::permalink_date;
use rstest::*;
use rstest_reuse::*;
use tower::ServiceExt;

use crate::helpers::{
    atompub_at, atompub_get, atompub_location, atompub_post_xml, atompub_put_xml, body_string,
    create_user_and_session, make_app, setup_with_base_url,
};
use storage::test_support::{Backend, TestEnv, backends};

use super::fixtures::location_post_id;

/// A non-draft text entry carrying an optional `<published>` element (RFC 3339).
/// `published == None` omits the element entirely (publish-now semantics).
fn entry_xml_with_published(title: &str, content: &str, published: Option<&str>) -> String {
    let published_elem =
        published.map_or_else(String::new, |ts| format!("\n  <published>{ts}</published>"));
    format!(
        r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>{title}</title>
  <content type="text">{content}</content>{published_elem}
</entry>"#
    )
}

/// A non-draft text entry whose explicit Atom lifecycle marker preserves its
/// supplied `<published>` instant.
fn entry_xml_with_draft_no_and_published(title: &str, content: &str, published: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>{title}</title>
  <content type="text">{content}</content>
  <published>{published}</published>
  <app:control><app:draft>no</app:draft></app:control>
</entry>"#
    )
}

#[apply(backends)]
#[tokio::test]
async fn create_draft_entry_is_unpublished(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>Draft</title>
  <content type="text">draft body</content>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#;

    let response = app
        .clone()
        .oneshot(atompub_post_xml(&session, "posts", xml))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let location = atompub_location(
        response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    );

    let get = app
        .oneshot(
            atompub_at(&session, Method::GET, &location)
                .body(Body::empty())
                .expect("failed to build atompub GET request"),
        )
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let body = body_string(get).await;
    // A draft post round-trips the app:draft marker.
    assert!(body.contains("app:draft"), "draft marker missing: {body}");
    // The read-only j:slug is emitted on every entry, drafts included (ADR-0023).
    assert!(
        body.contains("xmlns:j=\"https://jaunder.org/ns/atompub\""),
        "draft entry should declare xmlns:j: {body}"
    );
    assert!(
        body.contains("<j:slug>"),
        "draft entry should carry j:slug: {body}"
    );
}

/// Atom's explicit `app:draft` is a structured lifecycle scalar: `no` publishes
/// now and prevents an Org `JAUNDER_STATUS` header from supplying the lifecycle.
/// Other structured Atom fields still outrank their Org-header counterparts, and
/// accepted recognized headers are stripped from the native Org readback.
#[apply(backends)]
#[tokio::test]
async fn explicit_atom_draft_no_beats_org_metadata_and_canonicalizes_org(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>Atom title</title>
  <summary>Atom summary</summary>
  <content type="text/org">#+TITLE: Header title
#+DESCRIPTION: Header summary
#+KEYWORDS: header-tag
#+PROPERTY: JAUNDER_STATUS draft
#+UNKNOWN: retained

Org body</content>
  <category term="atom-tag"/>
  <app:control><app:draft>no</app:draft></app:control>
</entry>"#;

    let response = make_app(&state, &base)
        .oneshot(atompub_post_xml(&session, "posts", xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let location = atompub_location(
        response
            .headers()
            .get(header::LOCATION)
            .expect("create response has Location")
            .to_str()
            .expect("Location is text"),
    );
    let response = make_app(&state, &base)
        .oneshot(
            atompub_at(&session, Method::GET, &location)
                .body(Body::empty())
                .expect("build member GET"),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;
    assert!(body.contains("<title>Atom title</title>"), "body: {body}");
    assert!(
        body.contains("<summary>Atom summary</summary>"),
        "body: {body}"
    );
    assert!(body.contains("term=\"atom-tag\""), "body: {body}");
    assert!(body.contains("type=\"text/org\""), "body: {body}");
    assert!(body.contains("<published>"), "body: {body}");
    assert!(!body.contains("app:draft"), "body: {body}");
    assert!(body.contains("#+UNKNOWN: retained"), "body: {body}");
    assert!(body.contains("Org body"), "body: {body}");
    assert!(!body.contains("JAUNDER_STATUS"), "body: {body}");
    assert!(!body.contains("#+TITLE:"), "body: {body}");
    assert!(!body.contains("#+DESCRIPTION:"), "body: {body}");
    assert!(!body.contains("#+KEYWORDS:"), "body: {body}");
}

/// Create bookkeeping is checked against the finalized Org record, including
/// collision-free slug, selected format, and the supplied publication instant.
#[apply(backends)]
#[tokio::test]
async fn create_org_bookkeeping_must_match_final_values(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let publication = "2024-01-02T03:04:05Z";
    let valid = format!(
        r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Final Title</title>
  <content type="text/org">#+PROPERTY: JAUNDER_SLUG final-title
#+PROPERTY: JAUNDER_FORMAT org
#+PROPERTY: JAUNDER_DATE_UTC 2024-01-02T03:04:05+00:00

Body</content>
  <published>{publication}</published>
</entry>"#
    );
    let response = make_app(&state, &base)
        .oneshot(atompub_post_xml(&session, "posts", &valid))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    for metadata in [
        "#+PROPERTY: JAUNDER_SLUG wrong-slug",
        "#+PROPERTY: JAUNDER_FORMAT markdown",
        "#+PROPERTY: JAUNDER_DATE_UTC 2024-01-02T03:04:06Z",
    ] {
        let xml = format!(
            r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Different Title</title>
  <content type="text/org">{metadata}

Body</content>
  <published>{publication}</published>
</entry>"#
        );
        let response = make_app(&state, &base)
            .oneshot(atompub_post_xml(&session, "posts", &xml))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "bookkeeping {metadata:?} must match final post"
        );
    }
    let response = make_app(&state, &base)
        .oneshot(atompub_get(&session, "posts"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body_string(response).await.matches("<entry").count(),
        1,
        "rejected bookkeeping must not create posts"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_with_future_published_is_scheduled(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    // A non-draft entry whose <published> is in the far future schedules the post.
    let xml = entry_xml_with_published("Future post", "body", Some("2099-01-01T00:00:00Z"));
    let response = app
        .oneshot(atompub_post_xml(&session, "posts", &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = location_post_id(&response);

    // The owner may inspect the scheduled private post's persisted timestamp.
    let owner = common::visibility::ViewerIdentity::local(session.user_id);
    let rec = state
        .posts
        .get_post_by_id(PostId::from(post_id), &owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rec.published_at.unwrap().value().to_rfc3339(),
        "2099-01-01T00:00:00+00:00"
    );

    let viewer = common::visibility::ViewerIdentity::Anonymous;

    // ...and it is invisible on the public permalink at "now".
    let public = state
        .posts
        .get_post_by_permalink(
            &session.username,
            permalink_date(2099, 1, 1),
            &rec.slug,
            &viewer,
            common::time::UtcInstant::now(),
        )
        .await
        .unwrap();
    assert!(
        public.is_none(),
        "future-published AtomPub post must be hidden until due"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_with_past_published_is_live_backdated(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);

    // A non-draft entry whose <published> is in the past is live, backdated.
    let xml = entry_xml_with_published("Old post", "body", Some("2000-01-01T00:00:00Z"));
    let response = app
        .oneshot(atompub_post_xml(&session, "posts", &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = location_post_id(&response);

    let viewer = common::visibility::ViewerIdentity::local(session.user_id);
    let rec = state
        .posts
        .get_post_by_id(PostId::from(post_id), &viewer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rec.published_at.unwrap().value().to_rfc3339(),
        "2000-01-01T00:00:00+00:00"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_with_explicit_draft_no_preserves_published_instant(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let app = make_app(&state, &base);
    let xml = entry_xml_with_draft_no_and_published("Old post", "body", "2000-01-01T00:00:00Z");

    let response = app
        .oneshot(atompub_post_xml(&session, "posts", &xml))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let post_id = location_post_id(&response);
    let owner = common::visibility::ViewerIdentity::local(session.user_id);
    let rec = state
        .posts
        .get_post_by_id(PostId::from(post_id), &owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rec.published_at.unwrap().value().to_rfc3339(),
        "2000-01-01T00:00:00+00:00"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_with_future_published_schedules_post(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;

    // Start from a live post, then PUT a non-draft entry with a future
    // <published>: it must become scheduled (future published_at, hidden).
    let post = session.seed_post().seed(&state).await;

    let app = make_app(&state, &base);

    let xml = entry_xml_with_published("Rescheduled", "new body", Some("2099-06-01T00:00:00Z"));
    let response = app
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{}", post.post_id),
            &xml,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let viewer = common::visibility::ViewerIdentity::Anonymous;
    let rec = state
        .posts
        .get_post_by_id(post.post_id, &viewer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rec.published_at.unwrap().value().to_rfc3339(),
        "2099-06-01T00:00:00+00:00",
        "update must honor the wire <published> timestamp"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_with_explicit_draft_no_preserves_published_instant(#[case] backend: Backend) {
    let TestEnv { state, base } = setup_with_base_url(backend).await;
    let session = create_user_and_session(&state).await;
    let post = session.seed_post().seed(&state).await;
    let app = make_app(&state, &base);
    let xml =
        entry_xml_with_draft_no_and_published("Backdated", "new body", "2000-01-01T00:00:00Z");

    let response = app
        .oneshot(atompub_put_xml(
            &session,
            &format!("posts/{}", post.post_id),
            &xml,
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let viewer = common::visibility::ViewerIdentity::Anonymous;
    let rec = state
        .posts
        .get_post_by_id(post.post_id, &viewer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rec.published_at.unwrap().value().to_rfc3339(),
        "2000-01-01T00:00:00+00:00"
    );
}
