#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    common::mailer::{EmailMessage, MailSender},
    std::sync::Arc,
    storage::{EmailVerificationStorage, UserStorage},
};

use crate::error::WebResult;
// Unconditional: `Email` is the typed `#[server]` argument, so the generated request
// struct must carry it on both the client (serialize) and server (deserialize) sides.
use common::email::Email;
use leptos::prelude::*;

/// Sends a verification email to `email`. Requires authentication.
///
/// Creates a 24-hour verification token, sends a link to `/verify-email?token=…`
/// via the configured mailer.
#[server(endpoint = "/request_email_verification")]
pub async fn request_email_verification(email: Email) -> WebResult<()> {
    boundary!("request_email_verification", {
        // `email` is already validated/normalized: it arrives typed as `Email`, so the
        // arg `Deserialize` ran its `FromStr`. Legitimate clients pre-validate the form
        // field (ADR-0065), so an invalid value only reaches here from a non-browser caller.
        let auth = require_auth().await?;
        let email_verifications = expect_context::<Arc<dyn EmailVerificationStorage>>();
        let mailer = expect_context::<Arc<dyn MailSender>>();

        let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);
        let raw_token = email_verifications
            .create_email_verification(auth.user_id, &email, expires_at)
            .await?;

        let link = format!("/verify-email?token={raw_token}");
        let message = EmailMessage {
            from: None,
            to: vec![email],
            subject: "Verify your email address".to_string(),
            body_text: format!(
                "Click the link below to verify your email address:\n\n{link}\n\nThis link expires in 24 hours."
            ),
        };

        let started = std::time::Instant::now();
        let send_result = mailer.send_email(&message).await;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        host::metrics::email_send_duration_ms(elapsed_ms);
        host::metrics::email_send_result(host::metrics::EmailKind::Verification, &send_result);
        send_result?;

        Ok(())
    })
}

/// Consumes a verification token and marks the associated email as verified
/// on the user account.
#[server(endpoint = "/verify_email")]
pub async fn verify_email(token: String) -> WebResult<()> {
    boundary!("verify_email", {
        let email_verifications = expect_context::<Arc<dyn EmailVerificationStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();

        let (user_id, email_addr) = email_verifications
            .use_email_verification(&token)
            .await
            .map_err(InternalError::storage)?;

        users
            .set_email(user_id, Some(&email_addr), true)
            .await
            .map_err(InternalError::storage)
    })
}
