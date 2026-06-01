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

/// The raw token of a freshly minted app password, shown to the user once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPassword {
    /// The raw token — used as the password for `AtomPub` HTTP Basic auth.
    pub token: String,
    /// The label recorded for this app password.
    pub label: String,
}

/// Mints a new app-specific password (a labelled session) for the authenticated
/// user. The returned raw token is shown only once; only its hash is stored.
#[server(endpoint = "/create_app_password")]
pub async fn create_app_password(label: String) -> WebResult<AppPassword> {
    boundary!("create_app_password", {
        let auth = require_auth().await?;
        let label = label.trim();
        if label.is_empty() {
            return Err(InternalError::validation("a label is required"));
        }
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        let token = sessions
            .create_session(auth.user_id, label)
            .await
            .map_err(InternalError::storage)?;
        Ok(AppPassword {
            token,
            label: label.to_string(),
        })
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
