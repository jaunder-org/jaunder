//! Feed / RSD auto-discovery `<link>` tags and their pure URL/label helpers.
#[cfg(target_arch = "wasm32")]
mod component;
// Pure label/URL helpers, host-tested. Compiled only where actually used — the
// wasm component and the host test build — since the projector duplicates the
// label logic (`render::feed_label`) rather than calling `surface_label`, so a
// plain `mod labels;` would be dead code on the non-test host lib build.
#[cfg(any(target_arch = "wasm32", test))]
mod labels;

#[cfg(target_arch = "wasm32")]
pub use component::{FeedDiscovery, RsdDiscovery};
