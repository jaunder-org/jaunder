//! Server-only email helpers shared by the mail-sending verticals (`invites`,
//! `email` verification, `password_reset`). Extracted to avoid copy-pasting the
//! same base-URL guard and timed-send-with-metrics block into every vertical.
//!
//! These run inside a `boundary!` block, so they propagate the domain
//! [`InternalError`] (converted to the public `WebError` at the boundary), not
//! `WebError` directly.

use crate::error::InternalError;
use common::absolute_url::AbsoluteUrl;
use common::mailer::{EmailMessage, MailSender};
use host::metrics::EmailKind;
use storage::SiteConfigStorage;

/// The site's absolute base URL, or a validation error when it is unset — a
/// followable email link cannot be composed without it, so the caller must fail
/// rather than mail a dead relative link.
pub async fn require_base_url(
    site_config: &dyn SiteConfigStorage,
) -> Result<AbsoluteUrl, InternalError> {
    site_config
        .get_identity()
        .await?
        .base_url
        .ok_or_else(|| InternalError::validation("set the site base URL before sending email"))
}

/// Send `message`, recording the standard send-duration and send-result metrics
/// under `kind`. Propagates a send failure as the caller's error.
pub async fn send_recording_metrics(
    mailer: &dyn MailSender,
    message: &EmailMessage,
    kind: EmailKind,
) -> Result<(), InternalError> {
    let started = std::time::Instant::now();
    let send_result = mailer.send_email(message).await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    host::metrics::email_send_duration_ms(elapsed_ms);
    host::metrics::email_send_result(kind, &send_result);
    send_result?;
    Ok(())
}
