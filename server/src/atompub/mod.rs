//! Server-side `AtomPub` surface: the boundary mapping Jaunder posts/media to
//! `AtomPub` wire types, plus the HTTP handlers.

pub mod mapping;
pub mod media;
pub mod posts;
pub mod rsd;
pub mod service;

mod error;
mod guards;
mod router;

pub use error::HandlerError;
// `base_url` is deliberately absent: `required_base_url`, its only caller, sits
// beside it in `guards.rs`, so re-exporting it here would be an import nothing
// consumes — and `unused_imports` is denied. It keeps its `pub(crate)` on the
// definition, so no visibility narrowed; only an unreachable path went away.
pub(crate) use guards::{require_user_match, required_base_url};
pub use router::router;
