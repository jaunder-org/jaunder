use crate::error::{InternalError, InternalResult};
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use common::username::Username;
use std::sync::Arc;
use storage::AppState;

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
    MissingAppState,
    Session(storage::SessionAuthError),
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        match self {
            AuthRejection::MissingAppState
            | AuthRejection::Session(storage::SessionAuthError::Internal(_)) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AuthRejection::MissingToken
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
        // Cookie takes precedence over Authorization header.
        let raw_token = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| {
                s.split(';')
                    .map(str::trim)
                    .find(|c| c.starts_with("session="))
                    .and_then(|c| c.strip_prefix("session="))
                    .map(str::to_string)
            })
            .or_else(|| {
                parts
                    .headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer "))
                    .map(str::to_string)
            });

        let raw_token = raw_token.ok_or(AuthRejection::MissingToken)?;

        let state = parts
            .extensions
            .get::<Arc<AppState>>()
            .ok_or(AuthRejection::MissingAppState)?;

        match state.sessions.authenticate(&raw_token).await {
            Ok(record) => Ok(AuthUser {
                user_id: record.user_id,
                username: record.username,
                token_hash: record.token_hash,
            }),
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
        AuthRejection::MissingAppState => InternalError::server_message("missing AppState context"),
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
        storage::UserAuthError::Internal(message) => InternalError::server_message(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use leptos::prelude::{provide_context, Owner};

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
    fn auth_rejection_into_response_renders_500_for_missing_app_state() {
        let response = AuthRejection::MissingAppState.into_response();
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

        let missing_state = auth_rejection_error(AuthRejection::MissingAppState);
        assert!(matches!(
            missing_state.public(),
            crate::error::WebError::Server { .. }
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
        let err = login_error(storage::UserAuthError::Internal("db crashed".to_string()));
        assert!(matches!(
            err.public(),
            crate::error::WebError::Server { .. }
        ));
        assert_eq!(err.operator_message(), "db crashed");

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
