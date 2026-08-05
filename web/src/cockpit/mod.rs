//! The cockpit vertical (#317, ADR-0070): the routed `/app` authed-only
//! personalized Feed (#181, ADR-0044 D6). Module wiring only — a server-less
//! vertical (no `api.rs`/`server.rs`); its `component` composes `crate::auth`,
//! `crate::posts`, `crate::timeline`, and the shared `crate::topbar`. Since #306 the
//! page's reactive bundle and its load fold live in the host-compiled,
//! coverage-measured [`state`] leaf (ADR-0083), leaving `component` only the
//! `Effect` and the `view!`.

mod state;
// Exported rather than `pub(crate)`: the wasm-only `component` is the only caller, so
// a crate-internal item would read as dead code on the host build, where that leaf is
// compiled out.
pub use state::{CockpitLoad, CockpitState, resolve_initial_page};

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::CockpitPage;
