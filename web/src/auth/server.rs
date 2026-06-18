use crate::error::{InternalError, InternalResult};
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use common::username::Username;
use std::sync::Arc;
use storage::SessionStorage;

/// Cookie settings derived from the public deployment scheme.
#[derive(Clone, Copy)]
pub struct CookieSettings {
    pub secure: bool,
}

// ---------------------------------------------------------------------------
// AuthUser
// ---------------------------------------------------------------------------

/// The authenticated user extracted from a valid session cookie or Bearer token.
#[derive(Debug)]
pub struct AuthUser {
    pub user_id: i64,
    pub username: Username,
    pub token_hash: String,
}

#[derive(Debug)]
pub enum AuthRejection {
    MissingToken,
    MissingSessionStorage,
    Session(storage::SessionAuthError),
    BasicUsernameMismatch,
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        match self {
            AuthRejection::MissingSessionStorage
            | AuthRejection::Session(storage::SessionAuthError::Internal(_)) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AuthRejection::MissingToken
            | AuthRejection::BasicUsernameMismatch
            | AuthRejection::Session(
                storage::SessionAuthError::InvalidToken
                | storage::SessionAuthError::SessionNotFound,
            ) => StatusCode::UNAUTHORIZED,
        }
        .into_response()
    }
}

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let credential = resolve_credential(&parts.headers).ok_or(AuthRejection::MissingToken)?;

        let sessions = parts
            .extensions
            .get::<Arc<dyn SessionStorage>>()
            .ok_or(AuthRejection::MissingSessionStorage)?;

        match sessions.authenticate(&credential.token).await {
            Ok(record) => {
                verify_basic_username(&record.username, credential.expected_username.as_deref())?;
                Ok(AuthUser {
                    user_id: record.user_id,
                    username: record.username,
                    token_hash: record.token_hash,
                })
            }
            Err(error) => Err(AuthRejection::Session(error)),
        }
    }
}

/// Inner implementation of [`require_auth`] — accepts the already-extracted
/// `Parts` option so pure unit tests can exercise the missing-context path.
///
/// # Errors
///
/// Returns `Err` if `parts` is `None` (missing Leptos request context) or if
/// the session token is absent, invalid, or not found in storage.
pub async fn require_auth_with_parts(parts: Option<Parts>) -> InternalResult<AuthUser> {
    let mut parts = parts.ok_or_else(|| {
        InternalError::server_message("missing request Parts context in require_auth")
    })?;
    AuthUser::from_request_parts(&mut parts, &())
        .await
        .map_err(auth_rejection_error)
}

/// Extracts the authenticated user inside a Leptos server function.
/// Returns an internal auth error when no valid session is present.
///
/// # Errors
///
/// Returns `Err` if the request is not authenticated (missing or invalid session token).
#[tracing::instrument(name = "web.auth.require_auth")]
pub async fn require_auth() -> InternalResult<AuthUser> {
    require_auth_with_parts(leptos::context::use_context::<Parts>()).await
}

fn auth_rejection_error(error: AuthRejection) -> InternalError {
    match error {
        AuthRejection::MissingToken => InternalError::unauthorized("missing session token"),
        AuthRejection::MissingSessionStorage => {
            InternalError::server_message("missing SessionStorage context")
        }
        AuthRejection::BasicUsernameMismatch => {
            InternalError::unauthorized("basic auth username mismatch")
        }
        AuthRejection::Session(storage::SessionAuthError::InvalidToken) => {
            InternalError::unauthorized("invalid session token")
        }
        AuthRejection::Session(storage::SessionAuthError::SessionNotFound) => {
            InternalError::unauthorized("session not found")
        }
        AuthRejection::Session(storage::SessionAuthError::Internal(error)) => {
            InternalError::storage(error)
        }
    }
}

// ---------------------------------------------------------------------------
// Credential resolution
// ---------------------------------------------------------------------------

/// A session token resolved from request headers, plus — for HTTP Basic auth —
/// the username the authenticated session must belong to.
struct Credential {
    /// The raw session token to authenticate.
    token: String,
    /// For Basic auth, the username supplied alongside the token, which must
    /// match the authenticated session's user. `None` for cookie/Bearer auth.
    expected_username: Option<String>,
}

