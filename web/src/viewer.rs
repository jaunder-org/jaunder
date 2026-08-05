//! Viewer-identity extraction for `#[server]` read paths.
//!
//! [`viewer_identity`] resolves who is asking for content so the storage layer
//! can apply its read-time resolution filter (ADR-0020). Layer A constructs
//! only two viewer shapes:
//!
//! - [`ViewerIdentity::Anonymous`] — no account session is present.
//! - [`ViewerIdentity::Local`] — a logged-in local account.
//!
//! This module is the thin leptos adapter: it extracts the account session and
//! projects it straight into a viewer. A local viewer carries nothing but its
//! `user_id` (#6) — its channel is always the `local` row, resolved inline in
//! SQL by the queries that need it — so there is no channel lookup here and
//! nothing that can fail.

#[cfg(feature = "server")]
use {crate::auth::AuthUser, common::visibility::ViewerIdentity, leptos_axum};

/// Resolves the viewer for a `#[server]` read path.
///
/// Returns [`ViewerIdentity::Local`] keyed by the account's `user_id` when a
/// valid account session is present, otherwise [`ViewerIdentity::Anonymous`].
///
/// **Layer A** only ever resolves an account session or anonymous. **Layer C**
/// inserts a precedence ladder *here* — account session → viewer session →
/// anonymous — so that an unauthenticated request carrying a guest "viewer
/// session" cookie can still be admitted to subscriber/named content. The
/// account-session branch below stays first in that ladder; the viewer-session
/// branch slots in between it and the anonymous fallback.
#[cfg(feature = "server")]
pub async fn viewer_identity() -> ViewerIdentity {
    // ---- Layer C insertion point: precedence ladder begins here. ----
    // 1. Account session (the only positively-authenticated branch in Layer A).
    let Ok(auth) = leptos_axum::extract::<AuthUser>().await else {
        // 2. (Layer C) viewer-session branch inserts here.
        // 3. Anonymous fallback.
        return ViewerIdentity::Anonymous;
    };

    ViewerIdentity::Local {
        user_id: auth.user_id,
    }
}
