//! Viewer-identity extraction for `#[server]` read paths.
//!
//! [`viewer_identity`] resolves who is asking for content so the storage layer
//! can apply its read-time resolution filter (ADR-0020). Layer A constructs
//! only two viewer shapes:
//!
//! - [`ViewerIdentity::Anonymous`] — no account session is present.
//! - [`ViewerIdentity::Channel`] on the `local` channel — a logged-in local
//!   account, built via [`ViewerIdentity::local`].
//!
//! The `local` channel id is immutable for the life of the process (it is a
//! seeded lookup row), so it is resolved once and memoized in a process-level
//! [`OnceLock`](std::sync::OnceLock) rather than queried on every request.

#[cfg(feature = "server")]
use {
    crate::auth::AuthUser,
    common::visibility::ViewerIdentity,
    leptos_axum,
    std::sync::{Arc, OnceLock},
    storage::SubscriptionStorage,
};

/// Process-level cache of the seeded `local` channel id.
///
/// The `local` channel is created once by migration `0018` and never changes,
/// so a single lookup is reused for the life of the process instead of querying
/// `channels` on every read request.
#[cfg(feature = "server")]
static LOCAL_CHANNEL_ID: OnceLock<i64> = OnceLock::new();

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

/// Projects an authenticated account plus the resolved `local` channel id into a
/// [`ViewerIdentity`].
///
/// `Some(channel_id)` → a `local` channel viewer; `None` (the `local` channel id
/// could not be resolved) → [`ViewerIdentity::Anonymous`], fail-closed: a viewer
/// we cannot positively place on a channel gets no non-public reach.
#[cfg(feature = "server")]
fn account_viewer(user_id: i64, local_channel_id: Option<i64>) -> ViewerIdentity {
    match local_channel_id {
        Some(channel_id) => ViewerIdentity::local(user_id, channel_id),
        None => ViewerIdentity::Anonymous,
    }
}

/// The local user id of an account viewer, for *display* of owner controls.
///
/// This is the same identity `viewer_identity()` resolves, projected back to a
/// bare `user_id`: `Some(user_id)` for a `local` channel viewer, `None` for
/// anonymous. Filtering itself lives in the store query; this is used only to
/// decide whether to render author-only UI affordances.
#[cfg(feature = "server")]
#[must_use]
pub fn viewer_user_id(viewer: &ViewerIdentity) -> Option<i64> {
    match viewer {
        ViewerIdentity::Channel { subscriber_ref, .. } => subscriber_ref.parse::<i64>().ok(),
        ViewerIdentity::Anonymous => None,
    }
}

/// Looks up the seeded `local` channel id, memoizing it for the process.
///
/// The lookup runs at most once per process on the happy path: once the
/// [`OnceLock`] is populated it is returned without touching storage. A storage
/// error leaves the cell empty (the next request retries) and yields `None`,
/// which `viewer_identity` treats as fail-closed.
#[cfg(feature = "server")]
async fn local_channel_id(subscriptions: &dyn SubscriptionStorage) -> Option<i64> {
    if let Some(id) = LOCAL_CHANNEL_ID.get() {
        return Some(*id);
    }
    let id = subscriptions.local_channel_id().await.ok()?;
    // Race-loser's value is identical (the row is immutable), so ignore the Err.
    let _ = LOCAL_CHANNEL_ID.set(id);
    Some(id)
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::{account_viewer, viewer_user_id};
    use common::visibility::ViewerIdentity;

    #[test]
    fn account_viewer_with_channel_is_local() {
        assert_eq!(
            account_viewer(7, Some(3)),
            ViewerIdentity::local(7, 3),
            "a resolved local channel yields a Channel viewer keyed by the user id",
        );
    }

    #[test]
    fn account_viewer_without_channel_fails_closed_to_anonymous() {
        assert_eq!(
            account_viewer(7, None),
            ViewerIdentity::Anonymous,
            "an unresolved local channel must fail closed to Anonymous",
        );
    }

    #[test]
    fn viewer_user_id_projects_local_channel_to_user_id() {
        assert_eq!(viewer_user_id(&ViewerIdentity::local(42, 1)), Some(42));
    }

    #[test]
    fn viewer_user_id_is_none_for_anonymous() {
        assert_eq!(viewer_user_id(&ViewerIdentity::Anonymous), None);
    }
}
