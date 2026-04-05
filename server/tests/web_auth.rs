use std::sync::{Arc, OnceLock};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::Utc;
use leptos::prelude::LeptosOptions;
use server::storage::{open_database, DbConnectOptions};
use server::username::Username;
use tempfile::TempDir;
use tower::ServiceExt;

/// Explicitly register server functions once per test binary.
/// Inventory-based auto-registration is unreliable in Cargo test builds with
/// multiple codegen-units, so we register explicitly instead.
fn ensure_server_fns_registered() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        server_fn::axum::register_explicit::<web::auth::GetRegistrationPolicy>();
        server_fn::axum::register_explicit::<web::auth::Register>();
        server_fn::axum::register_explicit::<web::auth::Login>();
        server_fn::axum::register_explicit::<web::auth::Logout>();
    });
}

fn db_url(base: &TempDir) -> DbConnectOptions {
    format!("sqlite:{}", base.path().join("test.db").display())
        .parse()
        .unwrap()
}

async fn test_state(base: &TempDir) -> Arc<server::storage::AppState> {
    open_database(&db_url(base)).await.unwrap()
}

fn test_options() -> LeptosOptions {
    LeptosOptions::builder().output_name("test").build()
}

/// Sends a form-encoded POST request through a fresh router built from `state`.
/// Returns (status, Set-Cookie header value, response body).
async fn post_form(
    state: Arc<server::storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, Option<String>, String) {
    ensure_server_fns_registered();

    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder.body(Body::from(body.into())).unwrap();

    let app = server::create_router(test_options(), state);
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let set_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8(bytes.to_vec()).unwrap();

    (status, set_cookie, body_str)
}

/// Sends a form-encoded POST request through a fresh router built from `state`,
/// attaching an `Authorization: Bearer <token>` header instead of a cookie.
/// Returns (status, Set-Cookie header value, response body).
async fn post_form_with_bearer(
    state: Arc<server::storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    bearer_token: &str,
) -> (StatusCode, Option<String>, String) {
    ensure_server_fns_registered();

    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::AUTHORIZATION, format!("Bearer {bearer_token}"))
        .body(Body::from(body.into()))
        .expect("failed to build request with bearer token");

    let app = server::create_router(test_options(), state);
    let response = app.oneshot(request).await.expect("router oneshot failed");

    let status = response.status();
    let set_cookie = response.headers().get(header::SET_COOKIE).map(|v| {
        v.to_str()
            .expect("Set-Cookie header is not valid UTF-8")
            .to_string()
    });
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_str = String::from_utf8(bytes.to_vec()).expect("response body is not valid UTF-8");

    (status, set_cookie, body_str)
}

/// Extracts a raw token from a server-function JSON response body.
/// Successful server functions return a JSON string: `"<token>"`.
fn extract_token(body: &str) -> String {
    let trimmed = body.trim();
    assert!(
        trimmed.starts_with('"') && trimmed.ends_with('"'),
        "expected JSON string in body, got: {trimmed}"
    );
    trimmed[1..trimmed.len() - 1].to_string()
}

