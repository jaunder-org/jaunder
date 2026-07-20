#[cfg(feature = "server")]
use {
    chrono::Duration,
    common::mailer::{EmailMessage, MailSender},
    common::password::Password,
    std::sync::Arc,
    storage::{AtomicOps, PasswordResetStorage, UserStorage},
};

#[cfg(feature = "server")]
use crate::error::InternalError;
use crate::error::WebResult;
// `Username` / `ProfferedPassword` / `RawToken` are ungated: they type the
// `request_password_reset` / `confirm_password_reset` wire args, so the generated arg
// structs reference them on both the client and server builds.
use common::password::ProfferedPassword;
use common::token::RawToken;
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

        let link = format!("/reset-password?token={raw_token}");
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
pub async fn confirm_password_reset(
    token: RawToken,
    new_password: ProfferedPassword,
) -> WebResult<()> {
    boundary!("confirm_password_reset", {
        let atomic = expect_context::<Arc<dyn AtomicOps>>();

        // `new_password` is the inbound-secret twin (ADR-0063); convert into the
        // serde-free domain `Password` at the boundary. `token` is a `RawToken` wire
        // arg — its serde bridge already rejected a malformed shape on decode.
        let password = Password::try_from(new_password)?;

        atomic.confirm_password_reset(&token, &password).await?;

        host::metrics::password_reset(host::metrics::PasswordResetEvent::Completed);
        Ok(())
    })
}
