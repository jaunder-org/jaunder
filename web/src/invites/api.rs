//! Invites vertical — API surface: the `Info` wire type and the invite
//! `#[server]` endpoints (ADR-0070). Re-exported from `mod.rs`.

#[cfg(feature = "server")]
use {
    crate::auth,
    crate::error::{InternalError, from_write_scope_error as map_write_scope_error},
    crate::mail,
    chrono::Utc,
    common::mailer::{EmailMessage, MailSender},
    common::tagged_url::{self, MailConfirmUrl},
    leptos::prelude::*,
    std::sync::Arc,
    storage::{InviteStorage, RegistrationPolicy, SiteConfigStorage, WriteScope},
};

use crate::error::WebResult;
use common::{
    MutationOutcome, email::Email, ids::UserId, invite::InviteTtlHours, time::UtcInstant,
};
use serde::{Deserialize, Serialize};

/// Invite metadata returned by [`list`].
///
/// The raw code is deliberately **not** included — a capability token is never sent
/// server→client (#400). Codes are delivered out-of-band (the `jaunder user invite` CLI
/// prints the invitation URL; #433 will email them).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Info {
    pub created_at: UtcInstant,
    pub expires_at: UtcInstant,
    pub used_at: Option<UtcInstant>,
    pub used_by: Option<UserId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInviteRequest {
    pub expires_in_hours: Option<InviteTtlHours>,
    pub recipient_email: Email,
}

/// Creates an invite code expiring in `request.expires_in_hours` (default 168 = 7
/// days) and **emails the invitation link** to `request.recipient_email`. Requires
/// authentication. The code is never returned to the client (#400) — it is delivered
/// only as the link in the email (mirrors `password_reset::request`).
#[macros::server(skip_all)]
pub async fn create(request: CreateInviteRequest) -> WebResult<MutationOutcome<()>> {
    let CreateInviteRequest {
        expires_in_hours,
        recipient_email,
    } = request;
    let _auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    let invites = expect_context::<Arc<dyn InviteStorage>>();
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let mailer = expect_context::<Arc<dyn MailSender>>();

    // Validate the base URL up front, before creating the invite: a failure here
    // must not leave an undelivered invite behind (no orphan). The recipient is
    // already a validated `Email` — the typed `#[server]` arg rejects a malformed
    // address at decode time (ADR-0065), so no in-handler parse is needed.
    let base_url = mail::require_base_url(&*site_config).await?;

    // The bound now lives in `InviteTtlHours` (1..=336): the typed arg rejects an
    // out-of-range value at decode, so no in-body overflow check is needed. `hours` is
    // reused in the email body below.
    let hours = expires_in_hours.unwrap_or_default().value();
    let expires_at = UtcInstant::from(Utc::now() + chrono::Duration::hours(hours));

    let outcome = write_scope
        .run(|transaction| {
            Box::pin(async move {
                invites
                    .create_invite(transaction, expires_at)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(map_write_scope_error)?;

    // Deliberate egress of the secret via `AsRef` (InviteCode has no Display/serde).
    // Compose base + `/register` (correct slash boundary) then append the code as a
    // raw query param, preserving its exact spelling.
    let register_url: MailConfirmUrl = tagged_url::compose(&base_url, "/register");
    let link = format!("{register_url}?invite_code={}", outcome.value().as_ref());
    let message = EmailMessage {
        from: None,
        to: vec![recipient_email],
        subject: "You've been invited to Jaunder".to_string(),
        body_text: format!(
            "You've been invited to create an account. Click the link below to register:\n\n{link}\n\nThis invitation expires in {hours} hours."
        ),
    };
    mail::send_recording_metrics(&*mailer, &message, host::metrics::EmailKind::Invite).await?;
    if matches!(&outcome, MutationOutcome::Confirmed(_)) {
        host::metrics::invite(host::metrics::InviteEvent::Created);
    }
    Ok(outcome.map(|_| ()))
}

/// Returns invite metadata (never the raw codes) under an invitation registration policy.
#[macros::server]
pub async fn list() -> WebResult<Vec<Info>> {
    let _auth = auth::require_auth().await?;
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let invites = expect_context::<Arc<dyn InviteStorage>>();
    let policy = site_config.get_registration_policy().await?;
    if !matches!(
        policy,
        RegistrationPolicy::OperatorInvites | RegistrationPolicy::MemberInvites
    ) {
        return Err(InternalError::not_found("invites"));
    }
    let records = invites.list_invites().await?;
    Ok(records
        .into_iter()
        .map(|r| Info {
            created_at: r.created_at,
            expires_at: r.expires_at,
            used_at: r.used_at,
            used_by: r.used_by,
        })
        .collect())
}
