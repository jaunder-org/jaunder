use axum::{Router, http::StatusCode};
use common::ids::{PostId, RevisionId};
use common::revision_history::{RevisionHistoryAudience, RevisionHistoryDetail};
use rstest::*;
use rstest_reuse::*;
use server_fn::ServerFn;
use storage::test_support::{Backend, backends};
use web::posts::{PostInputs, PostRevisionHistory, RevisionHistoryPage};

use crate::helpers::{
    confirmed_created_post, create_post_json, create_user_and_session, make_app, post_json,
    set_public_default_audience, update_post_json,
};

async fn list_history(app: Router, cookie: Option<&str>) -> (StatusCode, String) {
    post_json(
        app,
        <web::posts::ListHistory as ServerFn>::PATH,
        serde_json::to_value(web::posts::ListHistory {
            cursor: None,
            limit: Some("1".parse().expect("valid page size")),
        })
        .expect("serialize history args"),
        cookie,
    )
    .await
}

async fn get_post_history(
    app: Router,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::posts::GetPostHistory as ServerFn>::PATH,
        serde_json::to_value(web::posts::GetPostHistory {
            post_id,
            cursor: None,
            limit: Some("1".parse().expect("valid page size")),
        })
        .expect("serialize post history args"),
        cookie,
    )
    .await
}

async fn get_revision_detail(
    app: Router,
    post_id: PostId,
    revision_id: RevisionId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::posts::GetRevisionHistoryDetail as ServerFn>::PATH,
        serde_json::to_value(web::posts::GetRevisionHistoryDetail {
            post_id,
            revision_id,
        })
        .expect("serialize revision detail args"),
        cookie,
    )
    .await
}

