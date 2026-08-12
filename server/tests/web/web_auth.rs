use axum::http::StatusCode;
use chrono::Utc;
use common::config_key::SiteConfigKey;
use common::session_label::MAX_SESSION_LABEL_CHARS;
use common::token::RawToken;
use common::username::Username;
use server_fn::ServerFn;

use rstest::*;
use rstest_reuse::*;

use crate::helpers::{
    create_user_and_session, post_form_with_bearer, post_form_with_secure_flag, post_form_with_ua,
    token_from_set_cookie,
};
use storage::test_support::{Backend, TestEnv, backends, backends_matrix};

/// The session token a login/register response established, read from its
/// `Set-Cookie` header — the only channel that carries it (#533). Tests reach the
/// session this way, which is also what proves the cookie is still being set.
fn session_token_of(set_cookie: Option<String>) -> RawToken {
    let cookie = set_cookie.expect("Set-Cookie header should be present");
    token_from_set_cookie(&cookie)
}

/// Asserts a response body does not carry the session token its `Set-Cookie`
/// established.
///
/// These assertions are the enforcement mechanism named by
/// `docs/adr/0107-web-session-establishment-is-cookie-only.md`, so the invariant
/// is spelled out once here rather than re-derived per endpoint. It compares the
/// token *value*: `register`'s body is a bare `null`, so checking for a `"token"`
/// field name would be vacuous.
fn assert_body_carries_no_token(endpoint: &str, body: &str, token: &RawToken) {
    assert!(
        !body.contains(&**token),
        "{endpoint} body leaked the session token: {body}"
    );
}

/// The `is_operator` flag from a login response body — all that `login` returns now
/// that the session token travels only in the `HttpOnly` cookie (#533).
///
/// Deserializes its own shape rather than `web::auth::LoginResponse`, so a change
/// to that type cannot silently make this assertion vacuous.
fn is_operator_from_body(body: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Resp {
        is_operator: bool,
    }
    serde_json::from_str::<Resp>(body.trim())
        .expect("valid login JSON body")
        .is_operator
}

// M2.9.8: `register` with Open policy creates the user and establishes the session
// through the HttpOnly cookie alone (#533).
#[apply(backends)]
#[tokio::test]
async fn register_open_creates_user_and_sets_session_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();

    let (status, set_cookie, body) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);

    let cookie = set_cookie.expect("Set-Cookie header should be present");
    assert!(cookie.starts_with("session="), "cookie: {cookie}");

    let cookie_token = token_from_set_cookie(&cookie);
    assert_body_carries_no_token("register", &body, &cookie_token);

    let user = state
        .users
        .get_user_by_username(&"alice".parse::<Username>().unwrap())
        .await
        .unwrap()
        .expect("user should exist after registration");

    // …and the cookie actually establishes a session for that user. A
    // `starts_with("session=")` check alone would pass against a cookie carrying a
    // token that authenticates nothing.
    let record = state
        .sessions
        .authenticate(&cookie_token)
        .await
        .expect("the register cookie authenticates");
    assert_eq!(record.user_id, user.user_id);
}

#[apply(backends)]
#[tokio::test]
async fn register_duplicate_username_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();

    // Register alice once.
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    // Register alice again.
    let (status, _, _) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=otherpassword",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// M2.9.9: `register` with InviteOnly + valid code creates user, marks invite used,
