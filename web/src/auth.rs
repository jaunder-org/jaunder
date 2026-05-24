use crate::error::WebResult;
#[cfg(feature = "ssr")]
use crate::error::{InternalError, InternalResult};
#[cfg(feature = "ssr")]
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
#[cfg(feature = "ssr")]
use common::password::Password;
#[cfg(feature = "ssr")]
use common::username::Username;
#[cfg(feature = "ssr")]
use std::sync::Arc;
#[cfg(feature = "ssr")]
use storage::{
    load_registration_policy, AppState, AtomicOps, RegistrationPolicy, SessionStorage,
    SiteConfigStorage, UserStorage,
};
#[cfg(feature = "ssr")]
use tracing::Instrument;

/// Cookie settings derived from the public deployment scheme.
#[cfg(feature = "ssr")]
#[derive(Clone, Copy)]
pub struct CookieSettings {
    pub secure: bool,
}

// ---------------------------------------------------------------------------
// AuthUser
// ---------------------------------------------------------------------------

/// The authenticated user extracted from a valid session cookie or Bearer token.
#[cfg(feature = "ssr")]
#[derive(Debug)]
pub struct AuthUser {
    pub user_id: i64,
    pub username: Username,
    pub token_hash: String,
}

#[cfg(feature = "ssr")]
#[derive(Debug)]
pub enum AuthRejection {
    MissingToken,
    MissingAppState,
    Session(storage::SessionAuthError),
}

#[cfg(feature = "ssr")]
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

#[cfg(feature = "ssr")]
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
#[cfg(feature = "ssr")]
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
#[cfg(feature = "ssr")]
#[tracing::instrument(name = "web.auth.require_auth")]
pub async fn require_auth() -> InternalResult<AuthUser> {
    require_auth_with_parts(leptos::context::use_context::<Parts>()).await
}

#[cfg(feature = "ssr")]
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

#[cfg(feature = "ssr")]
fn set_session_cookie(raw_token: &str) {
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

#[cfg(feature = "ssr")]
fn clear_session_cookie() {
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
// Server functions
// ---------------------------------------------------------------------------

use leptos::prelude::*;

/// Returns the site's current registration policy as a string.
/// Possible values: `"open"`, `"invite_only"`, `"closed"`.
#[server(endpoint = "/get_registration_policy")]
#[cfg_attr(
    feature = "ssr",
    tracing::instrument(name = "web.auth.get_registration_policy")
)]
pub async fn get_registration_policy() -> WebResult<String> {
    crate::web_server_fn!("get_registration_policy", => {
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let policy = load_registration_policy(&*site_config).await;
        Ok(policy.to_string())
    })
}

/// Maps a `require_auth` result to the `current_user` response shape:
/// `Ok` → username, `Unauthorized` error → `None`, other errors propagate.
#[cfg(feature = "ssr")]
fn classify_current_user(result: InternalResult<AuthUser>) -> InternalResult<Option<String>> {
    match result {
        Ok(auth) => Ok(Some(auth.username.to_string())),
        Err(error) if matches!(error.public(), crate::error::WebError::Unauthorized) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Returns the current logged-in username, if any.
#[server(endpoint = "/current_user")]
#[cfg_attr(feature = "ssr", tracing::instrument(name = "web.auth.current_user"))]
pub async fn current_user() -> WebResult<Option<String>> {
    crate::web_server_fn!("current_user", => {
        classify_current_user(require_auth().await)
    })
}

/// Registers a new user.  Returns the raw session token on success and sets
/// the `session` cookie.
#[server(endpoint = "/register")]
#[cfg_attr(
    feature = "ssr",
    tracing::instrument(name = "web.auth.register", skip(password, invite_code))
)]
pub async fn register(
    username: String,
    password: String,
    invite_code: Option<String>,
) -> WebResult<String> {
    crate::web_server_fn!("register", username, password, invite_code => {
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();
        let atomic = expect_context::<Arc<dyn AtomicOps>>();
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        let username = {
            let _phase = tracing::info_span!("web.auth.register.parse_username").entered();
            username
                .to_lowercase()
                .parse::<Username>()
                .map_err(|e| InternalError::validation(e.to_string()))?
        };
        let password = {
            let _phase = tracing::info_span!("web.auth.register.parse_password").entered();
            password
                .parse::<Password>()
                .map_err(|e| InternalError::validation(e.to_string()))?
        };
        let policy = load_registration_policy(&*site_config)
            .instrument(tracing::info_span!(
                "web.auth.register.load_registration_policy"
            ))
            .await;

        let user_id = match policy {
            RegistrationPolicy::Open => users
                .create_user(&username, &password, None, false)
                .instrument(tracing::info_span!("web.auth.register.create_user_open"))
                .await
                .map_err(register_open_error)?,
            RegistrationPolicy::InviteOnly => {
                let code = invite_code
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| InternalError::validation("invite code required"))?;
                atomic
                    .create_user_with_invite(&username, &password, None, false, &code)
                    .instrument(tracing::info_span!("web.auth.register.create_user_invite"))
                    .await
                    .map_err(register_invite_error)?
            }
            RegistrationPolicy::Closed => {
                return Err(InternalError::validation("registration is closed"));
            }
        };

        let raw_token = sessions
            .create_session(user_id, None)
            .instrument(tracing::info_span!("web.auth.register.create_session"))
            .await
            .map_err(InternalError::storage)?;

        set_session_cookie(&raw_token);
        leptos_axum::redirect("/");
        Ok(raw_token)
    })
}

