use super::fixtures::{etag_of, location_post_id};
use crate::helpers::{
    SeededSession, atompub, atompub_get, atompub_post_xml, atompub_put_xml, body_string,
    create_user_and_session, make_app,
};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
    response::Response,
};
use common::visibility::ViewerIdentity;
use rstest::*;
use rstest_reuse::*;
use storage::test_support::{Backend, backends};
use tower::ServiceExt;

async fn send(app: &Router, request: Request<Body>) -> Response {
    app.clone().oneshot(request).await.unwrap()
}

fn entry(zone: &str) -> String {
    format!(
        r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:device="https://jaunder.org/ns/atompub"><title>Zone</title><content type="text/markdown">Same body</content><published>2020-11-01T06:30:00.123456Z</published>{zone}</entry>"#
    )
}

fn org_entry(source: &str, published: &str, transport: &str) -> String {
    format!(
        r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:device="https://jaunder.org/ns/atompub"><title>Org zone</title><content type="text/org">{source}</content>{published}{transport}</entry>"#
    )
}

fn conditional_put(
    session: &SeededSession,
    member: &str,
    xml: String,
    etag: &str,
) -> Request<Body> {
    atompub(session, Method::PUT, member)
        .header(header::IF_MATCH, etag)
        .header(header::CONTENT_TYPE, "application/atom+xml")
        .body(Body::from(xml))
        .unwrap()
}

async fn assert_zone_and_instant(response: Response, zone: &str, instant: &str) {
    let xml = body_string(response).await;
    assert!(xml.contains(&format!(">{zone}</")), "{xml}");
    let entry: host::atompub::Entry = xml.parse().unwrap();
    let at: common::time::UtcInstant = entry.published().unwrap().to_rfc3339().parse().unwrap();
    assert_eq!(at.to_string(), instant);
}

