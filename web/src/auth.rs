#[cfg(feature = "ssr")]
use crate::error::WebError;
use crate::error::WebResult;
#[cfg(feature = "ssr")]
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
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
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

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

        let raw_token = raw_token.ok_or(StatusCode::UNAUTHORIZED)?;

        let state = parts
            .extensions
            .get::<Arc<AppState>>()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

        match state.sessions.authenticate(&raw_token).await {
            Ok(record) => Ok(AuthUser {
                user_id: record.user_id,
                username: record.username,
                token_hash: record.token_hash,
            }),
            Err(_) => Err(StatusCode::UNAUTHORIZED),
        }
    }
}

/// Extracts the authenticated user inside a Leptos server function.
/// Returns [`WebError::Unauthorized`] when no valid session is present.
#[cfg(feature = "ssr")]
#[tracing::instrument(name = "web.auth.require_auth")]
pub async fn require_auth() -> WebResult<AuthUser> {
    leptos_axum::extract::<AuthUser>()
        .await
        .map_err(|_| WebError::Unauthorized)
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
    let state = expect_context::<Arc<AppState>>();
    let policy = load_registration_policy(&*state.site_config).await;
    Ok(policy.to_string())
}

/// Returns the current logged-in username, if any.
#[server(endpoint = "/current_user")]
#[cfg_attr(feature = "ssr", tracing::instrument(name = "web.auth.current_user"))]
pub async fn current_user() -> WebResult<Option<String>> {
    match require_auth().await {
        Ok(auth) => Ok(Some(auth.username.to_string())),
        Err(_) => Ok(None),
    }
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
    let state = expect_context::<Arc<AppState>>();
    let username = {
        #[cfg(feature = "ssr")]
        let _phase = tracing::info_span!("web.auth.register.parse_username").entered();
        username
            .to_lowercase()
            .parse::<Username>()
            .map_err(|e| WebError::validation(e.to_string()))?
    };
    let password = {
        #[cfg(feature = "ssr")]
        let _phase = tracing::info_span!("web.auth.register.parse_password").entered();
        password
            .parse::<Password>()
            .map_err(|e| WebError::validation(e.to_string()))?
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
                .ok_or_else(|| WebError::validation("invite code required"))?;
            state
                .atomic
                .create_user_with_invite(&username, &password, None, &code)
                .instrument(tracing::info_span!("web.auth.register.create_user_invite"))
                .await
                .map_err(register_invite_error)?
        }
        RegistrationPolicy::Closed => {
            return Err(WebError::validation("registration is closed"));
        }
    };

    let raw_token = state
        .sessions
        .create_session(user_id, None)
        .instrument(tracing::info_span!("web.auth.register.create_session"))
        .await
        .map_err(WebError::storage)?;

    set_session_cookie(&raw_token);
    leptos_axum::redirect("/");
    Ok(raw_token)
}

/// Authenticates a user.  Returns the raw session token on success and sets
/// the `session` cookie.
#[server(endpoint = "/login")]
#[cfg_attr(
    feature = "ssr",
    tracing::instrument(name = "web.auth.login", skip(password, label))
)]
pub async fn login(username: String, password: String, label: Option<String>) -> WebResult<String> {
    let state = expect_context::<Arc<AppState>>();
    let username = {
        #[cfg(feature = "ssr")]
        let _phase = tracing::info_span!("web.auth.login.parse_username").entered();
        username
            .to_lowercase()
            .parse::<Username>()
            .map_err(|e| WebError::validation(e.to_string()))?
    };
    let password = {
        #[cfg(feature = "ssr")]
        let _phase = tracing::info_span!("web.auth.login.parse_password").entered();
        password
            .parse::<Password>()
            .map_err(|e| WebError::validation(e.to_string()))?
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
        .map_err(WebError::storage)?;

    set_session_cookie(&raw_token);
    leptos_axum::redirect("/");
    Ok(raw_token)
}

/// Revokes the current session and clears the `session` cookie.
#[server(endpoint = "/logout")]
#[cfg_attr(feature = "ssr", tracing::instrument(name = "web.auth.logout"))]
pub async fn logout() -> WebResult<()> {
    if let Ok(auth) = require_auth().await {
        let state = expect_context::<Arc<AppState>>();
        state
            .sessions
            .revoke_session(&auth.token_hash)
            .await
            .map_err(WebError::storage)?;
    }
    clear_session_cookie();
    #[cfg(feature = "ssr")]
    leptos_axum::redirect("/");
    Ok(())
}

#[cfg(feature = "ssr")]
fn register_open_error(error: common::storage::CreateUserError) -> WebError {
    match error {
        common::storage::CreateUserError::UsernameTaken => {
            WebError::conflict("username is already taken")
        }
        common::storage::CreateUserError::Internal(error) => WebError::storage(error),
    }
}

#[cfg(feature = "ssr")]
fn register_invite_error(error: common::storage::RegisterWithInviteError) -> WebError {
    match error {
        common::storage::RegisterWithInviteError::UsernameTaken => {
            WebError::conflict("username is already taken")
        }
        common::storage::RegisterWithInviteError::InviteNotFound => {
            WebError::validation("invite code not found")
        }
        common::storage::RegisterWithInviteError::InviteExpired => {
            WebError::validation("invite code has expired")
        }
        common::storage::RegisterWithInviteError::InviteAlreadyUsed => {
            WebError::validation("invite code has already been used")
        }
        common::storage::RegisterWithInviteError::Internal(error) => WebError::storage(error),
    }
}

#[cfg(feature = "ssr")]
fn login_error(error: common::storage::UserAuthError) -> WebError {
    match error {
        common::storage::UserAuthError::InvalidCredentials => WebError::Unauthorized,
        common::storage::UserAuthError::Internal(message) => WebError::server_message(message),
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
