//! The **registration** vertical's API surface (ADR-0070, amended #530): the
//! `#[server]` account-provisioning endpoints (`register`,
//! `get_policy`) and their wire types, dual-compiled. `mod.rs`
//! re-exports these so external call sites and the server-fn registrar keep the
//! stable `crate::registration::…` paths.

use crate::error::WebResult;
// `Username` / `ProfferedInviteCode` / `ProfferedPassword` are ungated: they are wire-arg
// types of `register`, so the `#[server]`-generated arg struct references them on both the
// client and server builds.
use common::invite::ProfferedInviteCode;
use common::password::ProfferedPassword;
// Ungated: `RegistrationPolicy` is the wire *return* type of `get_policy`, so the
// `#[server]`-generated signature references it on both the client and server
// builds. `RawToken` is deliberately absent — `register` returns `()`, and the
// session token it mints stays server-side in the HttpOnly cookie (#533; the rule
// is recorded in docs/adr/0107-web-session-establishment-is-cookie-only.md).
use common::registration::RegistrationPolicy;
use common::username::Username;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegistrationRequest {
    pub username: Username,
    pub password: ProfferedPassword,
    pub invite_code: Option<ProfferedInviteCode>,
}

// One grouped `feature = "server"` support block for the `#[server]` bodies.
// `set_session_cookie` is auth's — registration logs the freshly-created user in
// through it.
#[cfg(feature = "server")]
use {
    crate::auth::set_session_cookie,
    crate::error::InternalError,
    common::ids::UserId,
    common::password::Password,
    common::session_label::SessionLabel,
    host::invite::InviteCode,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{AtomicOps, SessionStorage, SiteConfigStorage, UserStorage},
    tracing::Instrument,
};

/// Returns the site's current registration policy — one of
/// [`RegistrationPolicy::Open`], [`RegistrationPolicy::InviteOnly`], or
/// [`RegistrationPolicy::Closed`].
#[macros::server]
pub async fn get_policy() -> WebResult<RegistrationPolicy> {
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let policy = site_config.get_registration_policy().await?;
    Ok(policy)
}

/// Registers a new user and logs them in by setting the `HttpOnly` `session` cookie.
///
/// Returns `()`: the freshly minted session token is deliberately not sent back in
/// the body (#533), so an XSS at registration time has no credential to read. The
/// rule is recorded in
/// `docs/adr/0107-web-session-establishment-is-cookie-only.md`.
#[macros::server(skip_all)]
pub async fn register(request: RegistrationRequest) -> WebResult<()> {
    let RegistrationRequest {
        username,
        password,
        invite_code,
    } = request;
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let atomic = expect_context::<Arc<dyn AtomicOps>>();
    let sessions = expect_context::<Arc<dyn SessionStorage>>();
    // `username` / `password` arrive already validated: typed wire args whose
    // serde bridge routes through their validating `FromStr` (a too-short
    // password is rejected at deserialization), client-pre-validated via
    // `<ValidatedInput<_>>` (ADR-0065). `ProfferedPassword` is the inbound-secret
    // twin of the serde-free `Password` (ADR-0063); convert into it here.
    let password = Password::try_from(password)?;
    let policy = site_config
        .get_registration_policy()
        .instrument(tracing::info_span!(
            "web.registration.register.get_registration_policy"
        ))
        .await?;

    let metric_policy = match &policy {
        RegistrationPolicy::Open => host::metrics::RegistrationPolicy::Open,
        RegistrationPolicy::InviteOnly => host::metrics::RegistrationPolicy::InviteOnly,
        RegistrationPolicy::Closed => host::metrics::RegistrationPolicy::Closed,
    };
    let user_id_result: Result<UserId, InternalError> = match policy {
        RegistrationPolicy::Open => users
            .create_user(&username, &password, None, false)
            .instrument(tracing::info_span!(
                "web.registration.register.create_user_open"
            ))
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
                        .instrument(tracing::info_span!(
                            "web.registration.register.create_user_invite"
                        ))
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

    let signup_label = SessionLabel::from_lossy("Sign-up session");
    let raw_token = sessions
        .create_session(user_id, &signup_label)
        .instrument(tracing::info_span!(
            "web.registration.register.create_session"
        ))
        .await?;

    set_session_cookie(&raw_token);
    leptos_axum::redirect("/");
    // Session establishment is cookie-only (#533) — nothing to return.
    Ok(())
}
