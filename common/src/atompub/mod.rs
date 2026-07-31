//! Generic `AtomPub` (RFC 5023) wire-format serialization and parsing.
//!
//! This module models `AtomPub` entities **independently of any CMS or storage
//! layer** — it deals only in plain data (strings, enums, small structs). The
//! mapping between these wire types and Jaunder's `Post`/`Media` records lives
//! at a boundary in the `server` crate, not here. The intent is that this
//! module could later be contributed upstream to `atom_syndication` or
//! extracted as a standalone `atompub` crate without dragging Jaunder types
//! along.

mod xml;

pub mod entry;
pub use entry::{
    entry_from_xml, entry_to_xml, is_draft, j_slug, render_feed, render_media_link_entry,
    set_draft, set_j_slug, FeedMeta, MediaLinkEntry,
};

pub mod service;
pub use service::{render_service_document, CollectionDecl, ServiceDocument};

pub mod categories;
pub use categories::render_categories_document;

pub mod rsd;
pub use rsd::render_rsd_document;

/// Re-export of the canonical Atom entry model and its component types,
/// used across the `AtomPub` surface (including the server-side mapping boundary).
pub use atom_syndication::{Category, Content, Entry, Link, Text};

use thiserror::Error;

/// The `AtomPub` Atom namespace URI.
pub const ATOM_NS: &str = "http://www.w3.org/2005/Atom";
/// The Atom Publishing Protocol control namespace URI (RFC 5023 §B).
pub const APP_NS: &str = "http://www.w3.org/2007/app";
/// Jaunder foreign-markup namespace (ADR-0023): `j:slug`, `j:extension`.
pub const J_NS: &str = "https://jaunder.org/ns/atompub";

/// Errors produced when reading or writing `AtomPub` wire documents.
#[derive(Debug, Error)]
pub enum AtomPubError {
    /// The supplied XML could not be parsed as the expected document type.
    /// Attributable to the client, so handlers surface it as a `400`.
    #[error("malformed AtomPub document: {0}")]
    Malformed(String),
    /// The document could not be written. Never the client's fault — handlers
    /// surface it as a `500` rather than blaming the request.
    #[error("failed to serialize AtomPub document: {0}")]
    Serialize(String),
}

impl From<atom_syndication::Error> for AtomPubError {
    /// Reading is the only direction that consults this impl; the writers map
    /// their (unreachable) failures to [`AtomPubError::Serialize`] explicitly,
    /// because landing them here would route a server fault onto the `400` path.
    fn from(e: atom_syndication::Error) -> Self {
        AtomPubError::Malformed(e.to_string())
    }
}

impl From<std::io::Error> for AtomPubError {
    fn from(e: std::io::Error) -> Self {
        AtomPubError::Malformed(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_converts_to_malformed() {
        let err: AtomPubError = std::io::Error::other("boom").into();
        assert!(matches!(err, AtomPubError::Malformed(msg) if msg.contains("boom")));
    }

    #[test]
    fn atom_error_converts_to_malformed() {
        let err: AtomPubError = atom_syndication::Error::InvalidStartTag.into();
        assert!(matches!(err, AtomPubError::Malformed(_)));
    }

    #[test]
    fn serialize_error_displays_its_cause() {
        let err = AtomPubError::Serialize("boom".to_string());
        assert!(err.to_string().contains("boom"));
    }
}
