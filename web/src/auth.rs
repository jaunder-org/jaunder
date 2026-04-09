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
/// Returns `ServerFnError` when no valid session is present.
#[cfg(feature = "ssr")]
pub async fn require_auth() -> Result<AuthUser, leptos::prelude::ServerFnError> {
    leptos_axum::extract::<AuthUser>()
        .await
        .map_err(|_| leptos::prelude::ServerFnError::new("unauthorized"))
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
pub async fn get_registration_policy() -> Result<String, ServerFnError> {
    let state = expect_context::<Arc<AppState>>();
    let policy = load_registration_policy(&*state.site_config).await;
    Ok(policy.to_string())
}

/// Returns the current logged-in username, if any.
#[server(endpoint = "/current_user")]
pub async fn current_user() -> Result<Option<String>, ServerFnError> {
    match require_auth().await {
        Ok(auth) => Ok(Some(auth.username.to_string())),
        Err(_) => Ok(None),
    }
}

/// Registers a new user.  Returns the raw session token on success and sets
/// the `session` cookie.
#[server(endpoint = "/register")]
pub async fn register(
    username: String,
    password: String,
    invite_code: Option<String>,
) -> Result<String, ServerFnError> {
    let state = expect_context::<Arc<AppState>>();
    let username = username
        .to_lowercase()
        .parse::<Username>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let password = password
        .parse::<Password>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let policy = load_registration_policy(&*state.site_config).await;

    let user_id = match policy {
        RegistrationPolicy::Open => state
            .users
            .create_user(&username, &password, None)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?,
        RegistrationPolicy::InviteOnly => {
            let code = invite_code
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ServerFnError::new("invite code required"))?;
            state
                .atomic
                .create_user_with_invite(&username, &password, None, &code)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?
        }
        RegistrationPolicy::Closed => {
            return Err(ServerFnError::new("registration is closed"));
        }
    };

    let raw_token = state
        .sessions
        .create_session(user_id, None)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    set_session_cookie(&raw_token);
    Ok(raw_token)
}

/// Authenticates a user.  Returns the raw session token on success and sets
/// the `session` cookie.
#[server(endpoint = "/login")]
pub async fn login(
    username: String,
    password: String,
    label: Option<String>,
) -> Result<String, ServerFnError> {
    let state = expect_context::<Arc<AppState>>();
    let username = username
        .to_lowercase()
        .parse::<Username>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let password = password
        .parse::<Password>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let record = state
        .users
        .authenticate(&username, &password)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let label = label.filter(|s| !s.is_empty());
    let raw_token = state
        .sessions
        .create_session(record.user_id, label.as_deref())
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    set_session_cookie(&raw_token);
    Ok(raw_token)
}

/// Revokes the current session and clears the `session` cookie.
#[server(endpoint = "/logout")]
pub async fn logout() -> Result<(), ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    state
        .sessions
        .revoke_session(&auth.token_hash)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    clear_session_cookie();
    Ok(())
}
