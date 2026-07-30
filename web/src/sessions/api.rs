//! Sessions wire DTOs + `#[server]` endpoints (ADR-0070, amended #530): the
//! `Info` / `AppPassword` payloads and the `list` /
//! `create_app_password` / `revoke` server fns. Dual-compiled (host +
//! wasm); the vertical's one grouped `#[cfg(feature = "server")]` use-block lives
//! here. Re-exported from `mod.rs` so external paths stay stable.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use common::session_label::SessionLabel;
use common::time::UtcInstant;
use common::token::{RawToken, TokenHash};

use crate::error::WebResult;

#[cfg(feature = "server")]
use {
    crate::auth::require_auth, crate::error::InternalError, std::sync::Arc, storage::SessionStorage,
};

/// Session info returned by [`list`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Info {
    pub token_hash: TokenHash,
    pub label: SessionLabel,
    pub created_at: UtcInstant,
    pub last_used_at: UtcInstant,
    pub is_current: bool,
}

/// Returns all sessions for the authenticated user.
/// `is_current` is `true` for the session used to make this request.
#[server(endpoint = "/sessions/list")]
#[tracing::instrument(name = "web.sessions.list")]
pub async fn list() -> WebResult<Vec<Info>> {
    boundary!("list", {
        let auth = require_auth().await?;
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        let records = sessions.list_sessions(auth.user_id).await?;
        Ok(records
            .into_iter()
            .map(|r| Info {
                is_current: r.token_hash == auth.token_hash,
                token_hash: r.token_hash,
                label: r.label,
                created_at: UtcInstant::from(r.created_at),
                last_used_at: UtcInstant::from(r.last_used_at),
            })
            .collect())
    })
}

/// The raw token of a freshly minted app password, shown to the user once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPassword {
    /// The raw token — used as the password for `AtomPub` HTTP Basic auth.
    pub token: RawToken,
    /// The label recorded for this app password.
    pub label: SessionLabel,
}

/// Mints a new app-specific password (a labelled session) for the authenticated
/// user. The returned raw token is shown only once; only its hash is stored.
#[server(endpoint = "/sessions/create_app_password")]
#[tracing::instrument(name = "web.sessions.create_app_password", skip_all)]
pub async fn create_app_password(label: SessionLabel) -> WebResult<AppPassword> {
    boundary!("create_app_password", {
        // `label` is a typed wire arg (ADR-0065): the `SessionLabel` serde bridge
        // already trimmed it and rejected empty/over-long at decode, so there is no
        // manual validation here.
        let auth = require_auth().await?;
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        let token = sessions.create_session(auth.user_id, &label).await?;
        Ok(AppPassword { token, label })
    })
}

/// Revokes a session belonging to the authenticated user.
#[server(endpoint = "/sessions/revoke")]
#[tracing::instrument(name = "web.sessions.revoke", skip_all)]
pub async fn revoke(token_hash: TokenHash) -> WebResult<()> {
    boundary!("revoke", {
        let auth = require_auth().await?;
        let sessions = expect_context::<Arc<dyn SessionStorage>>();
        let session_records = sessions.list_sessions(auth.user_id).await?;
        // `revoke_session` keys only on the token hash, so confirm the target
        // belongs to the caller before revoking — otherwise any authenticated
        // user could revoke another account's session by its hash.
        if !session_records.iter().any(|s| s.token_hash == token_hash) {
            return Err(InternalError::not_found("session"));
        }
        sessions
            .revoke_session(&token_hash)
            .await
            .map_err(InternalError::storage)
    })
}
