use leptos::prelude::*;
use serde::{Deserialize, Serialize};

/// Invite info returned by [`list_invites`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteInfo {
    pub code: String,
    pub created_at: String,
    pub expires_at: String,
    pub used_at: Option<String>,
    pub used_by: Option<i64>,
}

#[cfg(feature = "ssr")]
use crate::auth::require_auth;
#[cfg(feature = "ssr")]
use chrono::Utc;
#[cfg(feature = "ssr")]
use common::auth::{load_registration_policy, RegistrationPolicy};
#[cfg(feature = "ssr")]
use common::storage::AppState;
#[cfg(feature = "ssr")]
use std::sync::Arc;

/// Creates an invite code expiring in `expires_in_hours` (default 168 = 7 days).
/// Requires authentication.
#[server(endpoint = "/create_invite")]
pub async fn create_invite(expires_in_hours: Option<u64>) -> Result<String, ServerFnError> {
    let _auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    let hours = expires_in_hours.unwrap_or(168);
    let duration = i64::try_from(hours)
        .ok()
        .and_then(chrono::Duration::try_hours)
        .ok_or_else(|| ServerFnError::new("expires_in_hours too large"))?;
    let expires_at = Utc::now() + duration;
    state
        .invites
        .create_invite(expires_at)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Returns all invite codes. Requires `invite_only` registration policy;
/// returns an error otherwise.
#[server(endpoint = "/list_invites")]
pub async fn list_invites() -> Result<Vec<InviteInfo>, ServerFnError> {
    let _auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();
    let policy = load_registration_policy(&*state.site_config).await;
    if policy != RegistrationPolicy::InviteOnly {
        return Err(ServerFnError::new("not found"));
    }
    let records = state
        .invites
        .list_invites()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(records
        .into_iter()
        .map(|r| InviteInfo {
            code: r.code,
            created_at: r.created_at.to_rfc3339(),
            expires_at: r.expires_at.to_rfc3339(),
            used_at: r.used_at.map(|t| t.to_rfc3339()),
            used_by: r.used_by,
        })
        .collect())
}
