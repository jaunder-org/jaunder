//! Typed JSON request fixtures for valid Post create/update server-function calls.
//!
//! Malformed-input and exact wire-shape contracts deliberately stay on the raw
//! [`super::post_json`] seam.

use std::sync::Arc;

use axum::http::StatusCode;
use common::ids::PostId;
use server_fn::ServerFn;
use web::posts::PostInputs;

use super::post_json;

/// Sends a valid typed create-post request through the generated server-function wire contract.
pub async fn create_post_json(
    state: &Arc<storage::AppState>,
    post: PostInputs,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let payload = serde_json::to_value(web::posts::Create { post })
        .expect("generated create request serializes");
    post_json(
        state,
        <web::posts::Create as ServerFn>::PATH,
        payload,
        cookie,
    )
    .await
}

/// Sends a valid typed update-post request through the generated server-function wire contract.
pub async fn update_post_json(
    state: &Arc<storage::AppState>,
    post_id: PostId,
    post: PostInputs,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let payload = serde_json::to_value(web::posts::Update { post_id, post })
        .expect("generated update request serializes");
    post_json(
        state,
        <web::posts::Update as ServerFn>::PATH,
        payload,
        cookie,
    )
    .await
}
