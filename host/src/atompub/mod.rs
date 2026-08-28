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
//! Atom: the service document and RSD — neither of which upstream models.

pub mod title;
mod xml;
pub use title::{CollectionFeedTitle, CollectionTitle, WorkspaceTitle};

pub mod entry;
pub use entry::{
    FeedMeta, MediaLinkEntry, draft_marker, entry_to_xml, is_draft, j_slug, render_feed,
    render_media_link_entry, set_draft, set_j_slug,
};

pub mod service;
pub use service::{CollectionAccept, CollectionDecl, ServiceDocument, render_service_document};

pub mod rsd;
pub use rsd::render_rsd_document;

/// Re-export of the canonical Atom entry model and its component types, used
/// across the `AtomPub` surface (including the server-side mapping boundary).
///
/// Reading a document is `Entry::from_str` / `Feed::read_from` directly — there is
/// no wrapper. [`AtomError`] comes along so a consumer can map a parse failure onto
/// its own error type (the server turns it into a `400`).
pub use atom_syndication::{Category, Content, Entry, Error as AtomError, Link, Text};

mod ns;
pub use ns::{APP_NS, ATOM_NS, J_NS};

mod error;
pub use error::AtomPubError;
