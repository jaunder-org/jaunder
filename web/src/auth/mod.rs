use crate::error::WebResult;
// `Username` / `ProfferedInviteCode` are ungated: they are wire-arg types of `login` /
// `register`, so the `#[server]`-generated arg structs reference them on both the client
// and server builds.
use common::invite::ProfferedInviteCode;
use common::username::Username;
use leptos::prelude::*;

/// The client-side advisory auth marker (#181, ADR-0044). Pure encode/decode are
/// host-testable; `read`/`set`/`clear` are wasm-only (localStorage).
pub mod marker;

#[cfg(feature = "server")]
mod server;
#[cfg(feature = "server")]
use server::{classify_current_user, clear_session_cookie, set_session_cookie};

// Public re-exports — must remain accessible as crate::auth::* for other modules
#[cfg(feature = "server")]
pub use server::{require_auth, AuthRejection, AuthUser, CookieSettings};

// SSR-only imports for #[server] bodies
#[cfg(feature = "server")]
use {
    crate::error::InternalError,
    common::password::Password,
    host::invite::InviteCode,
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
    feature = "server",
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
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "web.auth.current_user")
)]
pub async fn current_user() -> WebResult<Option<Username>> {
    boundary!("current_user", {
        classify_current_user(require_auth().await)
    })
}

/// Registers a new user.  Returns the raw session token on success and sets
/// the `session` cookie.
#[server(endpoint = "/register")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "web.auth.register", skip(password, invite_code))
)]
pub async fn register(
    username: Username,
    password: String,
    invite_code: Option<ProfferedInviteCode>,
) -> WebResult<String> {
    boundary!("register", {
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();
        let atomic = expect_context::<Arc<dyn AtomicOps>>();
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        // `username` arrives already validated + lowercased (typed wire arg,
        // client-pre-validated via `<ValidatedInput<Username>>`, per ADR-0065).
        let password = {
            let _phase = tracing::info_span!("web.auth.register.parse_password").entered();
            password.parse::<Password>()?
        };
        let policy = load_registration_policy(&*site_config)
            .instrument(tracing::info_span!(
                "web.auth.register.load_registration_policy"
            ))
            .await;

        let metric_policy = match &policy {
            RegistrationPolicy::Open => host::metrics::RegistrationPolicy::Open,
            RegistrationPolicy::InviteOnly => host::metrics::RegistrationPolicy::InviteOnly,
            RegistrationPolicy::Closed => host::metrics::RegistrationPolicy::Closed,
        };
        let user_id_result: Result<i64, InternalError> = match policy {
            RegistrationPolicy::Open => users
                .create_user(&username, &password, None, false)
                .instrument(tracing::info_span!("web.auth.register.create_user_open"))
                .await
                .map_err(Into::into),
            RegistrationPolicy::InviteOnly => {
                // The client sends `None` for a blank field; a present code arrives already
                // shape-validated (deserialized through `ProfferedInviteCode`).
                match invite_code {
                    Some(proffered) => {
                        let code = InviteCode::try_from(proffered)
                            .map_err(|_| InternalError::validation("invalid invite code"))?;
                        let result = atomic
                            .create_user_with_invite(&username, &password, None, false, &code)
                            .instrument(tracing::info_span!("web.auth.register.create_user_invite"))
                            .await
                            .map_err(Into::into);
                        // A successful invite registration redeems the code.
                        if result.is_ok() {
                            host::metrics::invite(host::metrics::InviteEvent::Redeemed);
                        }
                        result
                    }
                    None => Err(InternalError::validation("invite code required")),
                }
            }
            RegistrationPolicy::Closed => Err(InternalError::validation("registration is closed")),
        };
        host::metrics::registration(
            host::metrics::RegistrationSource::Web,
            metric_policy,
            if user_id_result.is_ok() {
                host::metrics::RegistrationResult::Ok
            } else {
                host::metrics::RegistrationResult::Rejected
            },
        );
        let user_id = user_id_result?;

        let raw_token = sessions
            .create_session(user_id, "Sign-up session")
            .instrument(tracing::info_span!("web.auth.register.create_session"))
            .await?;

        set_session_cookie(&raw_token);
        leptos_axum::redirect("/");
        Ok(raw_token.to_string())
    })
}

/// Authenticates a user.  Returns the raw session token on success and sets
/// the `session` cookie.
#[server(endpoint = "/login")]
#[cfg_attr(
    feature = "server",
    tracing::instrument(name = "web.auth.login", skip(password, label))
)]
pub async fn login(
    username: Username,
    password: String,
    label: Option<String>,
) -> WebResult<String> {
    boundary!("login", {
        let users = expect_context::<Arc<dyn UserStorage>>();
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        // `username` arrives already validated + lowercased: its serde bridge routes through
        // `Username::from_str`, and the client pre-validates via `<ValidatedInput<Username>>`.
        let password = {
            let _phase = tracing::info_span!("web.auth.login.parse_password").entered();
            password.parse::<Password>()?
        };
        let record = match users
            .authenticate(&username, &password)
            .instrument(tracing::info_span!("web.auth.login.authenticate_user"))
            .await
        {
            Ok(record) => {
                host::metrics::login(host::metrics::LoginOutcome::Success);
                record
            }
            Err(error) => {
                host::metrics::login(storage::login_outcome(&error));
                return Err(error.into());
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
            .await?;

        set_session_cookie(&raw_token);
        leptos_axum::redirect("/");
        Ok(raw_token.to_string())
    })
}

/// Revokes the current session and clears the `session` cookie.
#[server(endpoint = "/logout")]
#[cfg_attr(feature = "server", tracing::instrument(name = "web.auth.logout"))]
pub async fn logout() -> WebResult<()> {
    boundary!("logout", {
        if let Ok(auth) = require_auth().await {
            let sessions = expect_context::<Arc<dyn SessionStorage>>();
            sessions.revoke_session(&auth.token_hash).await?;
        }
        clear_session_cookie();
        leptos_axum::redirect("/");
        Ok(())
    })
}
