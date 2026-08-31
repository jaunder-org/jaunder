use std::sync::Arc;

use axum::http::StatusCode;
use common::ids::PostId;
use common::seed::PageCursor;
use server_fn::ServerFn;

use crate::helpers::{create_user_and_session, post_form, post_json};
use storage::test_support::{Backend, TestBase, TestEnv};

pub(super) async fn get_post_form(
    state: &Arc<storage::AppState>,
    username: &str,
    year: i32,
    month: u32,
    day: u32,
    slug: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    // `get_post` takes a single `date: PermalinkDate` wire arg (serde-transparent →
    // the ISO `YYYY-MM-DD` field, #583).
    let body = format!("username={username}&date={year:04}-{month:02}-{day:02}&slug={slug}");
    post_form(state, <web::posts::Get as ServerFn>::PATH, body, cookie).await
}

// The listing helpers below post JSON, not a form: their `cursor` is a nested
// `PageCursor`, which the default form-urlencoded codec cannot carry, so the
// endpoints declare `input = Json`.
pub(super) async fn list_drafts(
    state: &Arc<storage::AppState>,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        state,
        <web::posts::ListDrafts as ServerFn>::PATH,
        serde_json::json!({ "cursor": cursor, "limit": limit }),
        cookie,
    )
    .await
}

pub(super) async fn list_scheduled(
    state: &Arc<storage::AppState>,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        state,
        <web::posts::ListScheduled as ServerFn>::PATH,
        serde_json::json!({ "cursor": cursor, "limit": limit }),
        cookie,
    )
    .await
}

pub(super) async fn publish_post_form(
    state: &Arc<storage::AppState>,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_form(
        state,
        <web::posts::Publish as ServerFn>::PATH,
        format!("post_id={post_id}"),
        cookie,
    )
    .await
}

pub(super) async fn list_user_posts(
    state: &Arc<storage::AppState>,
    username: &str,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        state,
        <web::timeline::ListByUser as ServerFn>::PATH,
        serde_json::json!({ "username": username, "cursor": cursor, "limit": limit }),
        cookie,
    )
    .await
}

pub(super) async fn list_local_timeline(
    state: &Arc<storage::AppState>,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        state,
        <web::timeline::ListLocalTimeline as ServerFn>::PATH,
        serde_json::json!({ "cursor": cursor, "limit": limit }),
        cookie,
    )
    .await
}

pub(super) async fn list_home_feed(
    state: &Arc<storage::AppState>,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        state,
        <web::timeline::ListHomeFeed as ServerFn>::PATH,
        serde_json::json!({ "cursor": cursor, "limit": limit }),
        cookie,
    )
    .await
}

pub(super) async fn login_and_state(
    backend: Backend,
) -> (TestBase, Arc<storage::AppState>, String) {
    let TestEnv { state, base } = backend.setup().await;
    let cookie = create_user_and_session(&state).await.cookie();
    (base, state, cookie)
}
