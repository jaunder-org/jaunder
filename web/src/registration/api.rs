//! The **registration** vertical's API surface (ADR-0070, amended #530): the
//! `#[server]` account-provisioning endpoints (`register`,
//! `get_policy`) and their wire types, dual-compiled. `mod.rs`
//! re-exports these so external call sites and the server-fn registrar keep the
//! stable `crate::registration::…` paths.

use crate::error::WebResult;
// `Username` and `RegistrationPolicy` cross the ordinary typed wire boundary.
// Proffered secrets stay confined to direct server-function parameters (ADR-0063).
use common::{MutationOutcome, registration::RegistrationPolicy, username::Username};

// One grouped `feature = "server"` support block for the `#[server]` bodies.
// `set_session_cookie` is auth's — registration logs the freshly-created user in
// through it.
#[cfg(feature = "server")]
use {
    crate::auth,
    crate::error::{InternalError, from_write_scope_error},
    common::ids::UserId,
    common::session_label::SessionLabel,
    host::invite::InviteCode,
    host::metrics::{self, InviteEvent, RegistrationResult, RegistrationSource},
    host::password,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{AtomicOps, SessionStorage, SiteConfigStorage, UserStorage, WriteScope},
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
#[macros::server(
    skip_all,
    fields(
        registration.policy = tracing::field::Empty,
        registration.invite_present = tracing::field::Empty,
        registration.outcome = tracing::field::Empty
    )
)]
pub async fn register(
    username: Username,
    password: common::password::ProfferedPassword,
    invite_code: Option<common::invite::ProfferedInviteCode>,
) -> WebResult<MutationOutcome<()>> {
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let atomic = expect_context::<Arc<dyn AtomicOps>>();
    let sessions = expect_context::<Arc<dyn SessionStorage>>();
    // `username` arrives as a validated typed wire arg. The proffered password is
    // accepted only at this boundary and immediately converted to the serde-free
    // domain secret (ADR-0063).
    let password = password::Password::try_from(password)?;
    let span = tracing::Span::current();
    span.record("registration.invite_present", invite_code.is_some());
    let policy = site_config
        .get_registration_policy()
        .instrument(tracing::info_span!(
            "web.registration.register.get_registration_policy"
        ))
        .await?;
    span.record("registration.policy", policy.as_ref());

    let metric_policy = match &policy {
        RegistrationPolicy::Open => host::metrics::RegistrationPolicy::Open,
        RegistrationPolicy::InviteOnly => host::metrics::RegistrationPolicy::InviteOnly,
        RegistrationPolicy::Closed => host::metrics::RegistrationPolicy::Closed,
    };
    let prepared_password = if matches!(&policy, RegistrationPolicy::Open) {
        Some(
            storage::prepare_password(password.clone())
                .await
                .map_err(InternalError::storage)?,
        )
    } else {
        None
    };
    let is_invite_registration = matches!(&policy, RegistrationPolicy::InviteOnly);
    let operation_span = span.clone();
    let scope_result = write_scope
        .run(|transaction| {
            Box::pin(async move {
                let user_id_result: Result<UserId, InternalError> = match policy {
                    RegistrationPolicy::Open => {
                        operation_span.record("registration.outcome", "create_user");
                        let password = prepared_password.as_ref().ok_or_else(|| {
                            InternalError::server(std::io::Error::other(
                                "open registration password was not prepared",
                            ))
                        })?;
                        users
                            .create_user(transaction, &username, password, None, false)
                            .instrument(tracing::info_span!(
                                "web.registration.register.create_user"
                            ))
                            .await
                            .map_err(Into::into)
                    }
                    RegistrationPolicy::InviteOnly => {
                        if let Some(proffered) = invite_code {
                            operation_span
                                .record("registration.outcome", "create_user_with_invite");
                            let code = InviteCode::try_from(proffered)
                                .map_err(|_| InternalError::validation("invalid invite code"))?;
                            atomic
                                .create_user_with_invite(
                                    transaction,
                                    &username,
                                    &password,
                                    None,
                                    false,
                                    &code,
                                )
                                .instrument(tracing::info_span!(
                                    "web.registration.register.create_user_with_invite"
                                ))
                                .await
                                .map_err(Into::into)
                        } else {
                            operation_span.record("registration.outcome", "invite_required");
                            Err(InternalError::validation("invite code required"))
                        }
                    }
                    RegistrationPolicy::Closed => {
                        operation_span.record("registration.outcome", "closed");
                        Err(InternalError::validation("registration is closed"))
                    }
                };
                let user_id = user_id_result?;
                let signup_label = SessionLabel::from_lossy("Sign-up session");
                sessions
                    .create_session(transaction, user_id, &signup_label)
                    .instrument(tracing::info_span!(
                        "web.registration.register.create_session"
                    ))
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await;
    let outcome = match scope_result {
        Ok(MutationOutcome::Confirmed(token)) => {
            metrics::registration(
                RegistrationSource::Web,
                metric_policy,
                RegistrationResult::Ok,
            );
            if is_invite_registration {
                metrics::invite(InviteEvent::Redeemed);
            }
            MutationOutcome::Confirmed(token)
        }
        Ok(MutationOutcome::CommitIndeterminate(token)) => {
            span.record("registration.outcome", "commit_indeterminate");
            MutationOutcome::CommitIndeterminate(token)
        }
        Err(error) => {
            metrics::registration(
                RegistrationSource::Web,
                metric_policy,
                RegistrationResult::Rejected,
            );
            return Err(from_write_scope_error(error));
        }
    };

    match outcome {
        MutationOutcome::Confirmed(raw_token) => {
            auth::set_session_cookie(&raw_token);
            leptos_axum::redirect("/");
            Ok(MutationOutcome::Confirmed(()))
        }
        MutationOutcome::CommitIndeterminate(raw_token) => {
            auth::set_session_cookie(&raw_token);
            Ok(MutationOutcome::CommitIndeterminate(()))
        }
    }
}