/// Resolves the session credential from request headers.
///
/// Precedence: the `session=` cookie, then `Authorization: Bearer <token>`,
/// then `Authorization: Basic <base64(user:token)>` (app passwords). Returns
/// `None` when no recognized credential is present.
fn resolve_credential(headers: &axum::http::HeaderMap) -> Option<Credential> {
    let from_cookie = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .map(str::trim)
                .find_map(|c| c.strip_prefix("session="))
                .map(|token| Credential {
                    token: token.to_string(),
                    expected_username: None,
                })
        });
    if from_cookie.is_some() {
        return from_cookie;
    }

    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())?;
    if let Some(token) = header.strip_prefix("Bearer ") {
        Some(Credential {
            token: token.to_string(),
            expected_username: None,
        })
    } else {
        let (username, password) = parse_basic_auth(header)?;
        Some(Credential {
            token: password,
            expected_username: Some(username),
        })
    }
}

/// Verifies that an app-password (Basic auth) request authenticated as the
/// user it claimed. Cookie/Bearer requests carry no expected username and
/// always pass.
///
/// # Errors
///
/// Returns [`AuthRejection::BasicUsernameMismatch`] when the Basic username
/// does not match the authenticated session's user.
fn verify_basic_username(
    authenticated: &Username,
    expected: Option<&str>,
) -> Result<(), AuthRejection> {
    match expected {
        Some(expected) if authenticated.as_str() != expected => {
            Err(AuthRejection::BasicUsernameMismatch)
        }
        _ => Ok(()),
    }
}

/// Parses an HTTP `Authorization: Basic` header value into `(username, password)`.
/// Returns `None` for non-Basic schemes or malformed/undecodable credentials.
fn parse_basic_auth(header: &str) -> Option<(String, String)> {
    use base64::Engine as _;

    let rest = header.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(rest)
        .ok()?;
    let credentials = String::from_utf8(decoded).ok()?;
    let (username, password) = credentials.split_once(':')?;
    Some((username.to_string(), password.to_string()))
}

// ---------------------------------------------------------------------------
// Cookie helpers
// ---------------------------------------------------------------------------

pub fn set_session_cookie(raw_token: &str) {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;

    let secure_attr = if use_context::<CookieSettings>().is_none_or(|settings| settings.secure) {
        "; Secure"
    } else {
        ""
    };

    if let Some(opts) = use_context::<ResponseOptions>() {
        let cookie = format!("session={raw_token}; HttpOnly; SameSite=Lax; Path=/{secure_attr}");
        if let Ok(val) = axum::http::HeaderValue::from_str(&cookie) {
            opts.insert_header(axum::http::header::SET_COOKIE, val);
        }
    }
}

pub fn clear_session_cookie() {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;

    let secure_attr = if use_context::<CookieSettings>().is_none_or(|settings| settings.secure) {
        "; Secure"
    } else {
        ""
    };

    if let Some(opts) = use_context::<ResponseOptions>() {
        let cookie = format!("session=; HttpOnly; SameSite=Lax; Path=/{secure_attr}; Max-Age=0");
        if let Ok(val) = axum::http::HeaderValue::from_str(&cookie) {
            opts.insert_header(axum::http::header::SET_COOKIE, val);
        }
    }
}

// ---------------------------------------------------------------------------
// Server function helpers
// ---------------------------------------------------------------------------

