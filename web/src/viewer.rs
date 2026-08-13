//! Viewer-identity extraction for `#[server]` read paths.
//!
//! [`viewer_identity`] resolves who is asking for content so the storage layer
//! can apply its read-time resolution filter (ADR-0020). Layer A constructs
//! only two viewer shapes:
//!
//! - [`ViewerIdentity::Anonymous`] — no account session is present.
//! - [`ViewerIdentity::Local`] — a logged-in local account.
//!
//! This module is the thin Leptos adapter: it extracts the account session and
//! projects it into a viewer. Missing or stale cookie credentials remain
//! anonymous; failures from an explicit Authorization credential propagate.

#[cfg(feature = "server")]
use {
    crate::{
        auth::{AuthRejection, AuthUser, auth_rejection_error},
        error::{InternalError, InternalResult},
    },
    axum::{extract::FromRequestParts, http::request::Parts},
    common::visibility::ViewerIdentity,
    host::auth::CredentialTransport,
};

/// Resolves the viewer for a `#[server]` read path.
///
/// Returns [`ViewerIdentity::Local`] keyed by the account's `user_id` when a
/// valid account session is present. Absent credentials and failed cookie-only
/// credentials resolve to [`ViewerIdentity::Anonymous`]. Explicit Authorization
/// failures reject the read rather than falling back to anonymous.
///
/// **Layer A** only ever resolves an account session or anonymous. **Layer C**
/// inserts a precedence ladder *here* — account session → viewer session →
/// anonymous — so that an unauthenticated request carrying a guest "viewer
/// session" cookie can still be admitted to subscriber/named content. The
/// account-session branch below stays first in that ladder; the viewer-session
/// branch slots in between it and the anonymous fallback.
///
/// # Errors
///
/// Returns an authentication error when a present Authorization credential
/// cannot be resolved or authenticated, and propagates infrastructure failures.
#[cfg(feature = "server")]
pub async fn viewer_identity() -> InternalResult<ViewerIdentity> {
    // ---- Layer C insertion point: precedence ladder begins here. ----
    // 1. Account session (the only positively-authenticated branch in Layer A).
    let mut parts = leptos::context::use_context::<Parts>()
        .ok_or_else(|| InternalError::server_message("missing request parts context"))?;
    match AuthUser::from_request_parts(&mut parts, &()).await {
        Ok(auth) => Ok(ViewerIdentity::local(auth.user_id)),
        Err(
            AuthRejection::MissingToken
            | AuthRejection::Session {
                transport: CredentialTransport::Cookie,
                error:
                    storage::SessionAuthError::InvalidToken | storage::SessionAuthError::SessionNotFound,
            },
        ) => {
            // 2. (Layer C) viewer-session branch inserts here.
            // 3. Anonymous fallback.
            Ok(ViewerIdentity::Anonymous)
        }
        Err(error) => Err(auth_rejection_error(error)),
    }
}
