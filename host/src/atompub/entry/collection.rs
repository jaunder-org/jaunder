//! `AtomPub` Collection Feed model and rendering.
//!
//! This leaf assembles collection metadata and member entries into the upstream
//! Atom Feed model; shared XML and link construction remain private siblings.

use std::collections::BTreeMap;

use atom_syndication::{Entry, Feed, Text};

use super::super::{AtomPubError, CollectionFeedTitle, ns};
use super::render::{rel_link, to_xml_string};
use common::tagged_url::{EntryIdUrl, FeedUrl, PaginationUrl};
use common::time::UtcInstant;

/// Feed-level metadata for an `AtomPub` collection document.
///
/// Used to wrap multiple entries in a `<feed>` with RFC 5005 paging links.
#[derive(Debug, Clone)]
pub struct FeedMeta {
    /// Stable feed id — the absolute collection IRI (#560, require-base).
    pub id: EntryIdUrl,
    /// Human-readable collection title.
    pub title: CollectionFeedTitle,
    /// Feed `updated` timestamp.
    pub updated: UtcInstant,
    /// `rel="self"` href (the absolute collection URL for this page).
    pub self_url: FeedUrl,
    /// `rel="first"` href, when paging.
    pub first: Option<PaginationUrl>,
    /// `rel="next"` href, when a next page exists.
    pub next: Option<PaginationUrl>,
    /// `rel="previous"` href, when a previous page exists.
    pub previous: Option<PaginationUrl>,
}