/// Authenticates a user.  Returns the raw session token on success and sets
/// the `session` cookie.
#[server(endpoint = "/login")]
#[cfg_attr(
    feature = "ssr",
    tracing::instrument(name = "web.auth.login", skip(password, label))
)]
pub async fn login(username: String, password: String, label: Option<String>) -> WebResult<String> {
    crate::web_server_fn!("login", username, password, label => {
        let users = expect_context::<Arc<dyn UserStorage>>();
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        let username = {
            let _phase = tracing::info_span!("web.auth.login.parse_username").entered();
            username
                .to_lowercase()
                .parse::<Username>()
                .map_err(|e| InternalError::validation(e.to_string()))?
        };
        let password = {
            let _phase = tracing::info_span!("web.auth.login.parse_password").entered();
            password
                .parse::<Password>()
                .map_err(|e| InternalError::validation(e.to_string()))?
        };
        let record = users
            .authenticate(&username, &password)
            .instrument(tracing::info_span!("web.auth.login.authenticate_user"))
            .await
            .map_err(login_error)?;

        let label = label.filter(|s| !s.is_empty());
        let raw_token = sessions
            .create_session(record.user_id, label.as_deref())
            .instrument(tracing::info_span!("web.auth.login.create_session"))
            .await
            .map_err(InternalError::storage)?;

        set_session_cookie(&raw_token);
        leptos_axum::redirect("/");
        Ok(raw_token)
    })
}

/// Revokes the current session and clears the `session` cookie.
#[server(endpoint = "/logout")]
#[cfg_attr(feature = "ssr", tracing::instrument(name = "web.auth.logout"))]
pub async fn logout() -> WebResult<()> {
    crate::web_server_fn!("logout", => {
        if let Ok(auth) = require_auth().await {
            let sessions = expect_context::<Arc<dyn SessionStorage>>();
            sessions
                .revoke_session(&auth.token_hash)
                .await
                .map_err(InternalError::storage)?;
        }
        clear_session_cookie();
        leptos_axum::redirect("/");
        Ok(())
    })
}

#[cfg(feature = "ssr")]
fn register_open_error(error: storage::CreateUserError) -> InternalError {
    match error {
        storage::CreateUserError::UsernameTaken => {
            InternalError::conflict("username is already taken")
        }
        storage::CreateUserError::Internal(error) => InternalError::storage(error),
    }
}

#[cfg(feature = "ssr")]
fn register_invite_error(error: storage::RegisterWithInviteError) -> InternalError {
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

#[cfg(feature = "ssr")]
fn login_error(error: storage::UserAuthError) -> InternalError {
    match error {
        storage::UserAuthError::InvalidCredentials => {
            InternalError::unauthorized("invalid credentials")
        }
        storage::UserAuthError::Internal(message) => InternalError::server_message(message),
    }
}

#[cfg(all(test, feature = "ssr"))]
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
