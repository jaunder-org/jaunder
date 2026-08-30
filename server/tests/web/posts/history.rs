use std::sync::Arc;

use axum::http::StatusCode;
use common::ids::{PostId, RevisionId};
use common::revision_history::{RevisionHistoryAudience, RevisionHistoryDetail};
use rstest::*;
use rstest_reuse::*;
use server_fn::ServerFn;
use storage::test_support::{Backend, TestEnv, backends};
use web::posts::{PostRevisionHistory, RevisionHistoryPage, SavedPost};

use crate::helpers::{create_user_and_session, post_json};

use super::fixtures::{create_post_json, update_post_json};

async fn list_history(
    state: &Arc<storage::AppState>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        state,
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
    state: &Arc<storage::AppState>,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        state,
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
    state: &Arc<storage::AppState>,
    post_id: PostId,
    revision_id: RevisionId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        state,
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
async fn revision_history_endpoints_hide_anonymous_access(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, body) = list_history(&state, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("Post not found"), "body: {body}");
    let expected_body = body;

    let (status, body) = get_post_history(&state, PostId::from(1), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(
        body, expected_body,
        "anonymous history responses must not disclose scope"
    );

    let (status, body) =
        get_revision_detail(&state, PostId::from(1), RevisionId::from(1), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert_eq!(
        body, expected_body,
        "anonymous history responses must not disclose scope"
    );
}

#[apply(backends)]
#[tokio::test]
async fn revision_history_http_exposes_page_current_and_detail_fields(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();
    let (status, body) = create_post_json(
        &state,
        "# First\n\noriginal body",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let post: SavedPost = serde_json::from_str(&body).unwrap();
    let (status, body) = update_post_json(
        &state,
        post.post_id,
        "# Second\n\nupdated body",
        "markdown",
        None,
        false,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let (status, body) = list_history(&state, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let page: RevisionHistoryPage = serde_json::from_str(&body).unwrap();
    assert_eq!(page.revisions.len(), 1);
    assert_eq!(page.revisions[0].post_id, post.post_id);
    assert!(page.next_cursor.is_none());
    assert!(!page.has_more);

    let (status, body) = get_post_history(&state, post.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let history: PostRevisionHistory = serde_json::from_str(&body).unwrap();
    assert_eq!(history.current.post_id, post.post_id);
    assert_eq!(
        history.revisions.revisions[0].revision_id,
        page.revisions[0].revision_id
    );

    let (status, body) = get_revision_detail(
        &state,
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
    let TestEnv { state, base: _base } = backend.setup().await;
    let owner = create_user_and_session(&state).await;
    let owner_cookie = owner.cookie();
    let stranger_cookie = create_user_and_session(&state).await.cookie();
    let (status, body) = create_post_json(
        &state,
        "# Private\n\noriginal body",
        "markdown",
        None,
        false,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let post: SavedPost = serde_json::from_str(&body).unwrap();
    let (status, body) = update_post_json(
        &state,
        post.post_id,
        "# Changed\n\nupdated body",
        "markdown",
        None,
        false,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let (_, body) = list_history(&state, Some(&owner_cookie)).await;
    let page: RevisionHistoryPage = serde_json::from_str(&body).unwrap();
    let revision_id = page.revisions[0].revision_id;

    let (status, body) = get_post_history(&state, post.post_id, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("not found"), "body: {body}");
    let (status, body) =
        get_revision_detail(&state, post.post_id, revision_id, Some(&stranger_cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("not found"), "body: {body}");
    let (status, body) = get_post_history(&state, PostId::from(999_999), Some(&owner_cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("not found"), "body: {body}");
    let (status, body) = get_revision_detail(
        &state,
        PostId::from(999_999),
        revision_id,
        Some(&owner_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {body}");
    assert!(body.contains("not found"), "body: {body}");
}
