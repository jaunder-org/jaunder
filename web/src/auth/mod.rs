use crate::error::WebResult;
use leptos::prelude::*;

/// The client-side advisory auth marker (#181, ADR-0044). Pure encode/decode are
/// host-testable; `read`/`set`/`clear` are wasm-only (localStorage).
pub mod marker;

#[cfg(feature = "ssr")]
mod server;
#[cfg(feature = "ssr")]
use server::{
    classify_current_user, clear_session_cookie, login_error, login_outcome, register_invite_error,
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
    boundary!("get_registration_policy", {
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let policy = load_registration_policy(&*site_config).await;
        Ok(policy.to_string())
    })
}

/// Returns the current logged-in username, if any.
#[server(endpoint = "/current_user")]
#[cfg_attr(feature = "ssr", tracing::instrument(name = "web.auth.current_user"))]
pub async fn current_user() -> WebResult<Option<String>> {
    boundary!("current_user", {
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
    boundary!("register", {
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

        let metric_policy = match &policy {
            RegistrationPolicy::Open => common::metrics::RegistrationPolicy::Open,
            RegistrationPolicy::InviteOnly => common::metrics::RegistrationPolicy::InviteOnly,
            RegistrationPolicy::Closed => common::metrics::RegistrationPolicy::Closed,
        };
        let user_id_result: Result<i64, InternalError> = match policy {
            RegistrationPolicy::Open => users
                .create_user(&username, &password, None, false)
                .instrument(tracing::info_span!("web.auth.register.create_user_open"))
                .await
                .map_err(register_open_error),
            RegistrationPolicy::InviteOnly => {
                match invite_code.and_then(common::text::non_empty_owned) {
                    Some(code) => {
                        let result = atomic
                            .create_user_with_invite(&username, &password, None, false, &code)
                            .instrument(tracing::info_span!("web.auth.register.create_user_invite"))
                            .await
                            .map_err(register_invite_error);
                        // A successful invite registration redeems the code.
                        if result.is_ok() {
                            common::metrics::invite(common::metrics::InviteEvent::Redeemed);
                        }
                        result
                    }
                    None => Err(InternalError::validation("invite code required")),
                }
            }
            RegistrationPolicy::Closed => Err(InternalError::validation("registration is closed")),
        };
        common::metrics::registration(
            common::metrics::RegistrationSource::Web,
            metric_policy,
            if user_id_result.is_ok() {
                common::metrics::RegistrationResult::Ok
            } else {
                common::metrics::RegistrationResult::Rejected
            },
        );
        let user_id = user_id_result?;

        let raw_token = sessions
            .create_session(user_id, "Sign-up session")
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
    boundary!("login", {
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
        let record = match users
            .authenticate(&username, &password)
            .instrument(tracing::info_span!("web.auth.login.authenticate_user"))
            .await
        {
            Ok(record) => {
                common::metrics::login(common::metrics::LoginOutcome::Success);
                record
            }
            Err(error) => {
                common::metrics::login(login_outcome(&error));
                return Err(login_error(error));
            }
        };

        // Prefer explicit label if provided; otherwise derive from User-Agent header
        let derived_label = if let Some(l) = label.and_then(common::text::non_empty_owned) {
            l
        } else {
            let ua = leptos_axum::extract::<axum::http::HeaderMap>()
                .await
                .ok()
                .and_then(|headers| {
                    headers
                        .get("user-agent")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "Unknown device".to_string());
            if ua.len() > 200 {
                ua.chars().take(200).collect::<String>()
            } else {
                ua
            }
        };

        let raw_token = sessions
            .create_session(record.user_id, &derived_label)
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
    boundary!("logout", {
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
