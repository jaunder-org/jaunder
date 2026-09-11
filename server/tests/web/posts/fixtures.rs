use axum::{Router, http::StatusCode};
use common::ids::PostId;
use common::seed::{PageCursor, TimelineCursor};
use server_fn::ServerFn;

use crate::helpers::{create_user_and_session, post_form, post_json};
use storage::test_support::{Backend, TestEnv};

pub(super) async fn get_post_form(
    app: Router,
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
    post_form(app, <web::posts::Get as ServerFn>::PATH, body, cookie).await
}

// The listing helpers below post JSON, not a form: their timeline request carries
// a nested `TimelineCursor`, which the default form-urlencoded codec cannot carry.
// Draft and scheduled endpoints retain their independent `PageCursor` contract.
pub(super) async fn list_drafts(
    app: Router,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::posts::ListDrafts as ServerFn>::PATH,
        serde_json::json!({ "cursor": cursor, "limit": limit }),
        cookie,
    )
    .await
}

pub(super) async fn list_scheduled(
    app: Router,
    cursor: Option<PageCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::posts::ListScheduled as ServerFn>::PATH,
        serde_json::json!({ "cursor": cursor, "limit": limit }),
        cookie,
    )
    .await
}

pub(super) async fn publish_post_form(
    app: Router,
    post_id: PostId,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_form(
        app,
        <web::posts::Publish as ServerFn>::PATH,
        format!("post_id={post_id}"),
        cookie,
    )
    .await
}

pub(super) async fn list_user_posts(
    app: Router,
    username: &str,
    cursor: Option<TimelineCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::timeline::ListByUser as ServerFn>::PATH,
        serde_json::json!({
            "username": username,
            "request": { "order": "newest", "cursor": cursor, "limit": limit },
        }),
        cookie,
    )
    .await
}

pub(super) async fn list_local_timeline(
    app: Router,
    cursor: Option<TimelineCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::timeline::ListLocalTimeline as ServerFn>::PATH,
        serde_json::json!({
            "request": { "order": "newest", "cursor": cursor, "limit": limit },
        }),
        cookie,
    )
    .await
}

pub(super) async fn list_home_feed(
    app: Router,
    cursor: Option<TimelineCursor>,
    limit: u32,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json(
        app,
        <web::timeline::ListHomeFeed as ServerFn>::PATH,
        serde_json::json!({
            "request": { "order": "newest", "cursor": cursor, "limit": limit },
        }),
        cookie,
    )
    .await
}

pub(super) async fn login_and_env(backend: Backend) -> (TestEnv, String) {
    let env = backend.setup().await;
    let cookie = create_user_and_session(env.users(), env.sessions(), env.write_scope())
        .await
        .cookie();
    (env, cookie)
}
