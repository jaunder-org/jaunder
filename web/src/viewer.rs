//! Viewer-identity extraction for `#[server]` read paths.
//!
//! [`viewer_identity`] resolves who is asking for content so the storage layer
//! can apply its read-time resolution filter (ADR-0020). Layer A constructs
//! only two viewer shapes:
//!
//! - [`ViewerIdentity::Anonymous`] — no account session is present.
//! - [`ViewerIdentity::Channel`] on the `local` channel — a logged-in local
//!   account.
//!
//! This module is the thin leptos adapter: it extracts the account session and
//! the storage handle, then delegates to the pushed-down cores —
//! [`account_viewer`](common::visibility::account_viewer) (the pure projection)
//! and [`local_channel_id`](storage::local_channel_id) (the memoized,
//! fail-closed `local` channel lookup).

#[cfg(feature = "server")]
use {
    crate::auth::AuthUser,
    common::visibility::{account_viewer, ViewerIdentity},
    leptos_axum,
    std::sync::Arc,
    storage::{local_channel_id, SubscriptionStorage},
};

/// Resolves the viewer for a `#[server]` read path.
///
/// Returns [`ViewerIdentity::Channel`] on the `local` channel when a valid
/// account session is present (keyed by the account's `user_id`), otherwise
/// [`ViewerIdentity::Anonymous`].
///
/// **Layer A** only ever resolves an account session or anonymous. **Layer C**
/// inserts a precedence ladder *here* — account session → viewer session →
/// anonymous — so that an unauthenticated request carrying a guest "viewer
/// session" cookie can still be admitted to subscriber/named content. The
/// account-session branch below stays first in that ladder; the viewer-session
/// branch slots in between it and the anonymous fallback.
///
/// A failure to look up the `local` channel id (storage error) falls back to
/// [`ViewerIdentity::Anonymous`] — fail closed: a viewer we cannot positively
/// identify is treated as anonymous and sees only public content.
#[cfg(feature = "server")]
pub async fn viewer_identity() -> ViewerIdentity {
    use leptos::prelude::expect_context;

    // ---- Layer C insertion point: precedence ladder begins here. ----
    // 1. Account session (the only positively-authenticated branch in Layer A).
    let Ok(auth) = leptos_axum::extract::<AuthUser>().await else {
        // 2. (Layer C) viewer-session branch inserts here.
        // 3. Anonymous fallback.
        return ViewerIdentity::Anonymous;
    };

    let subscriptions = expect_context::<Arc<dyn SubscriptionStorage>>();
    let channel_id = local_channel_id(subscriptions.as_ref()).await;
    account_viewer(auth.user_id, channel_id)
}