#[apply(backends)]
#[tokio::test]
async fn publication_timezone_atom_validation_omission_and_expanded_name(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let created = send(
        &app,
        atompub_post_xml(
            &session,
            "posts",
            &entry("<timezone xmlns=\"https://jaunder.org/ns/atompub\">US/Eastern</timezone>"),
        ),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let member = format!("posts/{}", location_post_id(&created));
    let initial = etag_of(&created);
    assert!(body_string(created).await.contains(">US/Eastern</"));
    for value in [
        "",
        " ",
        " UTC",
        "UTC ",
        "+05:00",
        "Etc/Unknown",
        "No/Such_Zone",
    ] {
        let xml = entry(&format!("<device:timezone>{value}</device:timezone>"));
        for request in [
            atompub_post_xml(&session, "posts", &xml),
            atompub_put_xml(&session, &member, &xml),
        ] {
            assert_eq!(
                send(&app, request).await.status(),
                StatusCode::BAD_REQUEST,
                "{value:?}"
            );
        }
    }
    for marker in [
        "<device:timezone>UTC</device:timezone><device:timezone>Europe/London</device:timezone>",
        "<device:timezone><device:nested>UTC</device:nested></device:timezone>",
    ] {
        assert_eq!(
            send(&app, atompub_put_xml(&session, &member, &entry(marker)))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let wrong_namespace =
        entry("<timezone xmlns=\"https://example.com/not-jaunder\">Europe/London</timezone>");
    let response = send(&app, atompub_put_xml(&session, &member, &wrong_namespace)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(etag_of(&response), initial);
    assert!(body_string(response).await.contains(">US/Eastern</"));
    let response = send(&app, atompub_post_xml(&session, "posts", &entry(""))).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(body_string(response).await.contains(">UTC</"));
}

#[apply(backends)]
#[tokio::test]
async fn publication_timezone_atom_org_sources_keep_publication_assertions(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let source = "#+DATE: [2020-11-01 Sun 01:30]\n#+PROPERTY: JAUNDER_DATE_TZ US/Eastern\n#+PROPERTY: JAUNDER_STATUS published\n\nSame body";
    for (transport, published, marker, expected_zone, expected_instant) in [
        ("", "", "", "US/Eastern", "2020-11-01T05:30:00Z"),
        (
            "<device:timezone>Europe/London</device:timezone>",
            "<published>2020-11-01T06:30:00.123456Z</published>",
            "#+PROPERTY: JAUNDER_DATE_UTC 2020-11-01T06:30:00.123456Z\n",
            "Europe/London",
            "2020-11-01T06:30:00.123456Z",
        ),
    ] {
        let xml = org_entry(&format!("{marker}{source}"), published, transport);
        let response = send(&app, atompub_post_xml(&session, "posts", &xml)).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let member = format!("posts/{}", location_post_id(&response));
        assert_zone_and_instant(response, expected_zone, expected_instant).await;
        let omitted = org_entry(
            "#+PROPERTY: JAUNDER_STATUS draft\n\nSame body",
            &format!("<published>{expected_instant}</published>"),
            "",
        );
        let response = send(&app, atompub_put_xml(&session, &member, &omitted)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_zone_and_instant(response, expected_zone, expected_instant).await;
        let response = send(&app, atompub_post_xml(&session, "posts", &omitted)).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_zone_and_instant(response, "UTC", expected_instant).await;
    }
    let inconsistent = org_entry(
        &format!("#+PROPERTY: JAUNDER_DATE_UTC 2020-11-01T05:30:00Z\n{source}"),
        "<published>2020-11-01T06:30:00Z</published>",
        "<device:timezone>Europe/London</device:timezone>",
    );
    assert_eq!(
        send(&app, atompub_post_xml(&session, "posts", &inconsistent))
            .await
            .status(),
        StatusCode::BAD_REQUEST,
        "DATE_UTC asserts the final structured instant"
    );
}

#[apply(backends)]
#[tokio::test]
async fn publication_timezone_atom_numeric_header_still_rejects_before_precedence(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let created = send(
        &app,
        atompub_post_xml(
            &session,
            "posts",
            &entry("<device:timezone>Europe/London</device:timezone>"),
        ),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let member = format!("posts/{}", location_post_id(&created));
    let before = etag_of(&created);
    let before_xml = body_string(created).await;
    let source = "#+DATE: [2020-11-01 Sun 01:30]\n#+PROPERTY: JAUNDER_DATE_TZ -0400\n#+PROPERTY: JAUNDER_STATUS published\n\nChanged body";
    for published in ["", "<published>2020-11-01T06:30:00.123456Z</published>"] {
        let xml = org_entry(
            source,
            published,
            "<device:timezone>Asia/Tokyo</device:timezone>",
        );
        for request in [
            atompub_post_xml(&session, "posts", &xml),
            atompub_put_xml(&session, &member, &xml),
        ] {
            assert_eq!(send(&app, request).await.status(), StatusCode::BAD_REQUEST);
        }
    }
    let read = send(&app, atompub_get(&session, &member)).await;
    assert_eq!(etag_of(&read), before);
    assert_eq!(
        body_string(read).await,
        before_xml,
        "rejected writes leave the complete member unchanged"
    );
}

#[apply(backends)]
#[tokio::test]
async fn publication_timezone_atom_round_trip_and_zone_only_validator(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let app = make_app!(&env, &env.base);
    let created = send(
        &app,
        atompub_post_xml(
            &session,
            "posts",
            &entry("<device:timezone>US/Eastern</device:timezone>"),
        ),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let id = location_post_id(&created);
    let member = format!("posts/{id}");
    let old_etag = etag_of(&created);
    assert_zone_and_instant(created, "US/Eastern", "2020-11-01T06:30:00.123456Z").await;
    let read = send(&app, atompub_get(&session, &member)).await;
    assert_eq!(etag_of(&read), old_etag);
    assert_zone_and_instant(read, "US/Eastern", "2020-11-01T06:30:00.123456Z").await;
    let updated = send(
        &app,
        conditional_put(
            &session,
            &member,
            entry("<device:timezone>Europe/London</device:timezone>"),
            &old_etag,
        ),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let new_etag = etag_of(&updated);
    assert_ne!(
        new_etag, old_etag,
        "zone-only edits invalidate conditional writes"
    );
    assert_zone_and_instant(updated, "Europe/London", "2020-11-01T06:30:00.123456Z").await;
    let read = send(&app, atompub_get(&session, &member)).await;
    assert_eq!(etag_of(&read), new_etag);
    assert_zone_and_instant(read, "Europe/London", "2020-11-01T06:30:00.123456Z").await;
    let current = env
        .posts()
        .get_post_by_id(id.into(), &ViewerIdentity::local(session.user_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.timezone.as_ref(), "Europe/London");
    let page = env
        .posts()
        .list_post_revision_history(session.user_id, id.into(), None, "10".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.revisions.len(), 1);
    let detail = env
        .posts()
        .get_post_revision_detail(session.user_id, id.into(), page.revisions[0].revision_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.revision.timezone.as_ref(), "US/Eastern");
    let stale = send(
        &app,
        conditional_put(
            &session,
            &member,
            entry("<device:timezone>Asia/Tokyo</device:timezone>"),
            &old_etag,
        ),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
    let omitted = send(&app, atompub_put_xml(&session, &member, &entry(""))).await;
    assert_eq!(omitted.status(), StatusCode::OK);
    assert_eq!(etag_of(&omitted), new_etag);
    assert_zone_and_instant(omitted, "Europe/London", "2020-11-01T06:30:00.123456Z").await;
    let page = env
        .posts()
        .list_post_revision_history(session.user_id, id.into(), None, "10".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.revisions.len(), 1, "true no-op creates no revision");
}
