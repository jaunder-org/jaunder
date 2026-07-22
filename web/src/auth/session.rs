//! The shared reactive session context (#591, ADR-0044): one marker-seeded
//! `SessionUser` signal plus a per-navigation reconcile against [`session`].
//! Supersedes the ad-hoc `current_user()` fetches that each component used to
//! spin. wasm-only reactive glue — the `session()` server fn itself lives in
//! [`super::api`]; this module is the client context around it.

use leptos::prelude::*;
use leptos_router::hooks::use_location;

use super::{marker_storage, session, SessionUser};
use crate::error::WebResult;

/// The viewer/session identity shared across the app tree.
#[derive(Clone, Copy)]
pub struct SessionContext {
    /// The live session: seeded synchronously from the advisory marker at mount
    /// (flash-free) and kept current by `reconcile` / `set_session` / `clear_session`.
    /// Synchronously readable for chrome.
    pub current: RwSignal<Option<SessionUser>>,
    /// Per-navigation server confirmation. Awaiting it yields the authoritative
    /// (cookie-checked) session for gates that must not trust a stale marker.
    pub reconcile: Resource<WebResult<Option<SessionUser>>>,
}

/// Provide the session context. Seeds from the marker synchronously, then
/// reconciles against `session()` on every navigation, writing the result back
/// into the `current` signal AND the marker (so the next boot stays flash-free).
/// ADR-0044 D3. Must be called from inside `<Router>` (it reads `use_location`) —
/// `AppShell` is that owner, and every consumer is a descendant of it.
pub fn provide_session_context() {
    let current = RwSignal::new(marker_storage::get());
    let location = use_location();
    let reconcile = Resource::new(move || location.pathname.get(), |_| session());
    Effect::new(move |_| {
        if let Some(Ok(next)) = reconcile.get() {
            match &next {
                Some(user) => marker_storage::set(user),
                None => marker_storage::remove(),
            }
            if current.get_untracked() != next {
                current.set(next);
            }
        }
    });
    provide_context(SessionContext { current, reconcile });
}

/// The shared session context. Panics if called outside the provider — every
/// consumer renders under `AppShell`, which provides it.
#[must_use]
pub fn use_session() -> SessionContext {
    expect_context::<SessionContext>()
}

/// Optimistically set the session (login/register) — `current` signal + marker, so
/// the chrome flips without waiting for the reconcile round-trip.
pub fn set_session(user: SessionUser) {
    let ctx = use_session();
    marker_storage::set(&user);
    ctx.current.set(Some(user));
}

/// Optimistically clear the session (logout) — `current` signal + marker.
pub fn clear_session() {
    let ctx = use_session();
    marker_storage::remove();
    ctx.current.set(None);
}
