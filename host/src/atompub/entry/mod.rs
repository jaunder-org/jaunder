//! `AtomPub` Entry document surface.
//!
//! This facade preserves the established `host::atompub::entry` API while
//! private leaves own marker bookkeeping and each Atom document shape. Atom XML
//! I/O remains delegated to `atom_syndication` (ADR-0089); Jaunder-owned
//! `app:draft` and `j:slug` semantics remain in `foreign_markers` (ADR-0023).

mod collection;
mod entry_document;
mod foreign_markers;
mod media_member;
mod render;

pub use collection::{FeedMeta, render_feed};
pub use entry_document::entry_to_xml;
pub use foreign_markers::{draft_marker, is_draft, j_slug, set_draft, set_j_slug};
pub use media_member::{MediaLinkEntry, render_media_link_entry};
