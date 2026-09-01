//! Email vertical — API surface: the verification `#[server]` endpoints and
//! their wire types (ADR-0070). Re-exported from `mod.rs`.

#[cfg(feature = "server")]
use {
    crate::auth,
    crate::error::{InternalError, from_write_scope_error},
    crate::mail,
    common::mailer::{EmailMessage, MailSender},
    common::tagged_url::{self, MailConfirmUrl},
    common::time::UtcInstant,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{EmailVerificationStorage, SiteConfigStorage, UserStorage, WriteScope},
};

use crate::error::WebResult;
// Unconditional: `Email` / `RawToken` are typed `#[server]` arguments, so the generated
// request structs must carry them on both the client (serialize) and server
// (deserialize) sides.
use common::{MutationOutcome, email::Email, token::RawToken};

#[cfg(feature = "server")]
fn finalize_verification(outcome: &MutationOutcome<common::ids::UserId>) -> MutationOutcome<()> {
    match outcome {
        MutationOutcome::Confirmed(user_id) => {
            tracing::info!(
                credential.kind = "email_verification",
                credential.outcome = "consumed",
                user.id = %user_id,
                "credential consumed"
            );
            MutationOutcome::Confirmed(())
        }
        MutationOutcome::CommitIndeterminate(_) => MutationOutcome::CommitIndeterminate(()),
    }
}

/// Sends a verification email to `email`. Requires authentication.
///
/// Creates a 24-hour verification token, sends an absolute
/// `{base_url}/verify-email?token=…` link via the configured mailer.
#[macros::server(skip_all)]
pub async fn request_verification(email: Email) -> WebResult<MutationOutcome<()>> {
    // `email` is already validated/normalized: it arrives typed as `Email`, so the
    // arg `Deserialize` ran its `FromStr`. Legitimate clients pre-validate the form
    // field (ADR-0065), so an invalid value only reaches here from a non-browser caller.
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    let email_verifications = expect_context::<Arc<dyn EmailVerificationStorage>>();
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let mailer = expect_context::<Arc<dyn MailSender>>();

    // Fetch the site's absolute base URL before minting a token so a
    // misconfigured site fails rather than mailing a dead relative link.
    let base_url = mail::require_base_url(&*site_config).await?;
    let expires_at = UtcInstant::from(chrono::Utc::now() + chrono::Duration::hours(24));
    let verification_email = email.clone();

    let outcome = write_scope
        .run(|transaction| {
            Box::pin(async move {
                email_verifications
                    .create_email_verification(
                        transaction,
                        auth.user_id,
                        &verification_email,
                        expires_at,
                    )
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)?;

    let verify_url: MailConfirmUrl = tagged_url::compose(&base_url, "/verify-email");
    let link = format!("{verify_url}?token={}", outcome.value());
    let message = EmailMessage {
        from: None,
        to: vec![email],
        subject: "Verify your email address".to_string(),
        body_text: format!(
            "Click the link below to verify your email address:\n\n{link}\n\nThis link expires in 24 hours."
        ),
    };

    mail::send_recording_metrics(&*mailer, &message, host::metrics::EmailKind::Verification)
        .await?;

    Ok(outcome.map(|_| ()))
}

/// Consumes a verification token and marks the associated email as verified
/// on the user account.
#[macros::server(skip_all)]
pub async fn verify(token: RawToken) -> WebResult<MutationOutcome<()>> {
    let write_scope = expect_context::<WriteScope>();
    let email_verifications = expect_context::<Arc<dyn EmailVerificationStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();

    // `token` is a `RawToken` wire arg — its serde bridge already rejected a
    // malformed shape on decode, so no in-body re-parse is needed.
    let outcome = write_scope
        .run(|transaction| {
            Box::pin(async move {
                let consumption = email_verifications
                    .use_email_verification(transaction, &token)
                    .await
                    .map_err(InternalError::storage)?;
                users
                    .set_email(
                        transaction,
                        consumption.user_id,
                        Some(&consumption.email),
                        true,
                    )
                    .await
                    .map_err(InternalError::storage)?;
                Ok(consumption.user_id)
            })
        })
        .await
        .map_err(from_write_scope_error)?;
    Ok(finalize_verification(&outcome))
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::finalize_verification;
    use common::{MutationOutcome, ids::UserId};

    #[test]
    fn verification_indeterminate_outcome_preserves_uncertainty_and_erases_user_id() {
        let outcome =
            finalize_verification(&MutationOutcome::CommitIndeterminate(UserId::from(41)));

        assert!(matches!(outcome, MutationOutcome::CommitIndeterminate(())));
    }
}
