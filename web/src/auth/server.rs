use crate::error::{InternalError, InternalResult};
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use common::username::Username;
use host::auth::resolve_credential;
use std::sync::Arc;
use storage::SessionStorage;

// `CookieSettings` now lives in `host` (pure config data); re-export it so the
// long-standing `web::auth::CookieSettings` path (the `server` crate provides it
// into leptos context) keeps resolving.
pub use host::auth::CookieSettings;

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
                common::metrics::session_validation(common::metrics::SessionOutcome::Ok);
                verify_basic_username(&record.username, credential.expected_username.as_deref())?;
                Ok(AuthUser {
                    user_id: record.user_id,
                    username: record.username,
                    token_hash: record.token_hash,
                })
            }
            Err(error) => {
                common::metrics::session_validation(storage::session_outcome(&error));
                Err(AuthRejection::Session(error))
            }
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
// Basic-auth username check (thin AuthRejection wrapper over common's core)
// ---------------------------------------------------------------------------

/// Verifies that an app-password (Basic auth) request authenticated as the
/// user it claimed. Cookie/Bearer requests carry no expected username and
/// always pass. The pure comparison lives in `common::auth`; this wrapper only
/// lifts it into the web-local [`AuthRejection`] result type.
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
        Some(expected) if !common::auth::basic_username_matches(authenticated, expected) => {
            Err(AuthRejection::BasicUsernameMismatch)
        }
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Cookie helpers (leptos/axum adapters over host's pure header builders)
// ---------------------------------------------------------------------------

pub fn set_session_cookie(raw_token: &str) {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;

    let secure = use_context::<CookieSettings>().is_none_or(|settings| settings.secure);

    if let Some(opts) = use_context::<ResponseOptions>() {
        let cookie = host::auth::session_cookie_header(raw_token, secure);
        if let Ok(val) = axum::http::HeaderValue::from_str(&cookie) {
            opts.insert_header(axum::http::header::SET_COOKIE, val);
        }
    }
}

pub fn clear_session_cookie() {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;

    let secure = use_context::<CookieSettings>().is_none_or(|settings| settings.secure);

    if let Some(opts) = use_context::<ResponseOptions>() {
        let cookie = host::auth::clear_session_cookie_header(secure);
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
        Err(error) if error.kind() == crate::error::ErrorKind::Auth => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use leptos::prelude::{provide_context, Owner};

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
            crate::error::project(missing_token.kind(), missing_token.public_message()),
            crate::error::WebError::Unauthorized
        ));

        let missing_state = auth_rejection_error(AuthRejection::MissingSessionStorage);
        assert!(matches!(
            crate::error::project(missing_state.kind(), missing_state.public_message()),
            crate::error::WebError::Server { .. }
        ));

        let basic_mismatch = auth_rejection_error(AuthRejection::BasicUsernameMismatch);
        assert!(matches!(
            crate::error::project(basic_mismatch.kind(), basic_mismatch.public_message()),
            crate::error::WebError::Unauthorized
        ));

        let invalid = auth_rejection_error(AuthRejection::Session(
            storage::SessionAuthError::InvalidToken,
        ));
        assert!(matches!(
            crate::error::project(invalid.kind(), invalid.public_message()),
            crate::error::WebError::Unauthorized
        ));

        let not_found = auth_rejection_error(AuthRejection::Session(
            storage::SessionAuthError::SessionNotFound,
        ));
        assert!(matches!(
            crate::error::project(not_found.kind(), not_found.public_message()),
            crate::error::WebError::Unauthorized
        ));

        let internal = auth_rejection_error(AuthRejection::Session(
            storage::SessionAuthError::Internal(sqlx::Error::PoolClosed),
        ));
        assert!(matches!(
            crate::error::project(internal.kind(), internal.public_message()),
            crate::error::WebError::Storage { .. }
        ));
    }

    #[tokio::test]
    async fn require_auth_with_parts_returns_server_error_when_parts_missing() {
        let e = require_auth_with_parts(None).await.unwrap_err();
        assert!(matches!(
            crate::error::project(e.kind(), e.public_message()),
            crate::error::WebError::Server { .. }
        ));
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
        // Non-Auth errors propagate unchanged; assert on the carried `kind` (the wire
        // projection is covered by `project`'s own test). A nested `matches!` guard
        // here would leave its no-match arm uncovered.
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), crate::error::ErrorKind::Storage);
    }

    // Pure extractor unit test: with a session cookie but no SessionStorage in the
    // request extensions, `AuthUser` extraction rejects with MissingSessionStorage.
    // Touches no router and no database.
    #[tokio::test]
    async fn auth_user_extraction_fails_without_session_storage_extension() {
        use axum::body::Body;
        use axum::http::{header, Request};

        let request = Request::builder()
            .header(header::COOKIE, "session=some-token")
            .body(Body::empty())
            .unwrap();
        let (mut parts, _) = request.into_parts();

        // Attempt to extract AuthUser without the session store in extensions
        let result = AuthUser::from_request_parts(&mut parts, &()).await;
        assert!(matches!(
            result.unwrap_err(),
            AuthRejection::MissingSessionStorage
        ));
    }
}
