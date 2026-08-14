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
// Public only so the integration contract can inject an identity-store failure
// at the narrowing seam; application handlers consume it within this module.
pub(crate) use guards::require_user_match;
pub use guards::required_base_url;
pub use router::router;
