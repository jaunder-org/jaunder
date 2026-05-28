mod helpers;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::password::Password;
use common::username::Username;
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

use helpers::{ensure_server_fns_registered, test_options, test_state};

async fn post_json(
    state: Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    ensure_server_fns_registered();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder.body(Body::from(body.into())).unwrap();

    let app = jaunder::create_router(
        test_options(),
        state,
        helpers::noop_mailer(),
        true,
        helpers::tmp_storage_path(),
    );
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();

    (status, body_str)
}

fn create_session_cookie(token: &str) -> String {
    format!("session={}", token)
}

#[tokio::test]
async fn create_published_post_enqueues_site_and_user_feeds() {
    let base = TempDir::new().expect("temp dir");
    let state = test_state(&base).await;

    // Create a user
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    // Create session
    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create session");
    let cookie = create_session_cookie(token.as_str());

    // Create published post with no tags
    let body = json!({
        "body": "Test post",
        "format": "markdown",
        "slug_override": None::<String>,
        "publish": true,
        "tags": None::<Vec<String>>
    });

    let (status, _response) = post_json(
        state.clone(),
        "/api/create_post",
        body.to_string(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Claim pending feed events and count them
    let batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Expected: Site (3 formats) + User (3 formats) = 6 rows
    assert_eq!(
        batch.len(),
        6,
        "Expected 6 feed events for published post with no tags"
    );
}

#[tokio::test]
async fn create_post_with_two_tags_enqueues_tag_feeds_too() {
    let base = TempDir::new().expect("temp dir");
    let state = test_state(&base).await;

    // Create a user
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    // Create session
    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create session");
    let cookie = create_session_cookie(token.as_str());

    // Create published post with two tags
    let body = json!({
        "body": "Test post",
        "format": "markdown",
        "slug_override": None::<String>,
        "publish": true,
        "tags": Some(vec!["rust".to_string(), "web".to_string()])
    });

    let (status, _response) = post_json(
        state.clone(),
        "/api/create_post",
        body.to_string(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Claim pending feed events and count them
    let batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Expected: Site (3) + User (3) + 2 tags × (SiteTag + UserTag) × 3 formats = 6 + 12 = 18 rows
    assert_eq!(
        batch.len(),
        18,
        "Expected 18 feed events for published post with 2 tags"
    );
}

#[tokio::test]
async fn update_with_tag_change_enqueues_old_and_new_tags() {
    let base = TempDir::new().expect("temp dir");
    let state = test_state(&base).await;

    // Create a user
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    // Create session
    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create session");
    let cookie = create_session_cookie(token.as_str());

    // Create published post with initial tags {rust, web}
    let create_body = json!({
        "body": "Test post",
        "format": "markdown",
        "slug_override": None::<String>,
        "publish": true,
        "tags": Some(vec!["rust".to_string(), "web".to_string()])
    });

    let (status, create_response) = post_json(
        state.clone(),
        "/api/create_post",
        create_body.to_string(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Extract post_id from response
    let create_json: serde_json::Value =
        serde_json::from_str(&create_response).expect("parse create response");
    let post_id = create_json
        .get("post_id")
        .and_then(|v| v.as_i64())
        .expect("get post_id");

    // Drain initial create events
    let _initial_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Update post tags from {rust, web} to {rust, leptos}
    // Union should be {leptos, rust, web} = 3 tags
    let update_body = json!({
        "post_id": post_id,
        "body": "Updated post",
        "format": "markdown",
        "slug_override": None::<String>,
        "publish": false,
        "tags": Some(vec!["rust".to_string(), "leptos".to_string()])
    });

    let (status, _) = post_json(
        state.clone(),
        "/api/update_post",
        update_body.to_string(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Claim feed events from the update
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

#[tokio::test]
async fn unpublish_enqueues_site_and_user_and_tag_feeds() {
    let base = TempDir::new().expect("temp dir");
    let state = test_state(&base).await;

    // Create a user
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    // Create session
    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create session");
    let cookie = create_session_cookie(token.as_str());

    // Create published post with 1 tag
    let create_body = json!({
        "body": "Test post",
        "format": "markdown",
        "slug_override": None::<String>,
        "publish": true,
        "tags": Some(vec!["rust".to_string()])
    });

    let (status, create_response) = post_json(
        state.clone(),
        "/api/create_post",
        create_body.to_string(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Extract post_id from response
    let create_json: serde_json::Value =
        serde_json::from_str(&create_response).expect("parse create response");
    let post_id = create_json
        .get("post_id")
        .and_then(|v| v.as_i64())
        .expect("get post_id");

    // Drain initial create events
    let _initial_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Unpublish the post
    let unpublish_body = format!("post_id={}", post_id);
    let (status, _) = post_json(
        state.clone(),
        "/api/unpublish_post",
        unpublish_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Claim feed events from the unpublish
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

#[tokio::test]
async fn delete_published_post_enqueues_feeds() {
    let base = TempDir::new().expect("temp dir");
    let state = test_state(&base).await;

    // Create a user
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    // Create session
    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create session");
    let cookie = create_session_cookie(token.as_str());

    // Create published post with 1 tag
    let create_body = json!({
        "body": "Test post",
        "format": "markdown",
        "slug_override": None::<String>,
        "publish": true,
        "tags": Some(vec!["rust".to_string()])
    });

    let (status, create_response) = post_json(
        state.clone(),
        "/api/create_post",
        create_body.to_string(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Extract post_id from response
    let create_json: serde_json::Value =
        serde_json::from_str(&create_response).expect("parse create response");
    let post_id = create_json
        .get("post_id")
        .and_then(|v| v.as_i64())
        .expect("get post_id");

    // Drain initial create events
    let _initial_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Delete the post
    let delete_body = format!("post_id={}", post_id);
    let (status, _) = post_json(
        state.clone(),
        "/api/delete_post",
        delete_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Claim feed events from the delete
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

#[tokio::test]
async fn delete_draft_post_enqueues_nothing() {
    let base = TempDir::new().expect("temp dir");
    let state = test_state(&base).await;

    // Create a user
    let username: Username = "alice".parse().expect("valid username");
    let password: Password = "password123".parse().expect("valid password");
    let user_id = state
        .users
        .create_user(&username, &password, None, false)
        .await
        .expect("create user");

    // Create session
    let token = state
        .sessions
        .create_session(user_id, None)
        .await
        .expect("create session");
    let cookie = create_session_cookie(token.as_str());

    // Create draft post (not published)
    let create_body = json!({
        "body": "Test draft",
        "format": "markdown",
        "slug_override": None::<String>,
        "publish": false,
        "tags": Some(vec!["rust".to_string()])
    });

    let (status, create_response) = post_json(
        state.clone(),
        "/api/create_post",
        create_body.to_string(),
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Extract post_id from response
    let create_json: serde_json::Value =
        serde_json::from_str(&create_response).expect("parse create response");
    let post_id = create_json
        .get("post_id")
        .and_then(|v| v.as_i64())
        .expect("get post_id");

    // Drain any events from create (drafts still enqueue as per spec)
    let _initial_batch = state
        .feed_events
        .claim_pending_batch(100, chrono::Duration::seconds(86400))
        .await
        .expect("claim batch");

    // Delete the draft post
    let delete_body = format!("post_id={}", post_id);
    let (status, _) = post_json(
        state.clone(),
        "/api/delete_post",
        delete_body,
        Some(&cookie),
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    // Claim feed events from the delete
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
