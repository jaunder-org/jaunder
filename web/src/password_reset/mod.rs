#[cfg(feature = "server")]
use {
    chrono::Duration,
    common::mailer::{EmailMessage, MailSender},
    common::password::Password,
    common::token::RawToken,
    std::sync::Arc,
    storage::{AtomicOps, PasswordResetStorage, UserStorage},
};

#[cfg(feature = "server")]
use crate::error::InternalError;
use crate::error::WebResult;
// `Username` is ungated: it types the `request_password_reset` wire arg, so the
// generated arg struct references it on both the client and server builds.
use common::username::Username;
use leptos::prelude::*;

#[server(endpoint = "/request_password_reset")]
pub async fn request_password_reset(username: Username) -> WebResult<()> {
    boundary!("request_password_reset", {
        let users = expect_context::<Arc<dyn UserStorage>>();
        let password_resets = expect_context::<Arc<dyn PasswordResetStorage>>();
        let mailer = expect_context::<Arc<dyn MailSender>>();

        // `username` arrives already validated + lowercased (typed wire arg,
        // client-pre-validated via `<ValidatedInput<Username>>`, per ADR-0065).
        let user = users.get_user_by_username(&username).await?;

        // Extract user_id and verified email together. Return the same "contact
        // operator" error whether the user is missing or lacks a verified email,
        // to avoid username enumeration.
        let (user_id, verified_email) = user
            .and_then(|u| {
                if u.email_verified {
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

        let expires_at = chrono::Utc::now() + Duration::hours(1);
        let raw_token = password_resets
            .create_password_reset(user_id, expires_at)
            .await?;

        let link = format!("/reset-password?token={}", raw_token.as_ref());
        let message = EmailMessage {
            from: None,
            to: vec![verified_email],
            subject: "Reset your password".to_string(),
            body_text: format!(
                "Click the link below to reset your password:\n\n{link}\n\nThis link expires in 1 hour."
            ),
        };

        let started = std::time::Instant::now();
        let send_result = mailer.send_email(&message).await;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        host::metrics::email_send_duration_ms(elapsed_ms);
        host::metrics::email_send_result(host::metrics::EmailKind::PasswordReset, &send_result);
        send_result?;

        host::metrics::password_reset(host::metrics::PasswordResetEvent::Requested);
        Ok(())
    })
}

#[server(endpoint = "/confirm_password_reset")]
pub async fn confirm_password_reset(token: String, new_password: String) -> WebResult<()> {
    boundary!("confirm_password_reset", {
        let atomic = expect_context::<Arc<dyn AtomicOps>>();

        let password = new_password.parse::<Password>()?;

        let raw_token = RawToken::try_from(token)
            .map_err(|_| InternalError::validation("invalid reset token"))?;

        atomic
            .confirm_password_reset(&raw_token, &password)
            .await
            .map_err(InternalError::storage)?;

        host::metrics::password_reset(host::metrics::PasswordResetEvent::Completed);
        Ok(())
    })
}
