use axum::http::StatusCode;
use serde_json::json;
use server_fn::ServerFn;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{confirmed_mutation, create_user_and_session, post_form, post_json};
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};
use web::posts::SavedPost;

async fn claim_pending(state: &std::sync::Arc<storage::AppState>) -> Vec<storage::FeedEventRecord> {
    let feed_events = state.feed_events.clone();
    storage::test_support::confirmed_for(
        state
            .write_scope
            .run(move |transaction| {
                Box::pin(async move {
                    feed_events
                        .claim_pending_batch(transaction, 100, chrono::Duration::seconds(86400))
                        .await
                })
            })
            .await
            .expect("claim batch"),
        "claim batch acknowledgement",
    )
}

fn confirmed_post_id(response: &str) -> i64 {
    i64::from(confirmed_mutation::<SavedPost>(response).post_id)
}

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

    let session = create_user_and_session(&state).await;

    let body = json!({
        "post": {
            "body": "Test post",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": true,
            "tags": tags
        }
    });

    let (status, _response) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        body,
        Some(&session.cookie()),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let batch = claim_pending(&state).await;

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

    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let create_body = json!({
        "post": {
            "body": "Test post",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": true,
            "tags": Some(vec!["rust".to_string(), "web".to_string()])
        }
    });

    let (status, create_response) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        create_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let post_id = confirmed_post_id(&create_response);

    // Drain initial create events
    let _initial_batch = claim_pending(&state).await;

    // Union should be {leptos, rust, web} = 3 tags
    let update_body = json!({
        "post_id": post_id,
        "post": {
            "body": "Updated post",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": false,
            "tags": Some(vec!["rust".to_string(), "leptos".to_string()])
        }
    });

    let (status, _) = post_json(
        &state,
        <web::posts::Update as ServerFn>::PATH,
        update_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let update_batch = claim_pending(&state).await;

    // Expected: Site (3) + User (3) + 3 tags × (SiteTag + UserTag) × 3 formats = 6 + 18 = 24 rows
    assert_eq!(
        update_batch.len(),
        24,
        "Expected 24 feed events from update with tag change: {update_batch:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn unpublish_enqueues_site_and_user_and_tag_feeds(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let create_body = json!({
        "post": {
            "body": "Test post",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": true,
            "tags": Some(vec!["rust".to_string()])
        }
    });

    let (status, create_response) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        create_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let post_id = confirmed_post_id(&create_response);

    // Drain initial create events
    let _initial_batch = claim_pending(&state).await;

    let unpublish_body = format!("post_id={post_id}");
    let (status, _) = post_form(
        &state,
        <web::posts::Unpublish as ServerFn>::PATH,
        unpublish_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let unpublish_batch = claim_pending(&state).await;

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

    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let create_body = json!({
        "post": {
            "body": "Test post",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": true,
            "tags": Some(vec!["rust".to_string()])
        }
    });

    let (status, create_response) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        create_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let post_id = confirmed_post_id(&create_response);

    // Drain initial create events
    let _initial_batch = claim_pending(&state).await;

    let delete_body = format!("post_id={post_id}");
    let (status, _) = post_form(
        &state,
        <web::posts::Delete as ServerFn>::PATH,
        delete_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let delete_batch = claim_pending(&state).await;

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

    let session = create_user_and_session(&state).await;
    let cookie = session.cookie();

    let create_body = json!({
        "post": {
            "body": "Test draft",
            "format": "markdown",
            "slug_override": None::<String>,
            "publish": false,
            "tags": Some(vec!["rust".to_string()])
        }
    });

    let (status, create_response) = post_json(
        &state,
        <web::posts::Create as ServerFn>::PATH,
        create_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let post_id = confirmed_post_id(&create_response);

    // Drain any events from create (drafts still enqueue as per spec)
    let _initial_batch = claim_pending(&state).await;

    let delete_body = format!("post_id={post_id}");
    let (status, _) = post_form(
        &state,
        <web::posts::Delete as ServerFn>::PATH,
        delete_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let delete_batch = claim_pending(&state).await;

    // Expected: 0 rows (draft posts don't affect feeds)
    assert_eq!(
        delete_batch.len(),
        0,
        "Expected 0 feed events from deleting draft post"
    );
}
