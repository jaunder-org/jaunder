// Leptos `view!` trees compile to deeply-nested tuple types; the editor's
// composer view exceeds the default type recursion limit, so raise it for this
// crate.
#![recursion_limit = "512"]

#[cfg(feature = "server")]
pub use common::username;

pub mod app;
pub mod audiences;
pub mod auth;
pub mod avatar;
pub mod backup;
pub mod banner;
pub mod cockpit;
pub mod email;
pub mod error;
pub mod feed_discovery;
pub mod forms;
pub mod home;
/// `Markup` — the render layer's currency and its one trusted-HTML door. Every
/// pure builder returns it; escaping is maud's job, not the caller's.
mod html;
pub mod icon;
pub mod invites;
#[cfg(feature = "server")]
mod mail;
pub mod media;
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) mod mutation_feedback;
pub mod password_reset;
pub mod posts;
pub mod profile;
pub mod reactive;
pub mod registration;
/// The `~`-only permalink route segment (#592). Pure `leptos_router` matching logic
/// (no `web_sys`), so it lives at the crate root — host-compiled and host-tested —
/// rather than under the wasm-only `app` module that consumes it.
pub mod route_segments;
pub mod sessions;
pub mod sidebar;
pub mod site;
pub mod subscriptions;
pub mod taglist;
pub mod tags;
#[cfg(all(test, feature = "server"))]
mod test_support;
pub mod timeline;
pub mod topbar;
pub mod viewer;
