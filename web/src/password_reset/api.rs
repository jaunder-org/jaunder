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
// `Username` / `Email` / `ProfferedPassword` / `RawToken` are ungated: they type
// request wire arguments, so generated inputs reference them on both client and
// server builds.
use common::email::Email;
use common::{MutationOutcome, password::ProfferedPassword, token::RawToken, username::Username};
use std::str::FromStr;

/// The validated identifier submitted to begin a password reset.
///
/// Classification is deliberate: `@` commits input to the email parser, so a
/// malformed email-looking identifier cannot be accepted as a username.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum PasswordResetIdentifier {
    Username(Username),
    Email(Email),
}

/// Error returned when a password-reset identifier fails its selected parser.
#[derive(Debug, thiserror::Error)]
pub enum InvalidPasswordResetIdentifier {
    #[error(transparent)]
    Username(#[from] common::username::InvalidUsername),
    #[error(transparent)]
    Email(#[from] common::email::InvalidEmail),
}

impl FromStr for PasswordResetIdentifier {
    type Err = InvalidPasswordResetIdentifier;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.contains('@') {
            Ok(Self::Email(value.parse()?))
        } else {
            Ok(Self::Username(value.parse()?))
        }
    }
}

impl TryFrom<String> for PasswordResetIdentifier {
    type Error = InvalidPasswordResetIdentifier;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<PasswordResetIdentifier> for String {
    fn from(value: PasswordResetIdentifier) -> Self {
        match value {
            PasswordResetIdentifier::Username(username) => username.into(),
            PasswordResetIdentifier::Email(email) => email.into(),
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::{InvalidPasswordResetIdentifier, PasswordResetIdentifier};
    use common::{email::Email, username::Username};

    #[test]
    fn password_reset_identifier_classifies_and_canonicalizes_input() {
        assert_eq!(
            "Alice".parse::<PasswordResetIdentifier>().unwrap(),
            PasswordResetIdentifier::Username("alice".parse::<Username>().unwrap())
        );
        assert_eq!(
            "Local.Part@EXAMPLE.COM"
                .parse::<PasswordResetIdentifier>()
                .unwrap(),
            PasswordResetIdentifier::Email("Local.Part@example.com".parse::<Email>().unwrap())
        );
    }

    #[test]
    fn password_reset_identifier_rejects_the_selected_invalid_variant() {
        assert!(matches!(
            "alice@".parse::<PasswordResetIdentifier>(),
            Err(InvalidPasswordResetIdentifier::Email(_))
        ));
        assert!(matches!(
            "alice.example".parse::<PasswordResetIdentifier>(),
            Err(InvalidPasswordResetIdentifier::Username(_))
        ));
    }

    #[cfg(feature = "server")]
    #[test]
    fn password_reset_indeterminate_outcome_preserves_uncertainty_and_erases_consumption() {
        let outcome = super::finalize_password_reset(
            &common::MutationOutcome::CommitIndeterminate(common::ids::UserId::from(42)),
        );

        assert!(matches!(
            outcome,
            common::MutationOutcome::CommitIndeterminate(())
        ));
    }
}
