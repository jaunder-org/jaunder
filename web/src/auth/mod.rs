use crate::error::WebResult;
use leptos::prelude::*;

#[cfg(feature = "ssr")]
mod server;
#[cfg(feature = "ssr")]
use server::{
    classify_current_user, clear_session_cookie, login_error, register_invite_error,
    register_open_error, set_session_cookie,
};

// Public re-exports — must remain accessible as crate::auth::* for other modules
#[cfg(feature = "ssr")]
pub use server::{require_auth, AuthRejection, AuthUser, CookieSettings};

// SSR-only imports for #[server] bodies
#[cfg(feature = "ssr")]
use {
    crate::error::InternalError,
    common::{password::Password, username::Username},
    std::sync::Arc,
    storage::{
        load_registration_policy, AtomicOps, RegistrationPolicy, SessionStorage, SiteConfigStorage,
        UserStorage,
    },
    tracing::Instrument,
};

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
