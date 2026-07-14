#[cfg(feature = "server")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    chrono::Utc,
    std::sync::Arc,
    storage::{load_registration_policy, InviteStorage, RegistrationPolicy, SiteConfigStorage},
};

use crate::error::WebResult;
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
    pub used_by: Option<i64>,
}

/// Creates an invite code expiring in `expires_in_hours` (default 168 = 7 days).
/// Requires authentication. The code is **not** returned to the client (#400); it is
/// delivered out-of-band (CLI now, email in #433).
#[server(endpoint = "/create_invite")]
pub async fn create_invite(expires_in_hours: Option<u64>) -> WebResult<()> {
    boundary!("create_invite", {
        let _auth = require_auth().await?;
        let invites = expect_context::<Arc<dyn InviteStorage>>();
        let hours = expires_in_hours.unwrap_or(168);
        let duration = i64::try_from(hours)
            .ok()
            .and_then(chrono::Duration::try_hours)
            .ok_or_else(|| InternalError::validation("expires_in_hours too large"))?;
        let expires_at = Utc::now() + duration;
        let created = invites
            .create_invite(expires_at)
            .await
            .map_err(InternalError::storage);
        if created.is_ok() {
            host::metrics::invite(host::metrics::InviteEvent::Created);
        }
        // The InviteCode is intentionally discarded — never returned to a client.
        created.map(|_| ())
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
