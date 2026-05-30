use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::error::WebResult;

#[cfg(feature = "ssr")]
use {
    crate::auth::require_auth, crate::error::InternalError, std::sync::Arc, storage::SessionStorage,
};

/// Session info returned by [`list_sessions`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub token_hash: String,
    pub label: String,
    pub created_at: String,
    pub last_used_at: String,
    pub is_current: bool,
}

/// Returns all sessions for the authenticated user.
/// `is_current` is `true` for the session used to make this request.
#[server(endpoint = "/list_sessions")]
pub async fn list_sessions() -> WebResult<Vec<SessionInfo>> {
    boundary!("list_sessions", {
        let auth = require_auth().await?;
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        let records = sessions
            .list_sessions(auth.user_id)
            .await
            .map_err(InternalError::storage)?;
        Ok(records
            .into_iter()
            .map(|r| SessionInfo {
                is_current: r.token_hash == auth.token_hash,
                token_hash: r.token_hash,
                label: r.label,
                created_at: r.created_at.to_rfc3339(),
                last_used_at: r.last_used_at.to_rfc3339(),
            })
            .collect())
    })
}

/// Revokes a session belonging to the authenticated user.
#[server(endpoint = "/revoke_session")]
pub async fn revoke_session(token_hash: String) -> WebResult<()> {
    boundary!("revoke_session", {
        let auth = require_auth().await?;
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        let session_records = sessions
            .list_sessions(auth.user_id)
            .await
            .map_err(InternalError::storage)?;
        if !session_records.iter().any(|s| s.token_hash == token_hash) {
            return Err(InternalError::not_found("session"));
        }
        sessions
            .revoke_session(&token_hash)
            .await
            .map_err(InternalError::storage)
    })
}
