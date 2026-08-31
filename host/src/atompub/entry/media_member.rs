//! `AtomPub` media Member model and rendering.
//!
//! This leaf creates RFC 5023 §9.6 media-link Entries while retaining distinct
//! typed URL roles through construction and upstream serialization.

use atom_syndication::{Content, Entry, Text};

use super::super::AtomPubError;
use super::render::{rel_link, to_xml_string};
use common::media::{ContentType, Filename};
use common::tagged_url::{ContentSrcUrl, EditMediaUriUrl, EditUriUrl, EntryIdUrl};
use common::time::UtcInstant;

/// A media-link entry (RFC 5023 §9.6): the Atom `<entry>` a server returns for
/// an uploaded media resource. Its `<content>` references the binary by `src`
/// rather than embedding it, and it carries both an `edit` link (the member)
/// and an `edit-media` link (the binary).
///
/// `edit_uri` (the member) and `edit_media_uri` (the binary) carry distinct roles, so
/// transposing them is a compile error rather than an entry whose `edit` link overwrites
/// the binary (#875):
///
/// ```compile_fail
/// # use host::atompub::entry::MediaLinkEntry;
/// # fn f(a: MediaLinkEntry, b: MediaLinkEntry) -> MediaLinkEntry {
/// MediaLinkEntry { edit_uri: b.edit_media_uri, edit_media_uri: b.edit_uri, ..a }
/// # }
/// ```
///
/// The correct assignment compiles — same fixture, so the negative above can only be
/// failing for the transposition:
///
/// ```
/// # use host::atompub::entry::MediaLinkEntry;
/// # fn f(a: MediaLinkEntry, b: MediaLinkEntry) -> MediaLinkEntry {
/// MediaLinkEntry { edit_uri: b.edit_uri, edit_media_uri: b.edit_media_uri, ..a }
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct MediaLinkEntry {
    /// Stable entry id — the absolute member IRI (#560, require-base).
    pub id: EntryIdUrl,
    /// The uploaded media's filename, in its **canonical** (percent-encoded) spelling —
    /// the same value the member URLs carry. The renderer decodes it for the entry's
    /// human-readable `<title>`; it is not stored decoded (#720).
    pub title: Filename,
    /// `rel="edit"` href — the media-link member resource.
    pub edit_uri: EditUriUrl,
    /// `rel="edit-media"` href — the binary media resource.
    pub edit_media_uri: EditMediaUriUrl,
    /// `<content src=...>` — the absolute URL of the binary.
    pub content_src: ContentSrcUrl,
    /// MIME type of the binary.
    pub content_type: ContentType,
    /// Publication timestamp.
    pub published: UtcInstant,
    /// Last-update timestamp.
    pub updated: UtcInstant,
}

/// Serializes a [`MediaLinkEntry`] to a standalone `<entry>` document.
///
/// # Errors
///
/// Returns [`AtomPubError`] if the document cannot be written.
pub fn render_media_link_entry(entry: &MediaLinkEntry) -> Result<String, AtomPubError> {
    let atom_entry = Entry {
        id: entry.id.to_string(),
        // The one display surface in this document: the title shows the name the user
        // typed, while every URL below keeps the canonical stored spelling (#720).
        title: Text::plain(entry.title.decoded()),
        updated: entry.updated.value().fixed_offset(),
        published: Some(entry.published.value().fixed_offset()),
        // A media-link entry references the binary rather than embedding it, so the
        // content carries `src` and no value (RFC 5023 §9.6).
        content: Some(Content {
            content_type: Some(entry.content_type.to_string()),
            src: Some(entry.content_src.to_string()),
            ..Default::default()
        }),
        links: vec![
            rel_link("edit", &entry.edit_uri),
            rel_link("edit-media", &entry.edit_media_uri),
        ],
        ..Default::default()
    };

    to_xml_string(atom_entry.write_to(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use common::test_support::{parse_content_type, parse_filename, parse_url, parse_utc_instant};

    /// The media-link sibling of [`sample_entry`]: a PNG member, whose fields the
    /// individual tests override where the case calls for it.
    fn sample_media_link_entry() -> MediaLinkEntry {
        MediaLinkEntry {
            id: parse_url("https://h/atompub/alice/media/abc/pic.png"),
            title: parse_filename("pic.png"),
            edit_uri: parse_url("https://h/atompub/alice/media/abc/pic.png"),
            edit_media_uri: parse_url("https://h/media/upload/ab/c0/abc/pic.png"),
            content_src: parse_url("https://h/media/upload/ab/c0/abc/pic.png"),
            content_type: parse_content_type("image/png"),
            published: parse_utc_instant("2026-06-01T00:00:00Z"),
            updated: parse_utc_instant("2026-06-01T00:00:00Z"),
        }
    }

    #[test]
    fn media_link_entry_timestamps_are_serialized_as_rfc3339_utc() {
        let out = render_media_link_entry(&MediaLinkEntry {
            updated: parse_utc_instant("2026-06-02T00:00:00Z"),
            ..sample_media_link_entry()
        })
        .expect("serialize");
        assert!(out.contains("<published>2026-06-01T00:00:00"), "out: {out}");
        assert!(out.contains("<updated>2026-06-02T00:00:00"), "out: {out}");
    }

    #[test]
    fn render_media_link_entry_references_binary_by_src() {
        let out = render_media_link_entry(&sample_media_link_entry()).expect("serialize");

        assert!(out.contains("<entry"), "out: {out}");
        // Upstream writes a paired `<content>`, not a self-closing form — a
        // deliberate, consumer-checked delta (ADR-0089).
        assert!(out.contains(r#"<content type="image/png""#), "out: {out}");
        assert!(
            out.contains("src=\"https://h/media/upload/ab/c0/abc/pic.png\""),
            "out: {out}"
        );
        assert!(out.contains("</content>"), "out: {out}");
        assert!(out.contains("rel=\"edit-media\""), "out: {out}");
        assert!(out.contains("rel=\"edit\""), "out: {out}");
        assert!(out.contains(">pic.png<"), "out: {out}");
    }

    #[test]
    fn media_link_entry_title_is_the_decoded_filename() {
        // The `<title>` is the entry's human-readable name, so it shows the name the user
        // typed — not the canonical stored spelling the URLs carry (#720). A missed decode
        // here is invisible to type checking, which is why it is asserted directly.
        let out = render_media_link_entry(&MediaLinkEntry {
            id: parse_url("https://h/atompub/alice/media/abc/my%20photo.jpg"),
            title: parse_filename("my%20photo.jpg"),
            edit_uri: parse_url("https://h/atompub/alice/media/abc/my%20photo.jpg"),
            edit_media_uri: parse_url("https://h/media/upload/ab/c0/abc/my%20photo.jpg"),
            content_src: parse_url("https://h/media/upload/ab/c0/abc/my%20photo.jpg"),
            content_type: parse_content_type("image/jpeg"),
            ..sample_media_link_entry()
        })
        .expect("serialize");

        assert!(out.contains("<title>my photo.jpg</title>"), "out: {out}");
        // The URLs keep the canonical spelling — one value, two views.
        assert!(out.contains("/my%20photo.jpg\""), "out: {out}");
    }
}
