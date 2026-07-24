use axum::http::StatusCode;
use serde_json::json;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_user_and_session, post_form, post_json};
use storage::test_support::{backends, backends_matrix, Backend, TestEnv};

// Creating a published post enqueues the Site and User feeds (3 formats each =
// 6 rows), plus 2 rows per tag (SiteTag + UserTag) × 3 formats. With no tags
// that's 6 rows; with two tags it's 6 + 2×2×3 = 18 rows.
#[apply(backends_matrix)]
#[case::no_tags(None::<Vec<String>>, 6)]
#[case::two_tags(Some(vec!["rust".to_string(), "web".to_string()]), 18)]
#[tokio::test]
async fn create_published_post_enqueues_expected_feeds(
    backend: Backend,
    #[case] tags: Option<Vec<String>>,
    #[case] expected_rows: usize,
) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state, "alice").await;
    let cookie = session.cookie();

    let body = json!({
        "args": {
            "body": "Test post",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": true,
            "tags": tags
        }
    });

    let (status, _response) =
        post_json(state.clone(), "/api/create_post", body, Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK);

    let batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    assert_eq!(
        batch.len(),
        expected_rows,
        "Expected {expected_rows} feed events for published post"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_with_tag_change_enqueues_old_and_new_tags(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state, "alice").await;
    let cookie = session.cookie();

    let create_body = json!({
        "args": {
            "body": "Test post",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": true,
            "tags": Some(vec!["rust".to_string(), "web".to_string()])
        }
    });

    let (status, create_response) = post_json(
        state.clone(),
        "/api/create_post",
        create_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let create_json: serde_json::Value =
        serde_json::from_str(&create_response).expect("parse create response");
    let post_id = create_json
        .get("post_id")
        .and_then(serde_json::Value::as_i64)
        .expect("get post_id");

    // Drain initial create events
    let _initial_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Union should be {leptos, rust, web} = 3 tags
    let update_body = json!({
        "args": {
            "post_id": post_id,
            "body": "Updated post",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": false,
            "tags": Some(vec!["rust".to_string(), "leptos".to_string()])
        }
    });

    let (status, _) = post_json(
        state.clone(),
        "/api/update_post",
        update_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let update_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Expected: Site (3) + User (3) + 3 tags × (SiteTag + UserTag) × 3 formats = 6 + 18 = 24 rows
    assert_eq!(
        update_batch.len(),
        24,
        "Expected 24 feed events from update with tag change"
    );
}

#[apply(backends)]
#[tokio::test]
async fn unpublish_enqueues_site_and_user_and_tag_feeds(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state, "alice").await;
    let cookie = session.cookie();

    let create_body = json!({
        "args": {
            "body": "Test post",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": true,
            "tags": Some(vec!["rust".to_string()])
        }
    });

    let (status, create_response) = post_json(
        state.clone(),
        "/api/create_post",
        create_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let create_json: serde_json::Value =
        serde_json::from_str(&create_response).expect("parse create response");
    let post_id = create_json
        .get("post_id")
        .and_then(serde_json::Value::as_i64)
        .expect("get post_id");

    // Drain initial create events
    let _initial_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    let unpublish_body = format!("post_id={post_id}");
    let (status, _) = post_form(
        state.clone(),
        "/api/unpublish_post",
        unpublish_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let unpublish_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Expected: Site (3) + User (3) + 1 tag × (SiteTag + UserTag) × 3 formats = 6 + 6 = 12 rows
    assert_eq!(
        unpublish_batch.len(),
        12,
        "Expected 12 feed events from unpublish with 1 tag"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_published_post_enqueues_feeds(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state, "alice").await;
    let cookie = session.cookie();

    let create_body = json!({
        "args": {
            "body": "Test post",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": true,
            "tags": Some(vec!["rust".to_string()])
        }
    });

    let (status, create_response) = post_json(
        state.clone(),
        "/api/create_post",
        create_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let create_json: serde_json::Value =
        serde_json::from_str(&create_response).expect("parse create response");
    let post_id = create_json
        .get("post_id")
        .and_then(serde_json::Value::as_i64)
        .expect("get post_id");

    // Drain initial create events
    let _initial_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    let delete_body = format!("post_id={post_id}");
    let (status, _) = post_form(
        state.clone(),
        "/api/delete_post",
        delete_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let delete_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Expected: Site (3) + User (3) + 1 tag × (SiteTag + UserTag) × 3 formats = 6 + 6 = 12 rows
    assert_eq!(
        delete_batch.len(),
        12,
        "Expected 12 feed events from deleting published post with 1 tag"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_draft_post_enqueues_nothing(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state, "alice").await;
    let cookie = session.cookie();

    let create_body = json!({
        "args": {
            "body": "Test draft",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": false,
            "tags": Some(vec!["rust".to_string()])
        }
    });

    let (status, create_response) = post_json(
        state.clone(),
        "/api/create_post",
        create_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let create_json: serde_json::Value =
        serde_json::from_str(&create_response).expect("parse create response");
    let post_id = create_json
        .get("post_id")
        .and_then(serde_json::Value::as_i64)
        .expect("get post_id");

    // Drain any events from create (drafts still enqueue as per spec)
    let _initial_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    let delete_body = format!("post_id={post_id}");
    let (status, _) = post_form(
        state.clone(),
        "/api/delete_post",
        delete_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let delete_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Expected: 0 rows (draft posts don't affect feeds)
    assert_eq!(
        delete_batch.len(),
        0,
        "Expected 0 feed events from deleting draft post"
    );
}
