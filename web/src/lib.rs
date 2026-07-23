// Leptos `view!` trees compile to deeply-nested tuple types; the editor's
// composer view (now carrying the audience picker) exceeds the default type
// recursion limit, so raise it for this crate.
#![recursion_limit = "512"]

#[cfg(feature = "server")]
pub use common::username;

/// Wraps a `#[server]` function body in [`error::server_boundary`]: runs the
/// async block, and on an `InternalError` logs the structured failure (kind,
/// class, cause chain) and maps it to the public `WebError` so operator detail
/// never reaches the client. `$name` is the server-fn label used in that log and
/// in the error metric. Every server function routes its body through this.
#[macro_export]
macro_rules! boundary {
    ($name:expr, $body:block) => {
        $crate::error::server_boundary($name, async move $body).await
    };
}

pub mod audiences;
pub mod auth;
pub mod avatar;
pub mod backup;
pub mod cockpit;
pub mod email;
pub mod error;
pub mod feed_discovery;
pub mod feed_events;
pub mod forms;
pub mod home;
pub mod icon;
pub mod invites;
#[cfg(feature = "server")]
mod mail;
pub mod media;
#[cfg(target_arch = "wasm32")]
pub mod pages;
pub mod password_reset;
pub mod posts;
pub mod profile;
pub mod reactive;
pub mod registration;
pub mod render;
/// The `~`-only permalink route segment (#592). Pure `leptos_router` matching logic
/// (no `web_sys`), so it lives at the crate root — host-compiled and host-tested —
/// rather than under the wasm-only `pages` module that consumes it.
pub mod route_segments;
pub mod sessions;
pub mod site;
pub mod subscriptions;
pub mod taglist;
pub mod tags;
#[cfg(all(test, feature = "server"))]
mod test_support;
pub mod timeline;
pub mod topbar;
pub mod viewer;

#[cfg(target_arch = "wasm32")]
pub use pages::App;