/// Serializes a collection `<feed>` wrapping the given entries, with RFC 5005
/// paging links.
///
/// The `<feed>` root declares `xmlns` plus the `app` and `j` prefixes; the
/// embedded entries inherit the default namespace from it rather than
/// redeclaring it.
///
/// # Errors
///
/// Returns [`AtomPubError`] if the document cannot be written.
pub fn render_feed(meta: &FeedMeta, entries: &[Entry]) -> Result<String, AtomPubError> {
    let mut links = vec![rel_link("self", &meta.self_url)];
    // Pagination links emit in a fixed order: first, previous, next.
    for (rel, href) in [
        ("first", &meta.first),
        ("previous", &meta.previous),
        ("next", &meta.next),
    ] {
        if let Some(href) = href.as_ref() {
            links.push(rel_link(rel, href));
        }
    }

    let feed = Feed {
        id: meta.id.to_string(),
        title: Text::plain(meta.title.to_string()),
        updated: meta.updated.value().fixed_offset(),
        links,
        entries: entries.to_vec(),
        namespaces: BTreeMap::from([
            ("app".to_string(), ns::APP_NS.to_string()),
            ("j".to_string(), ns::J_NS.to_string()),
        ]),
        ..Default::default()
    };

    to_xml_string(feed.write_to(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use atom_syndication::Text;

    use super::super::foreign_markers::set_j_slug;
    use common::test_support::{parse_url, parse_utc_instant};

    fn sample_entry() -> Entry {
        Entry {
            id: "tag:example.com,2026:post/1".to_string(),
            title: Text::plain("Hello"),
            updated: chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z").unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn render_feed_wraps_entries_with_paging() {
        let mut entry1 = sample_entry();
        entry1.id = "tag:example.com,2026:post/1".to_string();
        entry1.title = Text::plain("First");

        let mut entry2 = sample_entry();
        entry2.id = "tag:example.com,2026:post/2".to_string();
        entry2.title = Text::plain("Second");

        let meta = FeedMeta {
            id: parse_url("https://example.com/atompub/alice/posts"),
            title: CollectionFeedTitle::posts(&"alice".parse().unwrap()),
            updated: parse_utc_instant("2026-05-31T12:00:00Z"),
            self_url: parse_url("https://example.com/atompub/alice/posts"),
            first: Some(parse_url("https://example.com/atompub/alice/posts?page=1")),
            next: Some(parse_url("https://example.com/atompub/alice/posts?page=2")),
            previous: Some(parse_url("https://example.com/atompub/alice/posts?page=0")),
        };

        let out = render_feed(&meta, &[entry1, entry2]).expect("serialize");

        // Feed structure and metadata
        assert!(out.contains("<feed"), "out: {out}");
        assert!(out.contains("xmlns:app"), "out: {out}");
        assert!(
            out.contains("<title>alice&apos;s posts</title>"),
            "out: {out}"
        );
        assert!(
            out.contains("xmlns=\"http://www.w3.org/2005/Atom\""),
            "out: {out}"
        );

        // Paging links
        assert!(out.contains("rel=\"self\""), "out: {out}");
        assert!(out.contains("rel=\"first\""), "out: {out}");
        assert!(out.contains("rel=\"next\""), "out: {out}");
        assert!(out.contains("rel=\"previous\""), "out: {out}");
        assert!(out.contains("page=2"), "out: {out}");
        assert!(out.contains("page=0"), "out: {out}");

        // Entry titles present — and exactly one <entry> per input, since the titles
        // alone would still pass if an entry were dropped or duplicated.
        assert!(out.contains(">First<"), "out: {out}");
        assert!(out.contains(">Second<"), "out: {out}");
        assert_eq!(out.matches("<entry").count(), 2, "out: {out}");

        // Embedded entries should NOT redeclare the Atom namespace on their own
        // They should not have xmlns="..." as an attribute on the entry element
        let entry_with_xmlns = out.contains("<entry xmlns=\"");
        assert!(
            !entry_with_xmlns,
            "Entries should not redeclare xmlns; out: {out}"
        );

        // A marker-bearing entry *does* carry its own prefix declaration, because the
        // writer emits `Entry::namespaces` whether or not the entry is embedded. The
        // redundancy with the feed root is valid XML and is the accepted delta.
        let mut marked = sample_entry();
        set_j_slug(&mut marked, "my-post");
        let out = render_feed(&meta, &[marked]).expect("serialize");
        assert!(out.contains("<entry xmlns:j="), "out: {out}");

        // Feed closing tag present
        assert!(out.contains("</feed>"), "out: {out}");
    }

    #[test]
    fn render_feed_without_paging_omits_optional_links() {
        let mut entry = sample_entry();
        entry.title = Text::plain("Single");

        let meta = FeedMeta {
            id: parse_url("https://example.com/atompub/bob/posts"),
            title: CollectionFeedTitle::posts(&"bob".parse().unwrap()),
            updated: parse_utc_instant("2026-05-31T13:00:00Z"),
            self_url: parse_url("https://example.com/atompub/bob/posts"),
            first: None,
            next: None,
            previous: None,
        };

        let out = render_feed(&meta, &[entry]).expect("serialize");

        // Required elements present
        assert!(out.contains("<feed"), "out: {out}");
        assert!(
            out.contains("<title>bob&apos;s posts</title>"),
            "out: {out}"
        );
        assert!(out.contains("rel=\"self\""), "out: {out}");

        // Optional paging links absent
        assert!(!out.contains("rel=\"next\""), "out: {out}");
        assert!(!out.contains("rel=\"previous\""), "out: {out}");
        assert!(!out.contains("rel=\"first\""), "out: {out}");

        // Entry present
        assert!(out.contains(">Single<"), "out: {out}");
    }

    #[test]
    fn feed_meta_updated_is_serialized_as_rfc3339_utc() {
        let meta = FeedMeta {
            id: parse_url("https://example.com/atompub/alice/posts"),
            title: CollectionFeedTitle::posts(&"alice".parse().unwrap()),
            updated: parse_utc_instant("2026-05-31T12:00:00Z"),
            self_url: parse_url("https://example.com/atompub/alice/posts"),
            first: None,
            next: None,
            previous: None,
        };
        let out = render_feed(&meta, &[]).expect("serialize");
        assert!(out.contains("2026-05-31T12:00:00"), "out: {out}");
    }
}
