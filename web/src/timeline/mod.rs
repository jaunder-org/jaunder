//! The timeline vertical (#329, ADR-0070): shared cursor-paginated timeline
//! machinery used by the public Local timeline (`home`) and the authed `/app`
//! cockpit. Module wiring only.
//!
//! A server-less vertical — no `#[server]` fns or wire types of its own (it
//! re-uses `crate::posts::{TimelinePage, TimelinePostSummary, PostCard}`), so
//! there is no `api.rs`/`server.rs`: only the pure host-tested `state` and the
//! wasm-only reactive `component`. The `pub use` keeps the pure items reachable
//! on the host build, where `component` is compiled out.

mod state;
pub use state::{LoadStatus, TimelineCursor};

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::{spawn_load_more, TimelineRows, TimelineState};
