#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    chrono::Utc,
    common::email::Email,
    common::mailer::{EmailMessage, MailSender},
    std::sync::Arc,
    storage::{load_registration_policy, InviteStorage, RegistrationPolicy, SiteConfigStorage},
};

use crate::error::WebResult;
use common::ids::UserId;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Invite metadata returned by [`list_invites`].
///
/// The raw code is deliberately **not** included — a capability token is never sent
/// server→client (#400). Codes are delivered out-of-band (the `jaunder user invite` CLI
/// prints the invitation URL; #433 will email them).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteInfo {
    pub created_at: String,
    pub expires_at: String,
    pub used_at: Option<String>,
    pub used_by: Option<UserId>,
}

/// Creates an invite code expiring in `expires_in_hours` (default 168 = 7 days) and
/// **emails the invitation link** to `recipient_email`. Requires authentication. The
/// code is never returned to the client (#400) — it is delivered only as the link in
/// the email (mirrors `request_password_reset`).
#[server(endpoint = "/create_invite")]
pub async fn create_invite(
    expires_in_hours: Option<u64>,
    recipient_email: String,
) -> WebResult<()> {
    boundary!("create_invite", {
        let _auth = require_auth().await?;
        let invites = expect_context::<Arc<dyn InviteStorage>>();
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let mailer = expect_context::<Arc<dyn MailSender>>();

        // Validate the base URL and the recipient up front, before creating the invite:
        // a failure here must not leave an undelivered invite behind (no orphan).
        let base_url = site_config.get_identity().await?.base_url.ok_or_else(|| {
            InternalError::validation("set the site base URL before emailing invites")
        })?;
        let recipient = recipient_email
            .parse::<Email>()
            .map_err(|_| InternalError::validation("invalid recipient email address"))?;

        let hours = expires_in_hours.unwrap_or(168);
        let duration = i64::try_from(hours)
            .ok()
            .and_then(chrono::Duration::try_hours)
            .ok_or_else(|| InternalError::validation("expires_in_hours too large"))?;
        let expires_at = Utc::now() + duration;

        let code = invites
            .create_invite(expires_at)
            .await
            .map_err(InternalError::storage)?;
        host::metrics::invite(host::metrics::InviteEvent::Created);

        // Deliberate egress of the secret via `AsRef` (InviteCode has no Display/serde).
        let link = format!("{base_url}/register?invite_code={}", code.as_ref());
        let message = EmailMessage {
            from: None,
            to: vec![recipient],
            subject: "You've been invited to Jaunder".to_string(),
            body_text: format!(
                "You've been invited to create an account. Click the link below to register:\n\n{link}\n\nThis invitation expires in {hours} hours."
            ),
        };
        let started = std::time::Instant::now();
        let send_result = mailer.send_email(&message).await;
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        host::metrics::email_send_duration_ms(elapsed_ms);
        host::metrics::email_send_result(host::metrics::EmailKind::Invite, &send_result);
        send_result?;
        Ok(())
    })
}

/// Returns invite metadata (never the raw codes). Requires `invite_only` registration
/// policy; returns an error otherwise.
#[server(endpoint = "/list_invites")]
pub async fn list_invites() -> WebResult<Vec<InviteInfo>> {
    boundary!("list_invites", {
        let _auth = require_auth().await?;
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let invites = expect_context::<Arc<dyn InviteStorage>>();
        let policy = load_registration_policy(&*site_config).await;
        if policy != RegistrationPolicy::InviteOnly {
            return Err(InternalError::not_found("invites"));
        }
        let records = invites.list_invites().await?;
        Ok(records
            .into_iter()
            .map(|r| InviteInfo {
                created_at: r.created_at.to_rfc3339(),
                expires_at: r.expires_at.to_rfc3339(),
                used_at: r.used_at.map(|t| t.to_rfc3339()),
                used_by: r.used_by,
            })
            .collect())
    })
}
