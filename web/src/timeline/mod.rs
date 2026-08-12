//! The timeline vertical (#329, ADR-0070): the cursor-paginated listing
//! endpoints and the shared timeline machinery used by the public Local timeline
//! (`home`), the authed `/app` cockpit, and the user/tag pages. Module wiring
//! only.
//!
//! The `#[server]` listing fns live in the `api` leaf and their host-only storage
//! queries in `server` — the vertical owns both so that `(vertical, ident)` is a
//! key the compiler enforces (#714). The wire types they exchange
//! (`TimelinePage`/`RenderedPost`) are defined in `common::seed` and
//! reached through `crate::posts`.
//!
//! Alongside them sit the pure host-tested `state` and `render` leaves and the
//! wasm-only reactive `component`. `state` holds the reactive
//! `TimelineState` signal bundle as well as the pure value model (#671) — both
//! host-compiled and coverage-measured, the bundle exercised under an `Owner` —
//! leaving `component` only what cannot run on the host (`spawn_local`, `view!`).
//!
//! The `pub use` blocks keep the stable `crate::timeline::…` paths — the ones call
//! sites and the server-fn registrar depend on — and keep the pure items reachable
//! on the host build, where `component` is compiled out.

mod api;

#[cfg(feature = "server")]
mod server;

pub(crate) mod render;
mod state;
pub use state::{LoadStatus, NoIdentity, TimelinePaint, TimelineState};

pub use api::{
    ListByTag, ListByUser, ListByUserAndTag, ListHomeFeed, ListLocalTimeline, list_by_tag,
    list_by_user, list_by_user_and_tag, list_home_feed, list_local_timeline,
};

// Server-only shared fetch helpers, consumed by the `server` crate's public
// projector (one query, no drift).
#[cfg(feature = "server")]
pub use server::{
    fetch_local_timeline, fetch_posts_by_tag, fetch_user_posts, fetch_user_posts_by_tag,
};

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::{TimelineGate, TimelineRows, spawn_load_more, wire_timeline_resolve};
