//! Public Syndication Feed discovery: head links and a visible contextual index.
#[cfg(target_arch = "wasm32")]
mod component;
// Pure label/URL helpers, host-tested. Compiled only where actually used — the
// wasm component and the host test build — since the projector duplicates the
// label logic (`render::feed_label`) rather than calling `surface_label`, so a
// plain `mod labels;` would be dead code on the non-test host lib build.
#[cfg(any(target_arch = "wasm32", test))]
mod labels;
pub mod render;
pub mod routes;

#[cfg(target_arch = "wasm32")]
pub use component::{ConfirmedUserTag, FeedDiscovery, FeedIndexPage, RsdDiscovery};
