//! The timeline vertical (#329, ADR-0070): shared cursor-paginated timeline
//! machinery used by the public Local timeline (`home`) and the authed `/app`
//! cockpit. Module wiring only.
//!
//! A server-less vertical — no `#[server]` fns or wire types of its own (it
//! re-uses `crate::posts::{TimelinePage, TimelinePostSummary, PostCard}`), so
//! there is no `api.rs`/`server.rs`: the host-tested `state` and `render` leaves,
//! and the wasm-only `component`. Since #671 `state` holds the reactive
//! `TimelineState` signal bundle as well as the pure value model — both
//! host-compiled and coverage-measured, the bundle exercised under an `Owner` —
//! leaving `component` only what cannot run on the host (`spawn_local`, `view!`).
//! The `pub use` keeps those items reachable on the host build, where `component`
//! is compiled out.

pub(crate) mod render;
mod state;
pub use state::{LoadStatus, TimelineCursor, TimelineState};

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::{spawn_load_more, TimelineRows};
