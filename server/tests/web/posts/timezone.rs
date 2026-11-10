use crate::helpers::{
    SeededSession, confirmed_created_post, create_user_and_session, make_app, post_json,
};
use axum::http::StatusCode;
use common::{ids::PostId, visibility::ViewerIdentity};
use rstest::*;
use rstest_reuse::*;
use server_fn::ServerFn;
use storage::{
    PostRecord,
    test_support::{Backend, TestEnv, backends, backends_matrix},
};

async fn read_post(env: &TestEnv, session: &SeededSession, id: PostId) -> PostRecord {
    env.posts()
        .get_post_by_id(id, &ViewerIdentity::local(session.user_id))
        .await
        .unwrap()
        .unwrap()
}

#[apply(backends)]
#[tokio::test]
async fn publication_timezone_web_org_transport_over_header_preserves_instant(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let cookie = session.cookie();
    let app = make_app!(&env, &env.base);
    // DATE still uses its header zone; transport only selects the saved zone.
    let source = "#+DATE: [2020-11-01 Sun 01:30]\n#+PROPERTY: JAUNDER_DATE_TZ US/Eastern\n#+PROPERTY: JAUNDER_STATUS published\n#+PROPERTY: JAUNDER_DATE_UTC 2020-11-01T05:30:00Z\n\nSame body";
    for (transport, expected) in [
        (Some("Europe/London"), "Europe/London"),
        (None, "US/Eastern"),
    ] {
        let mut post = serde_json::json!({"body": source, "format": "org"});
        if let Some(zone) = transport {
            post["timezone"] = zone.into();
        }
        let (status, body) = post_json(
            app.clone(),
            <web::posts::Create as ServerFn>::PATH,
            serde_json::json!({"post": post}),
            Some(&cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let record = read_post(&env, &session, confirmed_created_post(&body).post_id).await;
        assert_eq!(record.timezone.as_ref(), expected);
        assert_eq!(
            record.published_at.unwrap().to_string(),
            "2020-11-01T05:30:00Z"
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn publication_timezone_web_old_client_omission_and_zone_only_revisions(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let cookie = session.cookie();
    let app = make_app!(&env, &env.base);
    let (status, body) = post_json(app.clone(), <web::posts::Create as ServerFn>::PATH,
        serde_json::json!({"post": {"body": "Legacy body", "format": "markdown", "publish": false}}), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = confirmed_created_post(&body).post_id;
    assert_eq!(read_post(&env, &session, id).await.timezone.as_ref(), "UTC");
    for (zone, expected) in [
        (Some("US/Eastern"), "US/Eastern"),
        (None, "US/Eastern"),
        (Some("Europe/London"), "Europe/London"),
    ] {
        let mut post =
            serde_json::json!({"body": "Legacy body", "format": "markdown", "publish": false});
        if let Some(zone) = zone {
            post["timezone"] = zone.into();
        }
        let (status, body) = post_json(
            app.clone(),
            <web::posts::Update as ServerFn>::PATH,
            serde_json::json!({"post_id": id, "post": post}),
            Some(&cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            read_post(&env, &session, id).await.timezone.as_ref(),
            expected
        );
    }
    let page = env
        .posts()
        .list_post_revision_history(session.user_id, id, None, "10".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        page.revisions.len(),
        2,
        "omission is a no-op, each zone-only edit changes state"
    );
    let old = env
        .posts()
        .get_post_revision_detail(session.user_id, id, page.revisions[0].revision_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.revision.timezone.as_ref(), "US/Eastern");
    assert!(read_post(&env, &session, id).await.published_at.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn publication_timezone_web_raw_numeric_rejection_and_genuine_omission(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let cookie = session.cookie();
    let app = make_app!(&env, &env.base);
    let (status, body) = post_json(app.clone(), <web::posts::Create as ServerFn>::PATH,
        serde_json::json!({"post": {"body": "Legacy body", "format": "org", "publish": false, "timezone": "Europe/London"}}), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = confirmed_created_post(&body).post_id;
    let before = read_post(&env, &session, id).await;
    let numeric = "#+DATE: [2020-11-01 Sun 01:30]\n#+PROPERTY: JAUNDER_DATE_TZ -0400\n#+PROPERTY: JAUNDER_STATUS published\n\nChanged body";
    let post = serde_json::json!({"body": numeric, "format": "org", "timezone": "Asia/Tokyo"});
    for (path, payload) in [
        (
            <web::posts::Create as ServerFn>::PATH,
            serde_json::json!({"post": post}),
        ),
        (
            <web::posts::Update as ServerFn>::PATH,
            serde_json::json!({"post_id": id, "post": post}),
        ),
    ] {
        let (status, body) = post_json(app.clone(), path, payload, Some(&cookie)).await;
        assert!(
            !status.is_success(),
            "numeric raw DATE remains invalid even with valid transport: {body}"
        );
    }
    let after = read_post(&env, &session, id).await;
    assert_eq!(after.timezone, before.timezone);
    assert_eq!(after.body, before.body);
    assert_eq!(after.published_at, before.published_at);
    assert_eq!(after.updated_at, before.updated_at);
    let omitted = serde_json::json!({"body": "Legacy body", "format": "org", "publish": false});
    let (status, body) = post_json(
        app.clone(),
        <web::posts::Update as ServerFn>::PATH,
        serde_json::json!({"post_id": id, "post": omitted}),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        read_post(&env, &session, id).await.timezone.as_ref(),
        "Europe/London"
    );
    let (status, body) = post_json(
        app,
        <web::posts::Create as ServerFn>::PATH,
        serde_json::json!({"post": omitted}),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        read_post(&env, &session, confirmed_created_post(&body).post_id)
            .await
            .timezone
            .as_ref(),
        "UTC"
    );
}

#[apply(backends_matrix)]
#[case::create(false)]
#[case::update(true)]
#[tokio::test]
async fn publication_timezone_web_rejects_null_or_non_string_without_mutation(
    backend: Backend,
    #[case] update: bool,
) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let cookie = session.cookie();
    let app = make_app!(&env, &env.base);
    let (status, body) = post_json(app.clone(), <web::posts::Create as ServerFn>::PATH,
        serde_json::json!({"post": {"body": "Original body", "format": "markdown", "publish": false, "timezone": "US/Eastern"}}), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = confirmed_created_post(&body).post_id;
    let before = read_post(&env, &session, id).await;
    for timezone in [
        serde_json::Value::Null,
        serde_json::json!(true),
        serde_json::json!(42),
        serde_json::json!([]),
        serde_json::json!({}),
    ] {
        let post = serde_json::json!({"body": "Changed body", "format": "markdown", "publish": false, "timezone": timezone});
        let (path, payload) = if update {
            (
                <web::posts::Update as ServerFn>::PATH,
                serde_json::json!({"post_id": id, "post": post}),
            )
        } else {
            (
                <web::posts::Create as ServerFn>::PATH,
                serde_json::json!({"post": post}),
            )
        };
        let (status, body) = post_json(app.clone(), path, payload, Some(&cookie)).await;
        assert!(
            !status.is_success(),
            "present {timezone} must reject: {body}"
        );
    }
    let after = read_post(&env, &session, id).await;
    assert_eq!(after.timezone, before.timezone);
    assert_eq!(after.body, before.body);
    assert_eq!(after.published_at, before.published_at);
    assert_eq!(after.updated_at, before.updated_at);
    let page = env
        .posts()
        .list_post_revision_history(session.user_id, id, None, "10".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(page.revisions.is_empty());
    let (status, body) = super::fixtures::list_drafts(app, None, 20, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let drafts: common::seed::Page<web::posts::UnpublishedPost> =
        serde_json::from_str(&body).unwrap();
    assert_eq!(
        drafts.posts.len(),
        1,
        "rejected creates must not insert Posts"
    );
    assert_eq!(drafts.posts[0].post.post_id, id);
}

#[apply(backends)]
#[tokio::test]
async fn publication_timezone_web_rejects_present_invalid_zone(#[case] backend: Backend) {
    let env = backend.setup().await;
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let cookie = session.cookie();
    let app = make_app!(&env, &env.base);
    let (status, body) = post_json(app.clone(), <web::posts::Create as ServerFn>::PATH,
        serde_json::json!({"post": {"body": "Same body", "format": "markdown", "publish": false, "timezone": "US/Eastern"}}), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = confirmed_created_post(&body).post_id;
    for zone in [
        "",
        " ",
        " UTC",
        "UTC ",
        "+05:00",
        "Etc/Unknown",
        "No/Such_Zone",
    ] {
        let post = serde_json::json!({"body": "Changed body", "format": "markdown", "publish": false, "timezone": zone});
        for (path, payload) in [
            (
                <web::posts::Create as ServerFn>::PATH,
                serde_json::json!({"post": post}),
            ),
            (
                <web::posts::Update as ServerFn>::PATH,
                serde_json::json!({"post_id": id, "post": post}),
            ),
        ] {
            let (status, body) = post_json(app.clone(), path, payload, Some(&cookie)).await;
            assert!(!status.is_success(), "{zone:?}: {status} {body}");
        }
    }
    let record = read_post(&env, &session, id).await;
    assert_eq!(record.timezone.as_ref(), "US/Eastern");
    assert_eq!(record.body.as_ref(), "Same body\n");
}
