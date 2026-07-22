//! The **auth** vertical: the `#[server]` session endpoints (`current_user`,
//! `login`, `logout`) in [`api`] and the co-located reactive UI (`LoginPage`,
//! `LogoutPage`) in [`component`], plus the advisory client auth marker. Account
//! creation lives in the sibling `registration` vertical, which establishes the
//! new user's session through this vertical's `set_session_cookie`.
//!
//! This module is **wiring only** (ADR-0070, amended #530): module declarations
//! and re-exports, no items of its own. The UI is wasm-only ([`component`],
//! `#[cfg(target_arch = "wasm32")]`) and never host-compiles.
//!
//! ## Authorization
//! Server fns derive the caller's identity server-side via `require_auth`
//! (`server::require_auth`), never from a client-supplied argument. The
//! localStorage marker is **advisory** (#181, ADR-0044) — it tunes pre-paint
//! chrome and is never a credential; the real session is the HTTP-only cookie.

/// The advisory auth-marker **codec** (#181, ADR-0044): pure `encode`/`decode` +
/// `MARKER_KEY`, host-tested and cfg-free. The wasm-only browser binding lives in
/// [`marker_storage`].
pub mod marker;

/// Browser `localStorage` binding of the auth marker (wasm-only): `get`/`set`/
/// `remove` over [`client::storage`] + the [`marker`] codec (#514).
#[cfg(target_arch = "wasm32")]
pub mod marker_storage;

mod api;
/// The wasm-only auth UI (`LoginPage`, `LogoutPage`) — never host-compiled
/// (ADR-0070); calls the marker binding and `client::` directly.
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(feature = "server")]
mod server;
/// The wasm-only shared session context (#591): the marker-seeded `SessionUser`
/// signal + per-navigation `session()` reconcile. Calls the wasm-only
/// `marker_storage`, and every consumer is itself wasm-only.
#[cfg(target_arch = "wasm32")]
mod session;

// The API surface — re-exported so external call sites and the server-fn
// registrar keep the stable `crate::auth::…` paths despite living in `api.rs`.
pub use api::{
    current_user, login, logout, session, CurrentUser, Login, LoginResponse, Logout, Session,
};
#[cfg(target_arch = "wasm32")]
pub use component::{LoginPage, LogoutPage};
pub use marker::SessionUser;
#[cfg(target_arch = "wasm32")]
pub use session::{
    clear_session, provide_session_context, set_session, use_session, SessionContext,
};

// Public re-exports — must remain accessible as crate::auth::* for other modules.
#[cfg(feature = "server")]
pub use server::{require_auth, AuthRejection, AuthUser, CookieSettings};
// Exposed for the sibling `registration` vertical, which logs a new user in after
// creating their account.
#[cfg(feature = "server")]
pub(crate) use server::set_session_cookie;
