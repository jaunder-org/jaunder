//! The cockpit vertical (#317, ADR-0070): the routed `/app` authed-only
//! personalized Feed (#181, ADR-0044 D6). Module wiring only — a server-less
//! vertical (no `api.rs`/`server.rs`); its `component` composes `crate::auth`,
//! `crate::posts`, `crate::timeline`, and the shared `crate::topbar`.

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::CockpitPage;
