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
use common::auth::{load_registration_policy, RegistrationPolicy};
#[cfg(feature = "ssr")]
use common::password::Password;
#[cfg(feature = "ssr")]
use common::storage::AppState;
#[cfg(feature = "ssr")]
use common::username::Username;
#[cfg(feature = "ssr")]
use std::sync::Arc;
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
    Session(common::storage::SessionAuthError),
}

#[cfg(feature = "ssr")]
impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        match self {
            AuthRejection::MissingAppState
            | AuthRejection::Session(common::storage::SessionAuthError::Internal(_)) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            AuthRejection::MissingToken
            | AuthRejection::Session(common::storage::SessionAuthError::InvalidToken)
            | AuthRejection::Session(common::storage::SessionAuthError::SessionNotFound) => {
                StatusCode::UNAUTHORIZED
            }
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

/// Extracts the authenticated user inside a Leptos server function.
/// Returns an internal auth error when no valid session is present.
#[cfg(feature = "ssr")]
#[tracing::instrument(name = "web.auth.require_auth")]
pub async fn require_auth() -> InternalResult<AuthUser> {
    let mut parts = leptos::context::use_context::<Parts>().ok_or_else(|| {
        InternalError::server_message("missing request Parts context in require_auth")
    })?;

    AuthUser::from_request_parts(&mut parts, &())
        .await
        .map_err(auth_rejection_error)
}

#[cfg(feature = "ssr")]
fn auth_rejection_error(error: AuthRejection) -> InternalError {
    match error {
        AuthRejection::MissingToken => InternalError::unauthorized("missing session token"),
        AuthRejection::MissingAppState => InternalError::server_message("missing AppState context"),
        AuthRejection::Session(common::storage::SessionAuthError::InvalidToken) => {
            InternalError::unauthorized("invalid session token")
        }
        AuthRejection::Session(common::storage::SessionAuthError::SessionNotFound) => {
            InternalError::unauthorized("session not found")
        }
        AuthRejection::Session(common::storage::SessionAuthError::Internal(error)) => {
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

    let secure_attr = if use_context::<CookieSettings>()
        .map(|settings| settings.secure)
        .unwrap_or(true)
    {
        "; Secure"
    } else {
        ""
    };

    if let Some(opts) = use_context::<ResponseOptions>() {
        let cookie = format!(
            "session={}; HttpOnly; SameSite=Lax; Path=/{}",
            raw_token, secure_attr
        );
        if let Ok(val) = axum::http::HeaderValue::from_str(&cookie) {
            opts.insert_header(axum::http::header::SET_COOKIE, val);
        }
    }
}

#[cfg(feature = "ssr")]
fn clear_session_cookie() {
    use leptos::context::use_context;
    use leptos_axum::ResponseOptions;

    let secure_attr = if use_context::<CookieSettings>()
        .map(|settings| settings.secure)
        .unwrap_or(true)
    {
        "; Secure"
    } else {
        ""
    };

    if let Some(opts) = use_context::<ResponseOptions>() {
        let cookie = format!(
            "session=; HttpOnly; SameSite=Lax; Path=/{}; Max-Age=0",
            secure_attr
        );
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
        let state = expect_context::<Arc<AppState>>();
        let policy = load_registration_policy(&*state.site_config).await;
        Ok(policy.to_string())
    })
}

/// Returns the current logged-in username, if any.
#[server(endpoint = "/current_user")]
#[cfg_attr(feature = "ssr", tracing::instrument(name = "web.auth.current_user"))]
pub async fn current_user() -> WebResult<Option<String>> {
    crate::web_server_fn!("current_user", => {
        match require_auth().await {
            Ok(auth) => Ok(Some(auth.username.to_string())),
            Err(error) if matches!(error.public(), crate::error::WebError::Unauthorized) => Ok(None),
            Err(error) => Err(error),
        }
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
        let state = expect_context::<Arc<AppState>>();
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
        let policy = load_registration_policy(&*state.site_config)
            .instrument(tracing::info_span!(
                "web.auth.register.load_registration_policy"
            ))
            .await;

        let user_id = match policy {
            RegistrationPolicy::Open => state
                .users
                .create_user(&username, &password, None)
                .instrument(tracing::info_span!("web.auth.register.create_user_open"))
                .await
                .map_err(register_open_error)?,
            RegistrationPolicy::InviteOnly => {
                let code = invite_code
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| InternalError::validation("invite code required"))?;
                state
                    .atomic
                    .create_user_with_invite(&username, &password, None, &code)
                    .instrument(tracing::info_span!("web.auth.register.create_user_invite"))
                    .await
                    .map_err(register_invite_error)?
            }
            RegistrationPolicy::Closed => {
                return Err(InternalError::validation("registration is closed"));
            }
        };

        let raw_token = state
            .sessions
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
        let state = expect_context::<Arc<AppState>>();
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
        let record = state
            .users
            .authenticate(&username, &password)
            .instrument(tracing::info_span!("web.auth.login.authenticate_user"))
            .await
            .map_err(login_error)?;

        let label = label.filter(|s| !s.is_empty());
        let raw_token = state
            .sessions
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
            let state = expect_context::<Arc<AppState>>();
            state
                .sessions
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
fn register_open_error(error: common::storage::CreateUserError) -> InternalError {
    match error {
        common::storage::CreateUserError::UsernameTaken => {
            InternalError::conflict("username is already taken")
        }
        common::storage::CreateUserError::Internal(error) => InternalError::storage(error),
    }
}

#[cfg(feature = "ssr")]
fn register_invite_error(error: common::storage::RegisterWithInviteError) -> InternalError {
    match error {
        common::storage::RegisterWithInviteError::UsernameTaken => {
            InternalError::conflict("username is already taken")
        }
        common::storage::RegisterWithInviteError::InviteNotFound => {
            InternalError::validation("invite code not found")
        }
        common::storage::RegisterWithInviteError::InviteExpired => {
            InternalError::validation("invite code has expired")
        }
        common::storage::RegisterWithInviteError::InviteAlreadyUsed => {
            InternalError::validation("invite code has already been used")
        }
        common::storage::RegisterWithInviteError::Internal(error) => InternalError::storage(error),
    }
}

#[cfg(feature = "ssr")]
fn login_error(error: common::storage::UserAuthError) -> InternalError {
    match error {
        common::storage::UserAuthError::InvalidCredentials => {
            InternalError::unauthorized("invalid credentials")
        }
        common::storage::UserAuthError::Internal(message) => InternalError::server_message(message),
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
}
