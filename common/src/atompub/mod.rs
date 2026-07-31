//! Generic `AtomPub` (RFC 5023) wire-format serialization and parsing.
//!
//! This module models `AtomPub` entities **independently of any CMS or storage
//! layer** — it deals only in plain data (strings, enums, small structs). The
//! mapping between these wire types and Jaunder's `Post`/`Media` records lives
//! at a boundary in the `server` crate, not here.
//!
//! The Atom documents themselves are `atom_syndication`'s. **Reading is entirely
//! upstream's** — a consumer calls `Entry::from_str` and maps [`AtomError`] at its
//! own boundary; this module deliberately offers no parse wrapper, because one
//! would be a rename of `parse` and nothing more. Writing goes through [`entry`],
//! which composes the documents whose shape is ours: the collection `<feed>` with
//! its RFC 5005 paging links, and the RFC 5023 §9.6 media-link entry.
//!
//! What this module still serializes by hand is the part of RFC 5023 that is *not*
//! Atom: the service document, the categories document, and RSD — none of which
//! upstream models.

mod xml;

pub mod entry;
pub use entry::{
    entry_to_xml, is_draft, j_slug, render_feed, render_media_link_entry, set_draft, set_j_slug,
    FeedMeta, MediaLinkEntry,
};

pub mod service;
pub use service::{render_service_document, CollectionDecl, ServiceDocument};

pub mod categories;
pub use categories::render_categories_document;

pub mod rsd;
pub use rsd::render_rsd_document;

/// Re-export of the canonical Atom entry model and its component types, used
/// across the `AtomPub` surface (including the server-side mapping boundary).
///
/// Reading a document is `Entry::from_str` / `Feed::read_from` directly — there is
/// no wrapper. [`AtomError`] comes along so a consumer can map a parse failure onto
/// its own error type (the server turns it into a `400`).
pub use atom_syndication::{Category, Content, Entry, Error as AtomError, Link, Text};

use thiserror::Error;

/// The `AtomPub` Atom namespace URI.
pub const ATOM_NS: &str = "http://www.w3.org/2005/Atom";
/// The Atom Publishing Protocol control namespace URI (RFC 5023 §B).
pub const APP_NS: &str = "http://www.w3.org/2007/app";
/// Jaunder foreign-markup namespace (ADR-0023): `j:slug`, `j:extension`.
pub const J_NS: &str = "https://jaunder.org/ns/atompub";

/// An `AtomPub` document could not be **written**.
///
/// There is deliberately no read counterpart. `atom_syndication` owns parsing, so
/// a document the client sent that will not parse surfaces as [`AtomError`] and
/// each consumer maps it at its own boundary (the server: a `400`). Failing to
/// write a document we composed ourselves is never the client's fault, so the two
/// directions are separate types rather than two variants of one — which is what
/// keeps a serialization failure off the `400` path.
#[derive(Debug, Error)]
#[error("failed to serialize AtomPub document: {0}")]
pub struct AtomPubError(String);

impl AtomPubError {
    /// Wraps the cause of a failed write.
    #[must_use]
    pub fn new(cause: impl Into<String>) -> Self {
        Self(cause.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_error_displays_its_cause() {
        assert!(AtomPubError::new("boom").to_string().contains("boom"));
    }
}
