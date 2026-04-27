#[cfg(feature = "ssr")]
use common::mailer::EmailMessage;
#[cfg(feature = "ssr")]
use common::password::Password;
#[cfg(feature = "ssr")]
use common::storage::AppState;
#[cfg(feature = "ssr")]
use common::username::Username;
#[cfg(feature = "ssr")]
use std::sync::Arc;

#[cfg(feature = "ssr")]
use crate::error::InternalError;
use crate::error::WebResult;
use leptos::prelude::*;

/// Looks up the user by username, checks for a verified email, creates a reset
/// token, and sends a password-reset link. Returns an error when the user has
/// no verified email on file; does **not** distinguish between "user not found"
/// and "no verified email" in the error message to avoid username enumeration.
#[server(endpoint = "/request_password_reset")]
pub async fn request_password_reset(username: String) -> WebResult<()> {
    crate::web_server_fn!("request_password_reset", username => {
        let state = expect_context::<Arc<AppState>>();

        let parsed_username = username
            .to_lowercase()
            .parse::<Username>()
            .map_err(|e| InternalError::validation(e.to_string()))?;

        let user = state
            .users
            .get_user_by_username(&parsed_username)
            .await
            .map_err(InternalError::storage)?;

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

        let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
        let raw_token = state
            .password_resets
            .create_password_reset(user_id, expires_at)
            .await
            .map_err(InternalError::storage)?;

        let link = format!("/reset-password?token={raw_token}");
        let message = EmailMessage {
            from: None,
            to: vec![verified_email],
            subject: "Reset your password".to_string(),
            body_text: format!(
                "Click the link below to reset your password:\n\n{link}\n\nThis link expires in 1 hour."
            ),
        };

        state
            .mailer
            .send_email(&message)
            .await
            .map_err(InternalError::server)?;

        Ok(())
    })
}

/// Validates a password-reset token, sets the new password, and revokes all
/// existing sessions for that user in one atomic database transaction.
#[server(endpoint = "/confirm_password_reset")]
pub async fn confirm_password_reset(token: String, new_password: String) -> WebResult<()> {
    crate::web_server_fn!("confirm_password_reset", token, new_password => {
        let state = expect_context::<Arc<AppState>>();

        let password = new_password
            .parse::<Password>()
            .map_err(|e| InternalError::validation(e.to_string()))?;

        state
            .atomic
            .confirm_password_reset(&token, &password)
            .await
            .map_err(InternalError::storage)?;

        Ok(())
    })
}
