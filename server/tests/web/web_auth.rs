use std::sync::Arc;

use axum::http::StatusCode;
use chrono::Utc;
use common::token::RawToken;
use common::username::Username;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{
    post_form_with_bearer, post_form_with_secure_flag, post_form_with_ua, session_cookie,
};
use storage::test_support::{backends, backends_matrix, Backend, TestEnv};

/// Extracts a raw token from a server-function JSON response body.
/// Successful server functions return a JSON string: `"<token>"`.
fn extract_token(body: &str) -> RawToken {
    let trimmed = body.trim();
    assert!(
        trimmed.starts_with('"') && trimmed.ends_with('"'),
        "expected JSON string in body, got: {trimmed}"
    );
    trimmed[1..trimmed.len() - 1]
        .parse()
        .expect("valid token in body")
}

// M2.9.8: `register` with Open policy creates user, sets cookie, returns non-empty token.
#[apply(backends)]
#[tokio::test]
async fn register_open_creates_user_sets_cookie_returns_token(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();

    let (status, set_cookie, body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let token = extract_token(&body);
    assert!(!token.is_empty());

    let cookie = set_cookie.expect("Set-Cookie header should be present");
    assert!(cookie.starts_with("session="), "cookie: {cookie}");

    let user = state
        .users
        .get_user_by_username(&"alice".parse::<Username>().unwrap())
        .await
        .unwrap();
    assert!(user.is_some(), "user should exist after registration");
}

#[apply(backends)]
#[tokio::test]
async fn register_duplicate_username_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();

    // Register alice once.
    post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    // Register alice again.
    let (status, _, _) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=otherpassword",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// M2.9.9: `register` with InviteOnly + valid code creates user, marks invite used, returns token.
#[apply(backends)]
#[tokio::test]
async fn register_invite_only_valid_code_creates_user_marks_invite_used(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();
    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let (status, _set_cookie, body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        format!(
            "username=bob&password=password123&invite_code={}",
            code.as_ref()
        ),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let token = extract_token(&body);
    assert!(!token.is_empty());

    let user = state
        .users
        .get_user_by_username(&"bob".parse::<Username>().unwrap())
        .await
        .unwrap();
    assert!(
        user.is_some(),
        "user should exist after invite registration"
    );

    let invites = state.invites.list_invites().await.unwrap();
    let invite = invites
        .iter()
        .find(|i| i.code.as_ref() == code.as_ref())
        .unwrap();
    assert!(invite.used_at.is_some(), "invite should be marked as used");
}

// M2.9.10: `register` with InviteOnly policy and missing code returns error.
#[apply(backends)]
#[tokio::test]
async fn register_invite_only_missing_code_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();

    let (status, _set_cookie, _body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=carol&password=password123",
        None,
        true,
    )
    .await;

    assert_ne!(status, StatusCode::OK);

    let user = state
        .users
        .get_user_by_username(&"carol".parse::<Username>().unwrap())
        .await
        .unwrap();
    assert!(
        user.is_none(),
        "user should not exist when invite code is missing"
    );
}

// M2.9.15: `register` with InviteOnly policy and invalid code returns error.
#[apply(backends)]
#[tokio::test]
async fn register_invite_only_invalid_code_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();

    let (status, _, _) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=password123&invite_code=invalid-code",
        None,
        true,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M2.9.16: `register` with InviteOnly policy and expired code returns error.
#[apply(backends)]
#[tokio::test]
async fn register_invite_only_expired_code_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();

    // Create an already-expired invite.
    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let (status, _, _) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        format!(
            "username=alice&password=password123&invite_code={}",
            code.as_ref()
        ),
        None,
        true,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M2.9.11: `register` with Closed policy returns error.
#[apply(backends)]
#[tokio::test]
async fn register_closed_policy_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    // Default policy is Closed; no need to set it explicitly.

    let (status, _set_cookie, _body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=dave&password=password123",
        None,
        true,
    )
    .await;

    assert_ne!(status, StatusCode::OK);

    let user = state
        .users
        .get_user_by_username(&"dave".parse::<Username>().unwrap())
        .await
        .unwrap();
    assert!(
        user.is_none(),
        "user should not exist on closed registration"
    );
}

// M2.9.12: `login` with correct password sets cookie and returns token.
#[apply(backends)]
#[tokio::test]
async fn login_correct_password_sets_cookie_and_returns_token(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=eve&password=password123",
        None,
        true,
    )
    .await;

    let (status, set_cookie, body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/login",
        "username=eve&password=password123",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let token = extract_token(&body);
    assert!(!token.is_empty());
    let cookie = set_cookie.expect("Set-Cookie header should be present on login");
    assert!(cookie.starts_with("session="), "cookie: {cookie}");
}

