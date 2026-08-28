//! Password-reset vertical — API surface: the reset `#[server]` endpoints and
//! their wire arg types (ADR-0070). Re-exported from `mod.rs`.

#[cfg(feature = "server")]
use {
    crate::error::InternalError,
    chrono::Duration,
    common::mailer::{EmailMessage, MailSender},
    common::tagged_url::{MailConfirmUrl, compose},
    common::time::UtcInstant,
    host::password::Password,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{AtomicOps, PasswordResetStorage, SiteConfigStorage, UserStorage},
};

use crate::error::WebResult;
// `Username` / `ProfferedPassword` / `RawToken` are ungated: they type the
// request wire arguments, so generated inputs reference them on both client and
// server builds.
use common::password::ProfferedPassword;
use common::token::RawToken;
use common::username::Username;

#[macros::server]
pub async fn request(username: Username) -> WebResult<()> {
    let users = expect_context::<Arc<dyn UserStorage>>();
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

    // Fetch the site's absolute base URL once we know we'll send — before
    // minting the token, so a misconfigured site fails without leaving an
    // orphan reset token behind.
    let base_url = crate::mail::require_base_url(&*site_config).await?;

    let expires_at = UtcInstant::from(chrono::Utc::now() + Duration::hours(1));
    let raw_token = password_resets
        .create_password_reset(user_id, expires_at)
        .await?;

    let reset_url: MailConfirmUrl = compose(&base_url, "/reset-password");
    let link = format!("{reset_url}?token={raw_token}");
    let message = EmailMessage {
        from: None,
        to: vec![verified_email],
        subject: "Reset your password".to_string(),
        body_text: format!(
            "Click the link below to reset your password:\n\n{link}\n\nThis link expires in 1 hour."
        ),
    };

    crate::mail::send_recording_metrics(
        &*mailer,
        &message,
        host::metrics::EmailKind::PasswordReset,
    )
    .await?;

    host::metrics::password_reset(host::metrics::PasswordResetEvent::Requested);
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfirmPasswordResetRequest {
    pub token: RawToken,
    pub new_password: ProfferedPassword,
}

#[macros::server(skip_all)]
pub async fn confirm(request: ConfirmPasswordResetRequest) -> WebResult<()> {
    let ConfirmPasswordResetRequest {
        token,
        new_password,
    } = request;
    let atomic = expect_context::<Arc<dyn AtomicOps>>();

    // `new_password` is the inbound-secret twin (ADR-0063); convert into the
    // serde-free domain `Password` at the boundary. `token` is a `RawToken` wire
    // arg — its serde bridge already rejected a malformed shape on decode.
    let password = Password::try_from(new_password)?;

    atomic.confirm_password_reset(&token, &password).await?;

    host::metrics::password_reset(host::metrics::PasswordResetEvent::Completed);
    Ok(())
}