#[apply(backends)]
#[tokio::test]
async fn publication_timezone_history_and_scheduled_wire_projections(#[case] backend: Backend) {
    let env = backend.setup().await;
    let app = make_app!(&env, &env.base);
    let session = create_user_and_session(env.users(), env.sessions(), env.write_scope()).await;
    let cookie = session.cookie();
    let mut input = PostInputs::new(
        common::test_support::parse_post_body("Publication projection"),
        storage::PostFormat::Markdown,
    );
    input.publish = Some(true);
    input.publish_at = Some("2024-07-01T02:30:00.123456Z".parse().unwrap());
    input.timezone = Some("US/Eastern".parse().unwrap());
    let (status, body) = create_post_json(app.clone(), input.clone(), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = confirmed_created_post(&body).post_id;
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(created["Confirmed"]["post"]["timezone"], "US/Eastern");
    input.timezone = Some("Europe/London".parse().unwrap());
    let (status, body) = update_post_json(app.clone(), id, input.clone(), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let updated: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(updated["Confirmed"]["timezone"], "Europe/London");
    let (status, body) = get_post_history(app.clone(), id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let history: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(history["current"]["timezone"], "Europe/London");
    assert_eq!(
        history["current"]["published_at"],
        "2024-07-01T02:30:00.123456000Z"
    );
    let revision_id = history["revisions"]["revisions"][0]["revision_id"]
        .as_i64()
        .unwrap()
        .into();
    let (status, body) = get_revision_detail(app.clone(), id, revision_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let detail: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(detail["timezone"], "US/Eastern");
    assert_eq!(detail["published_at"], "2024-07-01T02:30:00.123456000Z");
    input.publish_at = Some("2037-07-01T02:30:00Z".parse().unwrap());
    let (status, body) = update_post_json(app.clone(), id, input, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = super::fixtures::list_scheduled(app, None, 20, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let scheduled: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(scheduled["posts"][0]["post"]["timezone"], "Europe/London");
}

#[apply(backends)]
#[tokio::test]
async fn revision_history_endpoints_hide_anonymous_access(#[case] backend: Backend) {
    let env = backend.setup().await;
    set_public_default_audience(env.site_config(), env.write_scope())
        .await
        .unwrap();
    let app = make_app!(&env, &env.base);

    let (status, body) = list_history(app.clone(), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
    let expected_body = body;

    let (status, body) = get_post_history(app.clone(), PostId::from(1), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(
        body, expected_body,
        "anonymous history responses must not disclose scope"
    );

    let (status, body) =
        get_revision_detail(app.clone(), PostId::from(1), RevisionId::from(1), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(
        body, expected_body,
        "anonymous history responses must not disclose scope"
    );
}

#[apply(backends)]
#[tokio::test]
async fn revision_history_http_exposes_page_current_and_detail_fields(#[case] backend: Backend) {
    let env = backend.setup().await;
    set_public_default_audience(env.site_config(), env.write_scope())
        .await
        .unwrap();
    let app = make_app!(&env, &env.base);
    let session = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let cookie = session.cookie();
    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(
                common::test_support::parse_post_body("# First\n\noriginal body"),
                storage::PostFormat::Markdown,
            )
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let post = confirmed_created_post(&body);
    let (status, body) = update_post_json(
        app.clone(),
        post.post_id,
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(
                common::test_support::parse_post_body("# Second\n\nupdated body"),
                storage::PostFormat::Markdown,
            )
        },
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let (status, body) = list_history(app.clone(), Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: RevisionHistoryPage = serde_json::from_str(&body).unwrap();
    assert_eq!(page.revisions.len(), 1);
    assert_eq!(page.revisions[0].post_id, post.post_id);
    assert!(page.next_cursor.is_none());
    assert!(!page.has_more);

    let (status, body) = get_post_history(app.clone(), post.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let history: PostRevisionHistory = serde_json::from_str(&body).unwrap();
    assert_eq!(history.current.post_id, post.post_id);
    assert_eq!(
        history.revisions.revisions[0].revision_id,
        page.revisions[0].revision_id
    );

    let (status, body) = get_revision_detail(
        app.clone(),
        post.post_id,
        page.revisions[0].revision_id,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let detail: RevisionHistoryDetail = serde_json::from_str(&body).unwrap();
    assert_eq!(detail.post_id, post.post_id);
    assert!(detail.body.as_ref().contains("original body"));
    assert!(detail.tags.is_empty());
    assert_eq!(
        detail.audiences,
        vec![RevisionHistoryAudience {
            kind: "public".to_owned(),
            audience_id: None,
        }]
    );
    assert!(detail.media.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn revision_history_http_hides_foreign_missing_and_mismatched_resources(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    set_public_default_audience(env.site_config(), env.write_scope())
        .await
        .unwrap();
    let app = make_app!(&env, &env.base);
    let owner = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await;
    let owner_cookie = owner.cookie();
    let stranger_cookie = create_user_and_session(
        std::sync::Arc::clone(&env.users()),
        std::sync::Arc::clone(&env.sessions()),
        env.write_scope(),
    )
    .await
    .cookie();
    let (status, body) = create_post_json(
        app.clone(),
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(
                common::test_support::parse_post_body("# Private\n\noriginal body"),
                storage::PostFormat::Markdown,
            )
        },
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let post = confirmed_created_post(&body);
    let (status, body) = update_post_json(
        app.clone(),
        post.post_id,
        PostInputs {
            publish: Some(false),
            ..PostInputs::new(
                common::test_support::parse_post_body("# Changed\n\nupdated body"),
                storage::PostFormat::Markdown,
            )
        },
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let (_, body) = list_history(app.clone(), Some(&owner_cookie)).await;
    let page: RevisionHistoryPage = serde_json::from_str(&body).unwrap();
    let revision_id = page.revisions[0].revision_id;

    let (status, body) = get_post_history(app.clone(), post.post_id, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("not found"), "body: {body}");
    let (status, body) = get_revision_detail(
        app.clone(),
        post.post_id,
        revision_id,
        Some(&stranger_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("not found"), "body: {body}");
    let (status, body) =
        get_post_history(app.clone(), PostId::from(999_999), Some(&owner_cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("not found"), "body: {body}");
    let (status, body) = get_revision_detail(
        app.clone(),
        PostId::from(999_999),
        revision_id,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("not found"), "body: {body}");
}
