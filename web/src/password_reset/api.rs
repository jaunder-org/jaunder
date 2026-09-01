//! Password-reset vertical — API surface: the reset `#[server]` endpoints and
//! their wire arg types (ADR-0070). Re-exported from `mod.rs`.

#[cfg(feature = "server")]
use {
    crate::error::{InternalError, from_write_scope_error as map_write_scope_error},
    crate::mail,
    chrono::Duration,
    common::mailer::{EmailMessage, MailSender},
    common::tagged_url::{self, MailConfirmUrl},
    common::time::UtcInstant,
    host::metrics::{self, EmailKind, PasswordResetEvent},
    host::password,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{
        PasswordResetStorage, SessionStorage, SiteConfigStorage, UserStorage, WriteScope,
        account_mutations,
    },
};

use crate::error::WebResult;
// `Username` / `ProfferedPassword` / `RawToken` are ungated: they type the
// request wire arguments, so generated inputs reference them on both client and
// server builds.
use common::{MutationOutcome, password::ProfferedPassword, token::RawToken, username::Username};

#[cfg(feature = "server")]
fn finalize_password_reset(outcome: &MutationOutcome<common::ids::UserId>) -> MutationOutcome<()> {
    match outcome {
        MutationOutcome::Confirmed(user_id) => {
            tracing::info!(
                credential.kind = "password_reset",
                credential.outcome = "consumed",
                user.id = %user_id,
                "credential consumed"
            );
            metrics::password_reset(PasswordResetEvent::Completed);
            MutationOutcome::Confirmed(())
        }
        MutationOutcome::CommitIndeterminate(_) => MutationOutcome::CommitIndeterminate(()),
    }
}

#[macros::server]
pub async fn request(username: Username) -> WebResult<MutationOutcome<()>> {
    let users = expect_context::<Arc<dyn UserStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let password_resets = expect_context::<Arc<dyn PasswordResetStorage>>();
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let mailer = expect_context::<Arc<dyn MailSender>>();

    // `username` arrives already validated + lowercased (typed wire arg,
    // client-pre-validated via `<ValidatedInput<Username>>`, per ADR-0065).
    let user = users.get_user_by_username(&username).await?;

    // Extract user_id and verified email together. Return the same "contact
    // operator" error whether the user is missing or lacks a verified email,
    // to avoid username enumeration.
    let (user_id, verified_email) = user
        .and_then(|u| {
            if u.email_verified.is_verified() {
                u.email.map(|e| (u.user_id, e))
            } else {
                None
            }
        })
        .ok_or_else(|| {
            InternalError::validation(
                "No verified email address on file. Please contact the site operator.",
            )
        })?;

    // Fetch the site's absolute base URL once we know we'll send — before
    // minting the token, so a misconfigured site fails without leaving an
    // orphan reset token behind.
    let base_url = mail::require_base_url(&*site_config).await?;

    let expires_at = UtcInstant::from(chrono::Utc::now() + Duration::hours(1));
    let outcome = write_scope
        .run(|transaction| {
            Box::pin(async move {
                password_resets
                    .create_password_reset(transaction, user_id, expires_at)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(map_write_scope_error)?;

    let reset_url: MailConfirmUrl = tagged_url::compose(&base_url, "/reset-password");
    let link = format!("{reset_url}?token={}", outcome.value());
    let message = EmailMessage {
        from: None,
        to: vec![verified_email],
        subject: "Reset your password".to_string(),
        body_text: format!(
            "Click the link below to reset your password:\n\n{link}\n\nThis link expires in 1 hour."
        ),
    };

    mail::send_recording_metrics(&*mailer, &message, EmailKind::PasswordReset).await?;

    if matches!(&outcome, MutationOutcome::Confirmed(_)) {
        metrics::password_reset(PasswordResetEvent::Requested);
    }
    Ok(outcome.map(|_| ()))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfirmPasswordResetRequest {
    pub token: RawToken,
    pub new_password: ProfferedPassword,
}

#[macros::server(skip_all)]
pub async fn confirm(request: ConfirmPasswordResetRequest) -> WebResult<MutationOutcome<()>> {
    let ConfirmPasswordResetRequest {
        token,
        new_password,
    } = request;
    let write_scope = expect_context::<WriteScope>();
    let password_resets = expect_context::<Arc<dyn PasswordResetStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let sessions = expect_context::<Arc<dyn SessionStorage>>();

    // `new_password` is the inbound-secret twin (ADR-0063); convert into the
    // serde-free domain `Password` at the boundary. `token` is a `RawToken` wire
    // arg — its serde bridge already rejected a malformed shape on decode.
    let password = password::Password::try_from(new_password)?;

    let outcome = write_scope
        .run(|transaction| {
            Box::pin(async move {
                account_mutations::confirm_password_reset(
                    transaction,
                    password_resets.as_ref(),
                    users.as_ref(),
                    sessions.as_ref(),
                    &token,
                    &password,
                )
                .await
                .map_err(Into::into)
            })
        })
        .await
        .map_err(map_write_scope_error)?;
    Ok(finalize_password_reset(&outcome))
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::finalize_password_reset;
    use common::{MutationOutcome, ids::UserId};

    #[test]
    fn password_reset_indeterminate_outcome_preserves_uncertainty_and_erases_consumption() {
        let outcome =
            finalize_password_reset(&MutationOutcome::CommitIndeterminate(UserId::from(42)));

        assert!(matches!(outcome, MutationOutcome::CommitIndeterminate(())));
    }
}
