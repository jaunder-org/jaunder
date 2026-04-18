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
/// Returns `ServerFnError` when no valid session is present.
#[cfg(feature = "ssr")]
#[tracing::instrument(name = "web.auth.require_auth")]
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
#[cfg_attr(
    feature = "ssr",
    tracing::instrument(name = "web.auth.register", skip(password, invite_code))
)]
pub async fn register(
    username: String,
    password: String,
    invite_code: Option<String>,
) -> Result<String, ServerFnError> {
    let state = expect_context::<Arc<AppState>>();
    let username = {
        #[cfg(feature = "ssr")]
        let _phase = tracing::info_span!("web.auth.register.parse_username").entered();
        username
            .to_lowercase()
            .parse::<Username>()
            .map_err(|e| ServerFnError::new(e.to_string()))?
    };
    let password = {
        #[cfg(feature = "ssr")]
        let _phase = tracing::info_span!("web.auth.register.parse_password").entered();
        password
            .parse::<Password>()
            .map_err(|e| ServerFnError::new(e.to_string()))?
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
            .map_err(|e| ServerFnError::new(e.to_string()))?,
        RegistrationPolicy::InviteOnly => {
            let code = invite_code
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ServerFnError::new("invite code required"))?;
            state
                .atomic
                .create_user_with_invite(&username, &password, None, &code)
                .instrument(tracing::info_span!("web.auth.register.create_user_invite"))
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
        .instrument(tracing::info_span!("web.auth.register.create_session"))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

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
pub async fn login(
    username: String,
    password: String,
    label: Option<String>,
) -> Result<String, ServerFnError> {
    let state = expect_context::<Arc<AppState>>();
    let username = {
        #[cfg(feature = "ssr")]
        let _phase = tracing::info_span!("web.auth.login.parse_username").entered();
        username
            .to_lowercase()
            .parse::<Username>()
            .map_err(|e| ServerFnError::new(e.to_string()))?
    };
    let password = {
        #[cfg(feature = "ssr")]
        let _phase = tracing::info_span!("web.auth.login.parse_password").entered();
        password
            .parse::<Password>()
            .map_err(|e| ServerFnError::new(e.to_string()))?
    };
    let record = state
        .users
        .authenticate(&username, &password)
        .instrument(tracing::info_span!("web.auth.login.authenticate_user"))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let label = label.filter(|s| !s.is_empty());
    let raw_token = state
        .sessions
        .create_session(record.user_id, label.as_deref())
        .instrument(tracing::info_span!("web.auth.login.create_session"))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    set_session_cookie(&raw_token);
    leptos_axum::redirect("/");
    Ok(raw_token)
}

/// Revokes the current session and clears the `session` cookie.
#[server(endpoint = "/logout")]
#[cfg_attr(feature = "ssr", tracing::instrument(name = "web.auth.logout"))]
pub async fn logout() -> Result<(), ServerFnError> {
    if let Ok(auth) = require_auth().await {
        let state = expect_context::<Arc<AppState>>();
        state
            .sessions
            .revoke_session(&auth.token_hash)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
    }
    clear_session_cookie();
    Ok(())
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
