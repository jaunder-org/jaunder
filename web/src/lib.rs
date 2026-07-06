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
pub mod backup;
pub mod email;
pub mod error;
pub mod feed_discovery;
pub mod feed_events;
pub mod invites;
pub mod media;
pub mod pages;
pub mod password_reset;
pub mod posts;
pub mod profile;
pub mod render;
pub mod sessions;
pub mod site;
pub mod subscriptions;
pub mod tags;
#[cfg(all(test, feature = "server"))]
mod test_support;
pub mod viewer;

pub use error::server_resource;
pub use pages::App;

// Only the wasm32 body of `mount_csr` below uses the leptos prelude (the host
// csr build compiles that body out), so the import matches that gate.
#[cfg(all(feature = "csr", target_arch = "wasm32"))]
use leptos::prelude::*;

/// Boot the CSR client (#179). Adopts the public projector's data blob (#178):
/// reads `#jaunder-seed`, drops the projector-painted `#app` container, and
/// mounts [`App`] with the seed in context so the public pages render their
/// first paint from it (no reactive fetch) via the same `render` fn the
/// projector used — coincident, flash-free. On the static SPA shell (no blob,
/// no `#app`) the seed is `None` and this is an ordinary `mount_to_body`.
#[cfg(feature = "csr")]
// cov:ignore-start
pub fn mount_csr() {
    // cov:ignore-stop
    // Browser-only: the CSR client only ever runs in wasm, so the DOM adoption +
    // mount live behind `target_arch = "wasm32"`. On the host build (coverage /
    // tests) this is a no-op — which is also why it stays out of the coverage
    // report, like the rest of `web`'s wasm-only code.
    #[cfg(target_arch = "wasm32")]
    {
        let seed = read_dom_seed();
        if let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("app"))
        {
            // App re-renders the identical content from `seed`, so removing the
            // server-painted copy avoids a duplicate paint without a visible
            // flash (the removal and remount happen in one synchronous task).
            el.remove();
        }
        leptos::mount::mount_to_body(move || {
            provide_context(seed.clone());
            view! { <App /> }
        });
    }
} // cov:ignore

/// Read and deserialize the projector's `#jaunder-seed` JSON blob, if present.
#[cfg(all(feature = "csr", target_arch = "wasm32"))]
fn read_dom_seed() -> Option<render::PageSeed> {
    let json = web_sys::window()?
        .document()?
        .get_element_by_id("jaunder-seed")?
        .text_content()?;
    serde_json::from_str(&json).ok()
}
