//! Invites vertical — API surface: the `InviteInfo` wire type and the invite
//! `#[server]` endpoints (ADR-0070). Re-exported from `mod.rs`.

#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    chrono::Utc,
    common::absolute_url::compose,
    common::mailer::{EmailMessage, MailSender},
    std::sync::Arc,
    storage::{load_registration_policy, InviteStorage, RegistrationPolicy, SiteConfigStorage},
};

use crate::error::WebResult;
use common::email::Email;
use common::ids::UserId;
use common::invite::InviteTtlHours;
use common::time::UtcInstant;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Invite metadata returned by [`list_invites`].
///
/// The raw code is deliberately **not** included — a capability token is never sent
/// server→client (#400). Codes are delivered out-of-band (the `jaunder user invite` CLI
/// prints the invitation URL; #433 will email them).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteInfo {
    pub created_at: UtcInstant,
    pub expires_at: UtcInstant,
    pub used_at: Option<UtcInstant>,
    pub used_by: Option<UserId>,
}

/// Creates an invite code expiring in `expires_in_hours` (default 168 = 7 days) and
/// **emails the invitation link** to `recipient_email`. Requires authentication. The
/// code is never returned to the client (#400) — it is delivered only as the link in
/// the email (mirrors `request_password_reset`).
#[server(endpoint = "/create_invite")]
pub async fn create_invite(
    expires_in_hours: Option<InviteTtlHours>,
    recipient_email: Email,
) -> WebResult<()> {
    boundary!("create_invite", {
        let _auth = require_auth().await?;
        let invites = expect_context::<Arc<dyn InviteStorage>>();
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        let mailer = expect_context::<Arc<dyn MailSender>>();

        // Validate the base URL up front, before creating the invite: a failure here
        // must not leave an undelivered invite behind (no orphan). The recipient is
        // already a validated `Email` — the typed `#[server]` arg rejects a malformed
        // address at decode time (ADR-0065), so no in-handler parse is needed.
        let base_url = crate::mail::require_base_url(&*site_config).await?;

        // The bound now lives in `InviteTtlHours` (1..=336): the typed arg rejects an
        // out-of-range value at decode, so no in-body overflow check is needed. `hours` is
        // reused in the email body below.
        let hours = expires_in_hours.unwrap_or_default().value();
        let expires_at = Utc::now() + chrono::Duration::hours(hours);

        let code = invites
            .create_invite(expires_at)
            .await
            .map_err(InternalError::storage)?;
        host::metrics::invite(host::metrics::InviteEvent::Created);

        // Deliberate egress of the secret via `AsRef` (InviteCode has no Display/serde).
        // Compose base + `/register` (correct slash boundary) then append the code as a
        // raw query param, preserving its exact spelling.
        let register_url = compose(&base_url, "/register");
        let link = format!("{register_url}?invite_code={}", code.as_ref());
        let message = EmailMessage {
            from: None,
            to: vec![recipient_email],
            subject: "You've been invited to Jaunder".to_string(),
            body_text: format!(
                "You've been invited to create an account. Click the link below to register:\n\n{link}\n\nThis invitation expires in {hours} hours."
            ),
        };
        crate::mail::send_recording_metrics(&*mailer, &message, host::metrics::EmailKind::Invite)
            .await?;
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
                created_at: UtcInstant::from(r.created_at),
                expires_at: UtcInstant::from(r.expires_at),
                used_at: r.used_at.map(UtcInstant::from),
                used_by: r.used_by,
            })
            .collect())
    })
}
