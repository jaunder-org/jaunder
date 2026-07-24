use std::sync::Arc;

use axum::http::StatusCode;
use common::site::SiteIdentity;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{create_operator_and_session, create_user_and_session, post_form};
use storage::test_support::{backends, Backend, TestEnv};

#[apply(backends)]
#[tokio::test]
async fn get_site_identity_requires_operator(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let anonymous_cookie = None;
    let member_cookie = create_user_and_session(&state, "member").await.cookie();

    let (anon_status, anon_body) = post_form(
        Arc::clone(&state),
        "/api/get_site_identity",
        "",
        anonymous_cookie,
    )
    .await;
    assert_eq!(
        anon_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {anon_body}"
    );
    assert!(anon_body.contains("unauthorized"), "body: {anon_body}");

    let (member_status, member_body) = post_form(
        Arc::clone(&state),
        "/api/get_site_identity",
        "",
        Some(&member_cookie),
    )
    .await;
    assert_eq!(
        member_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {member_body}"
    );
    assert!(member_body.contains("unauthorized"), "body: {member_body}");
}

#[apply(backends)]
#[tokio::test]
async fn get_site_identity_returns_defaults_when_unconfigured(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    let (status, body) = post_form(state, "/api/get_site_identity", "", Some(&cookie)).await;

    assert_eq!(status, StatusCode::OK, "body: {body}");
    let identity: SiteIdentity = serde_json::from_str(&body).expect("json");
    assert_eq!(identity.title, "Jaunder");
    assert_eq!(identity.base_url, None);
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_round_trips_via_get(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    let update_body = "title=My+Blog&base_url=https%3A%2F%2Fexample.com%2F";
    let (update_status, update_body_resp) = post_form(
        Arc::clone(&state),
        "/api/update_site_identity",
        update_body,
        Some(&cookie),
    )
    .await;
    assert_eq!(update_status, StatusCode::OK, "body: {update_body_resp}");

    let (get_status, get_body) = post_form(
        Arc::clone(&state),
        "/api/get_site_identity",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "body: {get_body}");
    let identity: SiteIdentity = serde_json::from_str(&get_body).expect("json");
    assert_eq!(identity.title, "My Blog");
    assert_eq!(identity.base_url.as_deref(), Some("https://example.com/"));
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_rejects_empty_title(#[case] backend: Backend) {
    // A whitespace-only `title` fails at typed-arg decode — the validating serde
    // bridge for `SiteTitle` rejects an empty/whitespace-only value, a non-OK server
    // -function error rather than a specific in-body Validation message (ADR-0065).
    // The client's disable-until-valid gate keeps a real browser from reaching this;
    // a raw POST is the malformed-client path.
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    let (status, body) = post_form(
        state,
        "/api/update_site_identity",
        "title=+++&base_url=https%3A%2F%2Fexample.com",
        Some(&cookie),
    )
    .await;

    assert_ne!(status, StatusCode::OK, "empty title should fail: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_rejects_non_http_base_url(#[case] backend: Backend) {
    // A non-http(s) `base_url` fails at typed-arg decode — the validating serde
    // bridge for `Option<AbsoluteUrl>` rejects it, a non-OK server-function error
    // rather than a specific Validation message (ADR-0065). The client's
    // disable-until-valid gate keeps a real browser from reaching this; a raw POST
    // is the malformed-client path.
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    let (status, body) = post_form(
        state,
        "/api/update_site_identity",
        "title=My+Blog&base_url=ftp%3A%2F%2Fexample.com",
        Some(&cookie),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "non-http base_url should fail: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_rejects_malformed_base_url(#[case] backend: Backend) {
    // A syntactically malformed `base_url` (not a URL at all) also fails at
    // typed-arg decode — same non-OK path as the non-http case (ADR-0065).
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    let (status, body) = post_form(
        state,
        "/api/update_site_identity",
        "title=My+Blog&base_url=not-a-url",
        Some(&cookie),
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "malformed base_url should fail: {body}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_omits_base_url_as_none(#[case] backend: Backend) {
    // Clearing the base URL is the dispatch-`None` path: the typed
    // `Option<AbsoluteUrl>` wire arg is *omitted* (serde decodes a missing Option
    // field to `None`); an empty `base_url=` would instead fail to parse.
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie = create_operator_and_session(&state, "operator")
        .await
        .cookie();

    let (update_status, update_body) = post_form(
        Arc::clone(&state),
        "/api/update_site_identity",
        "title=My+Blog",
        Some(&cookie),
    )
    .await;
    assert_eq!(update_status, StatusCode::OK, "body: {update_body}");

    let (get_status, get_body) = post_form(
        Arc::clone(&state),
        "/api/get_site_identity",
        "",
        Some(&cookie),
    )
    .await;
    assert_eq!(get_status, StatusCode::OK, "body: {get_body}");
    let identity: SiteIdentity = serde_json::from_str(&get_body).expect("json");
    assert_eq!(identity.base_url, None);
}

#[apply(backends)]
#[tokio::test]
async fn update_site_identity_requires_operator(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let anonymous_cookie = None;
    let member_cookie = create_user_and_session(&state, "member").await.cookie();

    let body = "title=My+Blog&base_url=https%3A%2F%2Fexample.com";

    let (anon_status, anon_body) = post_form(
        Arc::clone(&state),
        "/api/update_site_identity",
        body,
        anonymous_cookie,
    )
    .await;
    assert_eq!(
        anon_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {anon_body}"
    );
    assert!(anon_body.contains("unauthorized"), "body: {anon_body}");

    let (member_status, member_body) = post_form(
        state,
        "/api/update_site_identity",
        body,
        Some(&member_cookie),
    )
    .await;
    assert_eq!(
        member_status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "body: {member_body}"
    );
    assert!(member_body.contains("unauthorized"), "body: {member_body}");
}
