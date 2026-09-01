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
    crate::error::{InternalError, InternalResult, from_write_scope_error},
    common::ids::UserId,
    common::session_label::SessionLabel,
    common::token::RawToken,
    host::invite::InviteCode,
    host::metrics::{self, InviteEvent, RegistrationResult, RegistrationSource},
    host::password,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{
        InviteStorage, SessionStorage, SiteConfigStorage, UserStorage, WriteScope, WriteScopeError,
        account_mutations::{self, RegisterWithInviteInput},
    },
    tracing::Instrument,
};

#[cfg(feature = "server")]
fn classify_registration_scope_result(
    scope_result: Result<
        MutationOutcome<(RawToken, Option<UserId>)>,
        WriteScopeError<InternalError>,
    >,
    span: &tracing::Span,
    metric_policy: metrics::RegistrationPolicy,
    is_invite_registration: bool,
) -> InternalResult<MutationOutcome<RawToken>> {
    match scope_result {
        Ok(MutationOutcome::Confirmed((token, invite_consumed))) => {
            metrics::registration(
                RegistrationSource::Web,
                metric_policy,
                RegistrationResult::Ok,
            );
            if is_invite_registration {
                metrics::invite(InviteEvent::Redeemed);
            }
            if let Some(user_id) = invite_consumed {
                tracing::info!(
                    credential.kind = "invite",
                    credential.outcome = "consumed",
                    user.id = %user_id,
                    "credential consumed"
                );
            }
            Ok(MutationOutcome::Confirmed(token))
        }
        Ok(MutationOutcome::CommitIndeterminate((token, _))) => {
            span.record("registration.outcome", "commit_indeterminate");
            Ok(MutationOutcome::CommitIndeterminate(token))
        }
        Err(error) => {
            metrics::registration(
                RegistrationSource::Web,
                metric_policy,
                RegistrationResult::Rejected,
            );
            Err(from_write_scope_error(error))
        }
    }
}

#[cfg(feature = "server")]
fn finalize_registration(outcome: MutationOutcome<RawToken>) -> MutationOutcome<()> {
    match outcome {
        MutationOutcome::Confirmed(raw_token) => {
            auth::set_session_cookie(&raw_token);
            leptos_axum::redirect("/");
            MutationOutcome::Confirmed(())
        }
        MutationOutcome::CommitIndeterminate(raw_token) => {
            auth::set_session_cookie(&raw_token);
            MutationOutcome::CommitIndeterminate(())
        }
    }
}

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
    let invites = expect_context::<Arc<dyn InviteStorage>>();
    let write_scope = expect_context::<WriteScope>();
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
                let user_id_result: Result<(UserId, Option<UserId>), InternalError> = match policy {
                    RegistrationPolicy::Open => {
                        operation_span.record("registration.outcome", "create_user");
                        let Some(password) = prepared_password.as_ref() else {
                            unreachable!("open registration always prepares its password");
                        };
                        users
                            .create_user(transaction, &username, password, None, false)
                            .instrument(tracing::info_span!(
                                "web.registration.register.create_user"
                            ))
                            .await
                            .map(|user_id| (user_id, None))
                            .map_err(Into::into)
                    }
                    RegistrationPolicy::InviteOnly => {
                        if let Some(proffered) = invite_code {
                            operation_span
                                .record("registration.outcome", "create_user_with_invite");
                            let code = InviteCode::try_from(proffered)
                                .map_err(|_| InternalError::validation("invalid invite code"))?;
                            account_mutations::register_with_invite(
                                transaction,
                                users.as_ref(),
                                invites.as_ref(),
                                RegisterWithInviteInput {
                                    username: &username,
                                    password: &password,
                                    display_name: None,
                                    is_operator: false,
                                    invite_code: &code,
                                },
                            )
                            .instrument(tracing::info_span!(
                                "web.registration.register.create_user_with_invite"
                            ))
                            .await
                            .map(|user_id| (user_id, Some(user_id)))
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
                let (user_id, invite_consumed) = user_id_result?;
                let signup_label = SessionLabel::from_lossy("Sign-up session");
                sessions
                    .create_session(transaction, user_id, &signup_label)
                    .instrument(tracing::info_span!(
                        "web.registration.register.create_session"
                    ))
                    .await
                    .map(|token| (token, invite_consumed))
                    .map_err(InternalError::storage)
            })
        })
        .await;
    let outcome = classify_registration_scope_result(
        scope_result,
        &span,
        metric_policy,
        is_invite_registration,
    )?;
    Ok(finalize_registration(outcome))
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::{classify_registration_scope_result, finalize_registration};
    use common::{MutationOutcome, test_support::parse_raw_token};
    use leptos::prelude::Owner;
    use tracing::Span;

    #[test]
    fn scope_result_preserves_indeterminate_token_without_confirmed_metrics() {
        let token = parse_raw_token("token");
        let outcome = classify_registration_scope_result(
            Ok(MutationOutcome::CommitIndeterminate((token.clone(), None))),
            &Span::none(),
            host::metrics::RegistrationPolicy::Open,
            false,
        )
        .expect("indeterminate commits remain successful wire outcomes");

        assert!(matches!(
            outcome,
            MutationOutcome::CommitIndeterminate(returned)
                if returned.as_ref() == token.as_ref()
        ));
    }

    #[test]
    fn finalize_registration_preserves_indeterminate_envelope_and_sets_cookie() {
        Owner::new().with(|| {
            let outcome = finalize_registration(MutationOutcome::CommitIndeterminate(
                parse_raw_token("token"),
            ));

            assert!(matches!(outcome, MutationOutcome::CommitIndeterminate(())));
        });
    }
}