/// Maps a `require_auth` result to the `current_user` response shape:
/// `Ok` → username, `Unauthorized` error → `None`, other errors propagate.
pub fn classify_current_user(result: InternalResult<AuthUser>) -> InternalResult<Option<String>> {
    match result {
        Ok(auth) => Ok(Some(auth.username.to_string())),
        Err(error) if matches!(error.public(), crate::error::WebError::Unauthorized) => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn register_open_error(error: storage::CreateUserError) -> InternalError {
    match error {
        storage::CreateUserError::UsernameTaken => {
            InternalError::conflict("username is already taken")
        }
        storage::CreateUserError::Internal(error) => InternalError::storage(error),
    }
}

pub fn register_invite_error(error: storage::RegisterWithInviteError) -> InternalError {
    match error {
        storage::RegisterWithInviteError::UsernameTaken => {
            InternalError::conflict("username is already taken")
        }
        storage::RegisterWithInviteError::InviteNotFound => {
            InternalError::validation("invite code not found")
        }
        storage::RegisterWithInviteError::InviteExpired => {
            InternalError::validation("invite code has expired")
        }
        storage::RegisterWithInviteError::InviteAlreadyUsed => {
            InternalError::validation("invite code has already been used")
        }
        storage::RegisterWithInviteError::Internal(error) => InternalError::storage(error),
    }
}

pub fn login_error(error: storage::UserAuthError) -> InternalError {
    match error {
        storage::UserAuthError::InvalidCredentials => {
            InternalError::unauthorized("invalid credentials")
        }
        // Preserve the structured cause chain for operator logs rather than
        // eagerly flattening it to a string (InternalError carries an anyhow
        // source since kq8w.16).
        storage::UserAuthError::Internal(source) => InternalError::server_boxed(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use leptos::prelude::{provide_context, Owner};

    #[test]
    fn parse_basic_auth_decodes_credentials() {
        // base64("alice:tok123") == "YWxpY2U6dG9rMTIz"
        assert_eq!(
            parse_basic_auth("Basic YWxpY2U6dG9rMTIz"),
            Some(("alice".to_string(), "tok123".to_string()))
        );
    }

    #[test]
    fn parse_basic_auth_rejects_non_basic_and_malformed() {
        use base64::Engine as _;
        assert_eq!(parse_basic_auth("Bearer abc"), None);
        assert_eq!(parse_basic_auth("Basic !!!notbase64!!!"), None);
        // decodes but has no colon
        let no_colon = base64::engine::general_purpose::STANDARD.encode("nocolon");
        assert_eq!(parse_basic_auth(&format!("Basic {no_colon}")), None);
    }

    fn headers_with(name: axum::http::HeaderName, value: &str) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(name, axum::http::HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn resolve_credential_prefers_cookie_over_authorization() {
        let mut headers = headers_with(axum::http::header::COOKIE, "session=cookie-token");
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer bearer-token"),
        );
        let credential = resolve_credential(&headers).expect("credential");
        assert_eq!(credential.token, "cookie-token");
        assert_eq!(credential.expected_username, None);
    }

    #[test]
    fn resolve_credential_reads_bearer_token() {
        let headers = headers_with(axum::http::header::AUTHORIZATION, "Bearer bearer-token");
        let credential = resolve_credential(&headers).expect("credential");
        assert_eq!(credential.token, "bearer-token");
        assert_eq!(credential.expected_username, None);
    }

    #[test]
    fn resolve_credential_reads_basic_app_password() {
        // base64("alice:tok123") == "YWxpY2U6dG9rMTIz"
        let headers = headers_with(axum::http::header::AUTHORIZATION, "Basic YWxpY2U6dG9rMTIz");
        let credential = resolve_credential(&headers).expect("credential");
        assert_eq!(credential.token, "tok123");
        assert_eq!(credential.expected_username.as_deref(), Some("alice"));
    }

    #[test]
    fn resolve_credential_returns_none_without_recognized_header() {
        assert!(resolve_credential(&axum::http::HeaderMap::new()).is_none());
        let headers = headers_with(axum::http::header::AUTHORIZATION, "Negotiate xyz");
        assert!(resolve_credential(&headers).is_none());
    }

    #[test]
    fn verify_basic_username_passes_without_expected_username() {
        let user: Username = "alice".parse().unwrap();
        assert!(verify_basic_username(&user, None).is_ok());
    }

    #[test]
    fn verify_basic_username_passes_on_match() {
        let user: Username = "alice".parse().unwrap();
        assert!(verify_basic_username(&user, Some("alice")).is_ok());
    }

    #[test]
    fn verify_basic_username_rejects_mismatch() {
        let user: Username = "alice".parse().unwrap();
        assert!(matches!(
            verify_basic_username(&user, Some("mallory")),
            Err(AuthRejection::BasicUsernameMismatch)
        ));
    }

    #[test]
    fn set_session_cookie_without_response_options_context_is_noop() {
        Owner::new().with(|| {
            provide_context(CookieSettings { secure: true });
            set_session_cookie("token");
        });
    }

    #[test]
    fn clear_session_cookie_without_response_options_context_is_noop() {
        Owner::new().with(|| {
            provide_context(CookieSettings { secure: true });
            clear_session_cookie();
        });
    }

    #[test]
    fn auth_rejection_into_response_renders_500_for_missing_session_storage() {
        let response = AuthRejection::MissingSessionStorage.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn auth_rejection_into_response_renders_500_for_session_internal_error() {
        let response =
            AuthRejection::Session(storage::SessionAuthError::Internal(sqlx::Error::PoolClosed))
                .into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn auth_rejection_error_maps_each_variant_to_expected_internal_error() {
        let missing_token = auth_rejection_error(AuthRejection::MissingToken);
        assert!(matches!(
            missing_token.public(),
            crate::error::WebError::Unauthorized
        ));

        let missing_state = auth_rejection_error(AuthRejection::MissingSessionStorage);
        assert!(matches!(
            missing_state.public(),
            crate::error::WebError::Server { .. }
        ));

        let basic_mismatch = auth_rejection_error(AuthRejection::BasicUsernameMismatch);
        assert!(matches!(
            basic_mismatch.public(),
            crate::error::WebError::Unauthorized
        ));

        let invalid = auth_rejection_error(AuthRejection::Session(
            storage::SessionAuthError::InvalidToken,
        ));
        assert!(matches!(
            invalid.public(),
            crate::error::WebError::Unauthorized
        ));

        let not_found = auth_rejection_error(AuthRejection::Session(
            storage::SessionAuthError::SessionNotFound,
        ));
        assert!(matches!(
            not_found.public(),
            crate::error::WebError::Unauthorized
        ));

        let internal = auth_rejection_error(AuthRejection::Session(
            storage::SessionAuthError::Internal(sqlx::Error::PoolClosed),
        ));
        assert!(matches!(
            internal.public(),
            crate::error::WebError::Storage { .. }
        ));
    }

    #[test]
    fn register_open_error_maps_internal_to_storage_error() {
        let err = register_open_error(storage::CreateUserError::Internal(sqlx::Error::PoolClosed));
        assert!(matches!(
            err.public(),
            crate::error::WebError::Storage { .. }
        ));

        let conflict = register_open_error(storage::CreateUserError::UsernameTaken);
        assert!(matches!(
            conflict.public(),
            crate::error::WebError::Conflict { .. }
        ));
    }

    #[test]
    fn register_invite_error_maps_each_arm() {
        assert!(matches!(
            register_invite_error(storage::RegisterWithInviteError::UsernameTaken).public(),
            crate::error::WebError::Conflict { .. }
        ));
        assert!(matches!(
            register_invite_error(storage::RegisterWithInviteError::InviteNotFound).public(),
            crate::error::WebError::Validation { .. }
        ));
        assert!(matches!(
            register_invite_error(storage::RegisterWithInviteError::InviteExpired).public(),
            crate::error::WebError::Validation { .. }
        ));
        assert!(matches!(
            register_invite_error(storage::RegisterWithInviteError::InviteAlreadyUsed).public(),
            crate::error::WebError::Validation { .. }
        ));
        assert!(matches!(
            register_invite_error(storage::RegisterWithInviteError::Internal(
                sqlx::Error::PoolClosed
            ))
            .public(),
            crate::error::WebError::Storage { .. }
        ));
    }

    #[test]
    fn login_error_maps_internal_to_server_error() {
        use std::fmt;

        // A two-level source chain proves login_error preserves the structured
        // cause chain rather than flattening it to the top error's string.
        #[derive(Debug)]
        struct Inner;
        impl fmt::Display for Inner {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "inner cause")
            }
        }
        impl std::error::Error for Inner {}

        #[derive(Debug)]
        struct Outer(Inner);
        impl fmt::Display for Outer {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "outer failure")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        let err = login_error(storage::UserAuthError::Internal(Box::new(Outer(Inner))));
        assert!(matches!(
            err.public(),
            crate::error::WebError::Server { .. }
        ));
        // anyhow's alternate formatting joins the whole chain; the inner cause
        // would be lost if the source were flattened via `to_string()`.
        let op = err.operator_message();
        assert!(op.contains("outer failure"), "operator message: {op}");
        assert!(op.contains("inner cause"), "operator message: {op}");

        let invalid = login_error(storage::UserAuthError::InvalidCredentials);
        assert!(matches!(
            invalid.public(),
            crate::error::WebError::Unauthorized
        ));
    }

    #[tokio::test]
    async fn require_auth_with_parts_returns_server_error_when_parts_missing() {
        let result = require_auth_with_parts(None).await;
        assert!(
            matches!(result, Err(e) if matches!(e.public(), crate::error::WebError::Server { .. }))
        );
    }

    #[test]
    fn classify_current_user_returns_none_for_unauthorized_error() {
        let err = InternalError::unauthorized("test");
        let result = classify_current_user(Err(err));
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn classify_current_user_propagates_non_unauthorized_errors() {
        let err = InternalError::storage(sqlx::Error::PoolClosed);
        let result = classify_current_user(Err(err));
        assert!(
            matches!(result, Err(e) if matches!(e.public(), crate::error::WebError::Storage { .. }))
        );
    }
}