// and establishes the session through the cookie (#533).
#[apply(backends)]
#[tokio::test]
async fn register_invite_only_valid_code_creates_user_marks_invite_used(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "invite_only")
        .await
        .unwrap();
    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let (status, set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        format!(
            "username=bob&password=password123&invite_code={}",
            code.as_ref()
        ),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let cookie = set_cookie.expect("Set-Cookie header should be present");
    assert!(cookie.starts_with("session="), "cookie: {cookie}");

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
        .set(SiteConfigKey::SiteRegistrationPolicy, "invite_only")
        .await
        .unwrap();

    let (status, _set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
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
        .set(SiteConfigKey::SiteRegistrationPolicy, "invite_only")
        .await
        .unwrap();

    let (status, _, _) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
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
        .set(SiteConfigKey::SiteRegistrationPolicy, "invite_only")
        .await
        .unwrap();

    // Create an already-expired invite.
    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let (status, _, _) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
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
        &state,
        <web::registration::Register as ServerFn>::PATH,
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
async fn login_correct_password_sets_session_cookie(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=eve&password=password123",
        None,
        true,
    )
    .await;

    let (status, set_cookie, body) = post_form_with_secure_flag(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        "username=eve&password=password123",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let cookie = set_cookie.expect("Set-Cookie header should be present on login");
    assert!(cookie.starts_with("session="), "cookie: {cookie}");

    let cookie_token = token_from_set_cookie(&cookie);
    assert_body_carries_no_token("login", &body, &cookie_token);
}

// #591: login's response carries `is_operator` so the client writes a complete
// marker (flash-free first login). A freshly registered user is a non-operator.
#[apply(backends)]
#[tokio::test]
async fn login_returns_is_operator_flag(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    let (status, _cookie, body) = post_form_with_secure_flag(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let is_operator = is_operator_from_body(&body);
    assert!(!is_operator, "a freshly registered user is not an operator");
}

#[apply(backends)]
#[tokio::test]
async fn login_unknown_user_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;

    let (status, _, _) = post_form_with_secure_flag(
        &state,
        <web::auth::Login as ServerFn>::PATH,
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
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    let (status, set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        "username=alice&password=password123&label=my-device",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = session_token_of(set_cookie);
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.label, "my-device");
}

#[apply(backends)]
#[tokio::test]
async fn login_with_empty_label_falls_back_to_user_agent_default(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    let (status, set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        "username=alice&password=password123&label=",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = session_token_of(set_cookie);
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    // An empty `label=` decodes to `None` (the Option form layer absorbs it before
    // SessionLabel's deserializer runs), so this takes the User-Agent branch — and
    // no UA header is sent here, so `from_lossy` supplies its default.
    assert_eq!(record.label, "Unknown device");
}

#[apply(backends)]
#[tokio::test]
async fn login_rejects_whitespace_only_label(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    // A whitespace-only label is rejected at the typed-wire-arg decode
    // (SessionLabel's FromStr trims, then rejects empty) — it must not fall
    // through to the User-Agent branch. Surfaces as 500, the session-fn convention.
    let (status, _, body) = post_form_with_secure_flag(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        "username=alice&password=password123&label=%20%20",
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    // Decode fails before the handler body runs, so no session is minted.
    assert!(!body.contains("\"token\""), "token minted: {body}");
}

#[apply(backends)]
#[tokio::test]
async fn login_rejects_overlong_label(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    // Past MAX_SESSION_LABEL_CHARS (255) the label is rejected at decode rather
    // than silently truncated, matching create_app_password_rejects_overlong_label.
    let overlong = "a".repeat(256);
    let (status, _, body) = post_form_with_secure_flag(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        format!("username=alice&password=password123&label={overlong}"),
        None,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.contains("\"token\""), "token minted: {body}");
}

// A long User-Agent is bounded by MAX_SESSION_LABEL_CHARS (255), the newtype's own
// cap, rather than a second hand-rolled 200-char cap in `login` (#685).
#[apply(backends)]
#[tokio::test]
async fn login_bounds_long_user_agent_at_session_label_cap(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    // 250 < 255, so this UA survives intact rather than being truncated.
    let long_ua = "a".repeat(250);

    let (status, set_cookie, _body) = post_form_with_ua(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        &long_ua,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = session_token_of(set_cookie);
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.label, "a".repeat(250).as_str());
}

// Past the cap, the UA is truncated (not rejected): it is an internally derived
// value, so it goes through the lossy door (ADR-0063 §2), unlike a submitted label.
#[apply(backends)]
#[tokio::test]
async fn login_truncates_user_agent_past_session_label_cap(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        true,
    )
    .await;

    let long_ua = "a".repeat(300);

    let (status, set_cookie, _body) = post_form_with_ua(
        &state,
        <web::auth::Login as ServerFn>::PATH,
        "username=alice&password=password123",
        None,
        &long_ua,
        true,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = session_token_of(set_cookie);
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.label.chars().count(), MAX_SESSION_LABEL_CHARS);
}

// M2.9.13: `login` with wrong password returns error.
#[apply(backends)]
#[tokio::test]
async fn login_wrong_password_returns_error(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();
    post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "username=frank&password=correctpassword",
        None,
        true,
    )
    .await;

    let (status, _set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::auth::Login as ServerFn>::PATH,
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
    let session = create_user_and_session(&state).await;

    let sessions_before = state.sessions.list_sessions(session.user_id).await.unwrap();
    assert_eq!(
        sessions_before.len(),
        1,
        "one session should exist before logout"
    );

    let cookie_header = session.cookie();
    let (status, set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::auth::Logout as ServerFn>::PATH,
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

    let sessions_after = state.sessions.list_sessions(session.user_id).await.unwrap();
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
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .expect("failed to set registration policy");

    // "alice doe" lowercases to "alice doe" which fails Username parse
    // because Username only allows [a-z0-9_-]+.
    let (status, _set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
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
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .expect("failed to set registration policy");

    let (status, _set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
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
        &state,
        <web::auth::Login as ServerFn>::PATH,
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
    let session = create_user_and_session(&state).await;

    let sessions_before = state
        .sessions
        .list_sessions(session.user_id)
        .await
        .expect("failed to list sessions");
    assert_eq!(
        sessions_before.len(),
        1,
        "one session should exist before logout"
    );

    // POST to /api/auth/logout with Bearer token instead of a cookie.
    let (status, set_cookie, _body) = post_form_with_bearer(
        &state,
        <web::auth::Logout as ServerFn>::PATH,
        "",
        session.token.as_ref(),
    )
    .await;

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
        .list_sessions(session.user_id)
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

    let (status, set_cookie, _body) = post_form_with_secure_flag(
        &state,
        <web::auth::Logout as ServerFn>::PATH,
        "",
        None,
        true,
    )
    .await;

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

    // Send a request with no body to /api/registration/register — if route exists we get
    // something other than 404 (probably a 400/422 for missing fields).
    let (status, _, _) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
        "",
        None,
        true,
    )
    .await;
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "/api/registration/register route not registered (got 404)"
    );
}

#[apply(backends)]
#[tokio::test]
async fn get_registration_policy_returns_correct_value(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "invite_only")
        .await
        .unwrap();

    // Server functions are POST by default.
    let (status, _, body) = post_form_with_secure_flag(
        &state,
        <web::registration::GetPolicy as ServerFn>::PATH,
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

    let (status, _, _) = post_form_with_secure_flag(
        &state,
        <web::profile::Get as ServerFn>::PATH,
        "",
        cookie,
        true,
    )
    .await;

    // Leptos server functions return 500 for ServerFnError.
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[apply(backends)]
#[tokio::test]
async fn logout_clears_cookie_without_secure_attribute_when_disabled(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    let cookie_header = create_user_and_session(&state).await.cookie();
    let (status, set_cookie, _) = post_form_with_secure_flag(
        &state,
        <web::auth::Logout as ServerFn>::PATH,
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
async fn register_sets_cookie_without_secure_attribute_when_disabled(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    state
        .site_config
        .set(SiteConfigKey::SiteRegistrationPolicy, "open")
        .await
        .unwrap();

    let (status, set_cookie, _) = post_form_with_secure_flag(
        &state,
        <web::registration::Register as ServerFn>::PATH,
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