// (Removed `authenticated_home_page_shows_logged_in_indicator`: under #180 the
// server renders the anonymous projector view for `/`, never authed page content
// — the logged-in indicator now appears client-side after the CSR client boots,
// which is exercised by the e2e suite, not a server-response test.)

#[apply(backends)]
#[tokio::test]
async fn login_unknown_user_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _, _) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/login",
        "username=nobody&password=password123",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn login_with_label_creates_session_with_label(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    let (status, _, body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/login",
        "username=alice&password=password123&label=my-device",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = extract_token(&body);
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.label, "my-device");
}

#[apply(backends)]
#[tokio::test]
async fn login_with_empty_label_creates_session_without_label(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    let (status, _, body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/login",
        "username=alice&password=password123&label=",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = extract_token(&body);
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    // When no label provided, should default to "Unknown device"
    assert_eq!(record.label, "Unknown device");
}

// M2.9.12: `login` with long User-Agent truncates to 200 chars.
#[apply(backends)]
#[tokio::test]
async fn login_truncates_long_user_agent(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    // Build a long User-Agent string (>200 chars)
    let long_ua = "a".repeat(250);

    let (status, _, body) = post_form_with_ua(
        Arc::clone(&state),
        "/api/login",
        "username=alice&password=password123",
        None,
        &long_ua,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = extract_token(&body);
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    // Label should be truncated to 200 chars
    assert_eq!(record.label.len(), 200);
    assert_eq!(record.label, "a".repeat(200));
}

// M2.9.13: `login` with wrong password returns error.
#[apply(backends)]
#[tokio::test]
async fn login_wrong_password_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=frank&password=correctpassword",
        None,
        true,
    )
    .await;

    let (status, _set_cookie, _body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/login",
        "username=frank&password=wrongpassword",
        None,
        true,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M2.9.14: `logout` revokes session and clears cookie.
#[apply(backends)]
#[tokio::test]
async fn logout_revokes_session_and_clears_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    // Create a user and a session directly, bypassing the HTTP layer so we
    // have the raw token without needing to parse the register response.
    let user_id = state
        .users
        .create_user(
            &"grace".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let raw_token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();

    let sessions_before = state.sessions.list_sessions(user_id).await.unwrap();
    assert_eq!(
        sessions_before.len(),
        1,
        "one session should exist before logout"
    );

    let cookie_header = session_cookie(&raw_token);
    let (status, set_cookie, _body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/logout",
        "",
        Some(&cookie_header),
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let clear_cookie = set_cookie.expect("Set-Cookie header should be present on logout");
    assert!(
        clear_cookie.contains("Max-Age=0"),
        "logout should clear cookie via Max-Age=0, got: {clear_cookie}"
    );

    let sessions_after = state.sessions.list_sessions(user_id).await.unwrap();
    assert!(
        sessions_after.is_empty(),
        "session should be revoked after logout"
    );
}

// register() with a username containing a space (invalid after lowercase parse) returns error.
#[apply(backends)]
#[tokio::test]
async fn register_invalid_username_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .expect("failed to set registration policy");

    // "alice doe" lowercases to "alice doe" which fails Username parse
    // because Username only allows [a-z0-9_-]+.
    let (status, _set_cookie, _body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=alice%20doe&password=password123",
        None,
        true,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "register with space in username should fail"
    );
}

// register() with a password shorter than 8 characters returns error and creates no user.
#[apply(backends)]
#[tokio::test]
async fn register_short_password_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .expect("failed to set registration policy");

    let (status, _set_cookie, _body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=short",
        None,
        true,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "register with short password should fail"
    );

    let user = state
        .users
        .get_user_by_username(&"alice".parse::<Username>().expect("valid username"))
        .await
        .expect("database query failed");
    assert!(
        user.is_none(),
        "user should not be created when password is too short"
    );
}

// login() with a username containing a space (invalid parse) returns error.
#[apply(backends)]
#[tokio::test]
async fn login_invalid_username_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _set_cookie, _body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/login",
        "username=alice%20doe&password=password123",
        None,
        true,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "login with space in username should fail"
    );
}