// M2.9.8: `register` with Open policy creates user, sets cookie, returns non-empty token.
#[tokio::test]
async fn register_open_creates_user_sets_cookie_returns_token() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();

    let (status, set_cookie, body) = post_form(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=password123",
        None,
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

#[tokio::test]
async fn register_duplicate_username_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();

    // Register alice once.
    post_form(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=password123",
        None,
    )
    .await;

    // Register alice again.
    let (status, _, _) = post_form(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=otherpassword",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// M2.9.9: `register` with InviteOnly + valid code creates user, marks invite used, returns token.
#[tokio::test]
async fn register_invite_only_valid_code_creates_user_marks_invite_used() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();
    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let (status, _set_cookie, body) = post_form(
        Arc::clone(&state),
        "/api/register",
        format!("username=bob&password=password123&invite_code={code}"),
        None,
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
    let invite = invites.iter().find(|i| i.code == code).unwrap();
    assert!(invite.used_at.is_some(), "invite should be marked as used");
}

// M2.9.10: `register` with InviteOnly policy and missing code returns error.
#[tokio::test]
async fn register_invite_only_missing_code_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();

    let (status, _set_cookie, _body) = post_form(
        Arc::clone(&state),
        "/api/register",
        "username=carol&password=password123",
        None,
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

// M2.9.11: `register` with Closed policy returns error.
#[tokio::test]
async fn register_closed_policy_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    // Default policy is Closed; no need to set it explicitly.

    let (status, _set_cookie, _body) = post_form(
        Arc::clone(&state),
        "/api/register",
        "username=dave&password=password123",
        None,
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
#[tokio::test]
async fn login_correct_password_sets_cookie_and_returns_token() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    // Register user first.
    post_form(
        Arc::clone(&state),
        "/api/register",
        "username=eve&password=password123",
        None,
    )
    .await;

    let (status, set_cookie, body) = post_form(
        Arc::clone(&state),
        "/api/login",
        "username=eve&password=password123",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let token = extract_token(&body);
    assert!(!token.is_empty());
    let cookie = set_cookie.expect("Set-Cookie header should be present on login");
    assert!(cookie.starts_with("session="), "cookie: {cookie}");
}

#[tokio::test]
async fn login_unknown_user_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    let (status, _, _) = post_form(
        Arc::clone(&state),
        "/api/login",
        "username=nobody&password=password123",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn login_with_label_creates_session_with_label() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    // Register.
    post_form(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=password123",
        None,
    )
    .await;

    // Login with label.
    let (status, _, body) = post_form(
        Arc::clone(&state),
        "/api/login",
        "username=alice&password=password123&label=my-device",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = extract_token(&body);
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.label.as_deref(), Some("my-device"));
}

#[tokio::test]
async fn login_with_empty_label_creates_session_without_label() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=password123",
        None,
    )
    .await;

    // Login with empty label.
    let (status, _, body) = post_form(
        Arc::clone(&state),
        "/api/login",
        "username=alice&password=password123&label=",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let raw_token = extract_token(&body);
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert!(record.label.is_none());
}

// M2.9.13: `login` with wrong password returns error.
#[tokio::test]
async fn login_wrong_password_returns_error() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .unwrap();
    post_form(
        Arc::clone(&state),
        "/api/register",
        "username=frank&password=correctpassword",
        None,
    )
    .await;

    let (status, _set_cookie, _body) = post_form(
        Arc::clone(&state),
        "/api/login",
        "username=frank&password=wrongpassword",
        None,
    )
    .await;

    assert_ne!(status, StatusCode::OK);
}

// M2.9.14: `logout` revokes session and clears cookie.
#[tokio::test]
async fn logout_revokes_session_and_clears_cookie() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    // Create a user and a session directly, bypassing the HTTP layer so we
    // have the raw token without needing to parse the register response.
    let user_id = state
        .users
        .create_user(
            &"grace".parse::<Username>().unwrap(),
            &"password123".parse().unwrap(),
            None,
        )
        .await
        .unwrap();
    let raw_token = state.sessions.create_session(user_id, None).await.unwrap();

    let sessions_before = state.sessions.list_sessions(user_id).await.unwrap();
    assert_eq!(
        sessions_before.len(),
        1,
        "one session should exist before logout"
    );

    let cookie_header = format!("session={raw_token}");
    let (status, set_cookie, _body) =
        post_form(Arc::clone(&state), "/api/logout", "", Some(&cookie_header)).await;

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
#[tokio::test]
async fn register_invalid_username_returns_error() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .expect("failed to set registration policy");

    // "alice doe" lowercases to "alice doe" which fails Username parse
    // because Username only allows [a-z0-9_-]+.
    let (status, _set_cookie, _body) = post_form(
        Arc::clone(&state),
        "/api/register",
        "username=alice%20doe&password=password123",
        None,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "register with space in username should fail"
    );
}

// register() with a password shorter than 8 characters returns error and creates no user.
#[tokio::test]
async fn register_short_password_returns_error() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "open")
        .await
        .expect("failed to set registration policy");

    let (status, _set_cookie, _body) = post_form(
        Arc::clone(&state),
        "/api/register",
        "username=alice&password=short",
        None,
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
#[tokio::test]
async fn login_invalid_username_returns_error() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;

    let (status, _set_cookie, _body) = post_form(
        Arc::clone(&state),
        "/api/login",
        "username=alice%20doe&password=password123",
        None,
    )
    .await;

    assert_ne!(
        status,
        StatusCode::OK,
        "login with space in username should fail"
    );
}

// logout() via Authorization: Bearer <token> revokes the session and clears the cookie.
#[tokio::test]
async fn logout_with_bearer_token_revokes_session() {
    let base = TempDir::new().expect("failed to create temp dir");
    let state = test_state(&base).await;

    // Create a user and session directly so we have the raw token.
    let user_id = state
        .users
        .create_user(
            &"henry".parse::<Username>().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
        )
        .await
        .expect("failed to create user");
    let raw_token = state
        .sessions
        .create_session(user_id, None)
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
        post_form_with_bearer(Arc::clone(&state), "/api/logout", "", &raw_token).await;

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

#[tokio::test]
async fn debug_api_routes_exist() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    // Send a request with no body to /api/register — if route exists we get
    // something other than 404 (probably a 400/422 for missing fields).
    let (status, _, _) = post_form(Arc::clone(&state), "/api/register", "", None).await;
    assert_ne!(
        status,
        StatusCode::NOT_FOUND,
        "/api/register route not registered (got 404)"
    );
}

#[tokio::test]
async fn get_registration_policy_returns_correct_value() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;
    state
        .site_config
        .set("site.registration_policy", "invite_only")
        .await
        .unwrap();

    // Server functions are POST by default.
    let (status, _, body) =
        post_form(Arc::clone(&state), "/api/get_registration_policy", "", None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "\"invite_only\"");
}

#[tokio::test]
async fn auth_user_extraction_fails_with_invalid_token() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    // logout() requires AuthUser. If we provide an invalid token, it should fail.
    let cookie_header = "session=invalidtoken";
    let (status, _, _) =
        post_form(Arc::clone(&state), "/api/logout", "", Some(&cookie_header)).await;

    // Leptos server functions return 500 for ServerFnError.
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn auth_user_extraction_fails_when_missing() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;

    // logout() requires AuthUser. If we provide no token, it should fail.
    let (status, _, _) = post_form(Arc::clone(&state), "/api/logout", "", None).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
