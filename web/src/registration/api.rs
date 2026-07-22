//! The **registration** vertical's API surface (ADR-0070, amended #530): the
//! `#[server]` account-provisioning endpoints (`register`,
//! `get_registration_policy`) and their wire types, dual-compiled. `mod.rs`
//! re-exports these so external call sites and the server-fn registrar keep the
//! stable `crate::registration::…` paths.

use crate::error::WebResult;
// `Username` / `ProfferedInviteCode` / `ProfferedPassword` are ungated: they are wire-arg
// types of `register`, so the `#[server]`-generated arg struct references them on both the
// client and server builds.
use common::invite::ProfferedInviteCode;
use common::password::ProfferedPassword;
// Ungated: `RegistrationPolicy` is the wire *return* type of `get_registration_policy`,
// so the `#[server]`-generated signature references it on both the client and server builds.
use common::registration::RegistrationPolicy;
use common::username::Username;
use leptos::prelude::*;

// One grouped `feature = "server"` support block for the `#[server]` bodies.
// `set_session_cookie` is auth's — registration logs the freshly-created user in
// through it.
#[cfg(feature = "server")]
use {
    crate::auth::set_session_cookie,
    crate::error::InternalError,
    common::ids::UserId,
    common::password::Password,
    host::invite::InviteCode,
    std::sync::Arc,
    storage::{
        load_registration_policy, AtomicOps, SessionStorage, SiteConfigStorage, UserStorage,
    },
    tracing::Instrument,
};

/// Returns the site's current registration policy — one of
/// [`RegistrationPolicy::Open`], [`RegistrationPolicy::InviteOnly`], or
/// [`RegistrationPolicy::Closed`].
#[server(endpoint = "/get_registration_policy")]
#[tracing::instrument(name = "web.registration.get_registration_policy")]
pub async fn get_registration_policy() -> WebResult<RegistrationPolicy> {
    boundary!("get_registration_policy", {
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let policy = load_registration_policy(&*site_config).await;
        Ok(policy)
    })
}

/// Registers a new user.  Returns the raw session token on success and sets
/// the `session` cookie.
#[server(endpoint = "/register")]
#[tracing::instrument(name = "web.registration.register", skip(password, invite_code))]
pub async fn register(
    username: Username,
    password: ProfferedPassword,
    invite_code: Option<ProfferedInviteCode>,
) -> WebResult<String> {
    boundary!("register", {
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
        let policy = load_registration_policy(&*site_config)
            .instrument(tracing::info_span!(
                "web.registration.register.load_registration_policy"
            ))
            .await;

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

        let raw_token = sessions
            .create_session(user_id, "Sign-up session")
            .instrument(tracing::info_span!(
                "web.registration.register.create_session"
            ))
            .await?;

        set_session_cookie(&raw_token);
        leptos_axum::redirect("/");
        Ok(raw_token.to_string())
    })
}
