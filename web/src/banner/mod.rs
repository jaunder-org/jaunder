//! Shared operational warning banner (`WarnBanner`) — the sticky `role="alert"`
//! `.j-warn-banner` bar used by the backup and site verticals. Wasm-only: the
//! banner is a browser-bound, server-fn-driven component with no host-rendered twin.
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::WarnBanner;
