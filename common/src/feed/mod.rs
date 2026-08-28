//! Dual-target Syndication Feed URL grammar used by CSR discovery and host producers.
//!
//! Host-only feed identities, settings, events, models, and serializers live in
//! `host::feed`; this module retains the representation and surface vocabulary
//! required to build the same canonical discovery URLs on both targets.

pub mod grammar;
pub use grammar::{FeedFormat, FeedSurface, canonicalize};
