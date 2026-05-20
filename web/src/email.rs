#[cfg(feature = "ssr")]
use crate::auth::require_auth;
#[cfg(feature = "ssr")]
use crate::error::InternalError;
use crate::error::WebResult;
#[cfg(feature = "ssr")]
use common::mailer::{EmailMessage, MailSender};
#[cfg(feature = "ssr")]
use std::sync::Arc;
#[cfg(feature = "ssr")]
use storage::{EmailVerificationStorage, UserStorage};

use leptos::prelude::*;

/// Sends a verification email to `email`. Requires authentication.
///
/// Creates a 24-hour verification token, sends a link to `/verify-email?token=…`
/// via the configured mailer.
#[server(endpoint = "/request_email_verification")]
pub async fn request_email_verification(email: String) -> WebResult<()> {
    crate::web_server_fn!("request_email_verification", email => {
        let auth = require_auth().await?;
        let email_verifications = expect_context::<Arc<dyn EmailVerificationStorage>>();
        let mailer = expect_context::<Arc<dyn MailSender>>();

        let email_addr = email
            .parse::<email_address::EmailAddress>()
            .map_err(|e| InternalError::validation(e.to_string()))?;

        let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);
        let raw_token = email_verifications
            .create_email_verification(auth.user_id, &email, expires_at)
            .await
            .map_err(InternalError::storage)?;

        let link = format!("/verify-email?token={raw_token}");
        let message = EmailMessage {
            from: None,
            to: vec![email_addr],
            subject: "Verify your email address".to_string(),
            body_text: format!(
                "Click the link below to verify your email address:\n\n{link}\n\nThis link expires in 24 hours."
            ),
        };

        mailer
            .send_email(&message)
            .await
            .map_err(InternalError::server)?;

        Ok(())
    })
}

/// Consumes a verification token and marks the associated email as verified
/// on the user account.
#[server(endpoint = "/verify_email")]
pub async fn verify_email(token: String) -> WebResult<()> {
    crate::web_server_fn!("verify_email", token => {
        let email_verifications = expect_context::<Arc<dyn EmailVerificationStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();

        let (user_id, email_str) = email_verifications
            .use_email_verification(&token)
            .await
            .map_err(InternalError::storage)?;

        let email_addr = email_str
            .parse::<email_address::EmailAddress>()
            .map_err(|e| InternalError::validation(e.to_string()))?;

        users
            .set_email(user_id, Some(&email_addr), true)
            .await
            .map_err(InternalError::storage)
    })
}