// logout() via Authorization: Bearer <token> revokes the session and clears the cookie.
#[apply(backends)]
#[tokio::test]
async fn logout_with_bearer_token_revokes_session(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Create a user and session directly so we have the raw token.
    let user_id = state
        .users
        .create_user(
            &"henry".parse::<Username>().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
            false,
        )
        .await
        .expect("failed to create user");
    let raw_token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .expect("failed to create session");

    let sessions_before = state
        .sessions
        .list_sessions(user_id)
        .await
        .expect("failed to list sessions");
    assert_eq!(
        sessions_before.len(),
        1,
        "one session should exist before logout"
    );

    // POST to /api/logout with Bearer token instead of a cookie.
    let (status, set_cookie, _body) =
        post_form_with_bearer(Arc::clone(&state), "/api/logout", "", raw_token.as_ref()).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "logout with bearer token should succeed"
    );

    let clear_cookie = set_cookie.expect("Set-Cookie header should be present on logout");
    assert!(
        clear_cookie.contains("Max-Age=0"),
        "logout should clear cookie via Max-Age=0, got: {clear_cookie}"
    );

    let sessions_after = state
        .sessions
        .list_sessions(user_id)
        .await
        .expect("failed to list sessions after logout");
    assert!(
        sessions_after.is_empty(),
        "session should be revoked after bearer-token logout"
    );
}

// Unauthenticated logout: no session cookie → skips revoke, still clears cookie.
#[apply(backends)]
#[tokio::test]
async fn logout_without_session_still_clears_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, set_cookie, _body) =
        post_form_with_secure_flag(Arc::clone(&state), "/api/logout", "", None, true).await;

    assert_eq!(status, StatusCode::OK);
    let clear_cookie = set_cookie.expect("Set-Cookie header should be present on logout");
    assert!(
        clear_cookie.contains("Max-Age=0"),
        "logout should clear cookie via Max-Age=0, got: {clear_cookie}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn debug_api_routes_exist(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    // Send a request with no body to /api/register — if route exists we get
    // something other than 404 (probably a 400/422 for missing fields).
    let (status, _, _) =
        post_form_with_secure_flag(Arc::clone(&state), "/api/register", "", None, true).await;
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "/api/register route not registered (got 404)"
    );
}

#[apply(backends)]
#[tokio::test]
async fn get_registration_policy_returns_correct_value(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();

    // Server functions are POST by default.
    let (status, _, body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/get_registration_policy",
        "",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "\"invite_only\"");
}

// Shape B — `get_profile()` requires `AuthUser`; both an invalid token and a
// missing token must fail extraction with INTERNAL_SERVER_ERROR. Identical
// setup + assertion; only the supplied cookie varies.
#[apply(backends_matrix)]
#[case::invalid_token(Some("session=invalidtoken"))]
#[case::missing(None)]
#[tokio::test]
async fn auth_user_extraction_fails(backend: Backend, #[case] cookie: Option<&str>) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _, _) =
        post_form_with_secure_flag(Arc::clone(&state), "/api/get_profile", "", cookie, true).await;

    // Leptos server functions return 500 for ServerFnError.
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn logout_clears_cookie_without_secure_attribute_when_disabled(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"insecure".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let raw_token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();

    let cookie_header = session_cookie(&raw_token);
    let (status, set_cookie, _) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/logout",
        "",
        Some(&cookie_header),
        false,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let clear_cookie = set_cookie.expect("Set-Cookie header should be present");
    assert!(clear_cookie.contains("Max-Age=0"));
    assert!(!clear_cookie.contains("Secure"));
}

#[apply(backends)]
#[tokio::test]
async fn current_user_returns_username_when_authenticated(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let user_id = state
        .users
        .create_user(
            &"alice".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
            false,
        )
        .await
        .unwrap();
    let raw_token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();

    let cookie_header = session_cookie(&raw_token);
    let (status, _, body) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/current_user",
        "",
        Some(&cookie_header),
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "\"alice\"");
}

#[apply(backends)]
#[tokio::test]
async fn current_user_returns_null_when_unauthenticated(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _, body) =
        post_form_with_secure_flag(Arc::clone(&state), "/api/current_user", "", None, true).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "null");
}

#[apply(backends)]
#[tokio::test]
async fn register_sets_cookie_without_secure_attribute_when_disabled(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();

    let (status, set_cookie, _) = post_form_with_secure_flag(
        Arc::clone(&state),
        "/api/register",
        "username=insecure&password=password123",
        None,
        false,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let cookie = set_cookie.expect("Set-Cookie header should be present");
    assert!(cookie.contains("session="));
    assert!(!cookie.contains("Secure"));
}
