//! Sessions wire DTOs + `#[server]` endpoints (ADR-0070, amended #530): the
//! `Info` / `AppPassword` payloads and the `list` /
//! `create_app_password` / `revoke` server fns. Dual-compiled (host +
//! wasm); the vertical's one grouped `#[cfg(feature = "server")]` use-block lives
//! here. Re-exported from `mod.rs` so external paths stay stable.

use serde::{Deserialize, Serialize};

use common::{
    MutationOutcome,
    session_label::SessionLabel,
    time::UtcInstant,
    token::{RawToken, TokenHash},
};

use crate::error::WebResult;

#[cfg(feature = "server")]
use {
    crate::auth,
    crate::error::{InternalError, from_write_scope_error},
    leptos::prelude::*,
    std::sync::Arc,
    storage::{SessionStorage, WriteScope},
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
#[macros::server]
pub async fn list() -> WebResult<Vec<Info>> {
    let auth = auth::require_auth().await?;
    let sessions = expect_context::<Arc<dyn SessionStorage>>();
    let records = sessions.list_sessions(auth.user_id).await?;
    Ok(records
        .into_iter()
        .map(|r| Info {
            is_current: r.token_hash == auth.token_hash,
            token_hash: r.token_hash,
            label: r.label,
            created_at: r.created_at,
            last_used_at: r.last_used_at,
        })
        .collect())
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
#[macros::server(skip_all)]
pub async fn create_app_password(label: SessionLabel) -> WebResult<MutationOutcome<AppPassword>> {
    // `label` is a typed wire arg (ADR-0065): the `SessionLabel` serde bridge
    // already trimmed it and rejected empty/over-long at decode, so there is no
    // manual validation here.
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    let sessions = expect_context::<Arc<dyn SessionStorage>>();
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                let token = sessions
                    .create_session(transaction, auth.user_id, &label)
                    .await
                    .map_err(InternalError::storage)?;
                Ok(AppPassword { token, label })
            })
        })
        .await
        .map_err(from_write_scope_error)
}

/// Revokes a session belonging to the authenticated user.
#[macros::server(skip_all)]
pub async fn revoke(token_hash: TokenHash) -> WebResult<MutationOutcome<()>> {
    let auth = auth::require_auth().await?;
    let write_scope = expect_context::<WriteScope>();
    let sessions = expect_context::<Arc<dyn SessionStorage>>();
    let session_records = sessions.list_sessions(auth.user_id).await?;
    // `revoke_session` keys only on the token hash, so confirm the target
    // belongs to the caller before revoking — otherwise any authenticated
    // user could revoke another account's session by its hash.
    if !session_records.iter().any(|s| s.token_hash == token_hash) {
        return Err(InternalError::not_found("session"));
    }
    write_scope
        .run(|transaction| {
            Box::pin(async move {
                sessions
                    .revoke_session(transaction, &token_hash)
                    .await
                    .map_err(InternalError::storage)
            })
        })
        .await
        .map_err(from_write_scope_error)
}
