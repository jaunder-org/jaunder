//! Email vertical — API surface: the verification `#[server]` endpoints and
//! their wire types (ADR-0070). Re-exported from `mod.rs`.

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    common::absolute_url::compose,
    common::mailer::{EmailMessage, MailSender},
    std::sync::Arc,
    storage::{EmailVerificationStorage, SiteConfigStorage, UserStorage},
};

use crate::error::WebResult;
// Unconditional: `Email` / `RawToken` are typed `#[server]` arguments, so the generated
// request structs must carry them on both the client (serialize) and server
// (deserialize) sides.
use common::email::Email;
use common::token::RawToken;
use leptos::prelude::*;

/// Sends a verification email to `email`. Requires authentication.
///
/// Creates a 24-hour verification token, sends an absolute
/// `{base_url}/verify-email?token=…` link via the configured mailer.
#[server(endpoint = "/request_email_verification")]
pub async fn request_email_verification(email: Email) -> WebResult<()> {
    boundary!("request_email_verification", {
        // `email` is already validated/normalized: it arrives typed as `Email`, so the
        // arg `Deserialize` ran its `FromStr`. Legitimate clients pre-validate the form
        // field (ADR-0065), so an invalid value only reaches here from a non-browser caller.
        let auth = require_auth().await?;
        let email_verifications = expect_context::<Arc<dyn EmailVerificationStorage>>();
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let mailer = expect_context::<Arc<dyn MailSender>>();

        // Fetch the site's absolute base URL before minting a token so a
        // misconfigured site fails rather than mailing a dead relative link.
        let base_url = crate::mail::require_base_url(&*site_config).await?;

        let expires_at = chrono::Utc::now() + chrono::Duration::hours(24);
        let raw_token = email_verifications
            .create_email_verification(auth.user_id, &email, expires_at)
            .await?;

        let verify_url = compose(&base_url, "/verify-email");
        let link = format!("{verify_url}?token={raw_token}");
        let message = EmailMessage {
            from: None,
            to: vec![email],
            subject: "Verify your email address".to_string(),
            body_text: format!(
                "Click the link below to verify your email address:\n\n{link}\n\nThis link expires in 24 hours."
            ),
        };

        crate::mail::send_recording_metrics(
            &*mailer,
            &message,
            host::metrics::EmailKind::Verification,
        )
        .await?;

        Ok(())
    })
}

/// Consumes a verification token and marks the associated email as verified
/// on the user account.
#[server(endpoint = "/verify_email")]
pub async fn verify_email(token: RawToken) -> WebResult<()> {
    boundary!("verify_email", {
        let email_verifications = expect_context::<Arc<dyn EmailVerificationStorage>>();
        let users = expect_context::<Arc<dyn UserStorage>>();

        // `token` is a `RawToken` wire arg — its serde bridge already rejected a
        // malformed shape on decode, so no in-body re-parse is needed.
        let (user_id, email_addr) = email_verifications.use_email_verification(&token).await?;

        users
            .set_email(user_id, Some(&email_addr), true)
            .await
            .map_err(InternalError::storage)
    })
}
