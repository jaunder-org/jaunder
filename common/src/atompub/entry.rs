//! Standalone Atom entry (`<entry>`) and collection `<feed>` read/write for `AtomPub`.
//!
//! The data model *and* the XML I/O are `atom_syndication`'s: `Entry::read_from`
//! and `Entry::write_to` handle the standalone `<entry>` documents `AtomPub`
//! exchanges (POST to create, PUT to edit, GET a member), and `Feed::write_to`
//! handles the collection. Bare-entry I/O was crate-private until
//! `atom_syndication` 0.12.10, which is why this module once carried its own
//! `quick-xml` reader and writers.
//!
//! What remains here is what upstream does not model: the Atom Publishing
//! Protocol control element `app:control/app:draft` (RFC 5023 §B) and jaunder's
//! own `j:slug` (ADR-0023), both stored in the entry's extension map and reached
//! through [`is_draft`] / [`set_draft`] / [`j_slug`] / [`set_j_slug`] — plus the
//! two wire structs, [`FeedMeta`] and [`MediaLinkEntry`], that describe documents
//! assembled from more than one Atom element.
//!
//! Each marker helper also owns its namespace prefix in `Entry::namespaces`,
//! because that map is what the writer turns into `xmlns:*` declarations: a
//! prefix is declared exactly while the marker it labels is present.

use std::collections::BTreeMap;

use atom_syndication::extension::Extension;
use atom_syndication::{Content, Entry, Feed, Link, Text};

use super::{AtomPubError, APP_NS, J_NS};
use crate::absolute_url::AbsoluteUrl;
use crate::media::{ContentType, Filename};
use crate::time::UtcInstant;

// ---------------------------------------------------------------------------
// Draft flag (app:control/app:draft) helpers
// ---------------------------------------------------------------------------

/// Returns true when the entry carries `app:control/app:draft = yes`.
#[must_use]
pub fn is_draft(entry: &Entry) -> bool {
    entry.extensions.values().any(|elements| {
        elements
            .get("control")
            .is_some_and(|controls| controls.iter().any(control_marks_draft))
    })
}

fn control_marks_draft(control: &Extension) -> bool {
    control.children.iter().any(|(name, drafts)| {
        (name == "draft" || name.ends_with(":draft"))
            && drafts.iter().any(|d| {
                d.value
                    .as_deref()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("yes"))
            })
    })
}

/// Sets or clears the `app:control/app:draft` marker on an entry.
pub fn set_draft(entry: &mut Entry, draft: bool) {
    // Idempotent replace: strip any existing control element (whatever namespace
    // prefix it was parsed under) and drop the now-empty extension maps, so
    // toggling the draft flag never leaves a stale or duplicate marker behind.
    // The canonical `app`-prefixed control is re-added below only when draft.
    for elements in entry.extensions.values_mut() {
        elements.remove("control");
    }
    entry.extensions.retain(|_, elements| !elements.is_empty());
    // The writer emits `xmlns:*` from `Entry::namespaces`, so the declaration has to
    // track the extensions that need it. Only drop `xmlns:app` once *no* `app:`
    // extension is left — a parsed entry may carry others besides the control we
    // just removed, and undeclaring their prefix would emit unbound-prefix XML.
    if !entry.extensions.contains_key("app") {
        entry.namespaces.remove("app");
    }

    if draft {
        let draft_ext = Extension {
            name: "app:draft".to_string(),
            value: Some("yes".to_string()),
            attrs: BTreeMap::new(),
            children: BTreeMap::new(),
        };
        let mut children = BTreeMap::new();
        children.insert("app:draft".to_string(), vec![draft_ext]);
        let control = Extension {
            name: "app:control".to_string(),
            value: None,
            attrs: BTreeMap::new(),
            children,
        };
        entry
            .extensions
            .entry("app".to_string())
            .or_default()
            .insert("control".to_string(), vec![control]);
        entry
            .namespaces
            .insert("app".to_string(), APP_NS.to_string());
    }
}

// ---------------------------------------------------------------------------
// Slug marker (j:slug) helpers
// ---------------------------------------------------------------------------

/// Read the read-only server slug from a `j:slug` extension, if present.
#[must_use]
pub fn j_slug(entry: &Entry) -> Option<String> {
    entry.extensions.values().find_map(|by_local| {
        by_local
            .get("slug")
            .and_then(|exts| exts.first())
            .and_then(|e| e.value.clone())
    })
}

/// Set (idempotently replace) the `j:slug` extension. Emitted on every outgoing
/// entry; the server never reads an incoming one.
pub fn set_j_slug(entry: &mut Entry, slug: &str) {
    // Idempotent replace: drop any existing slug (whatever prefix it was parsed
    // under) and prune now-empty extension maps, then re-add under the canonical
    // `j` prefix — so re-setting never leaves a stale or duplicate marker behind.
    for by_local in entry.extensions.values_mut() {
        by_local.remove("slug");
    }
    entry.extensions.retain(|_, by_local| !by_local.is_empty());
    let ext = Extension {
        name: "j:slug".to_string(),
        value: Some(slug.to_string()),
        attrs: BTreeMap::new(),
        children: BTreeMap::new(),
    };
    entry
        .extensions
        .entry("j".to_string())
        .or_default()
        .insert("slug".to_string(), vec![ext]);
    // As with `set_draft`, this helper owns its prefix's `xmlns:j` declaration.
    // There is no clearing counterpart: a slug is set on every outgoing entry.
    entry.namespaces.insert("j".to_string(), J_NS.to_string());
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parses a standalone `AtomPub` `<entry>` document into an [`Entry`].
///
/// Server-owned fields a client omits (id, dates, links) are simply left at
/// their defaults; this reader captures whatever the document provides.
///
/// # Errors
///
/// Returns [`AtomPubError::Malformed`] when the bytes are not a well-formed
/// `<entry>` document, when the root element is not `<entry>`, or when a field
/// upstream validates — a timestamp, or an atomTextConstruct's `type` — is
/// out of spec.
pub fn entry_from_xml(xml: &str) -> Result<Entry, AtomPubError> {
    Ok(xml.parse::<Entry>()?)
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

/// Decodes what an `atom_syndication` writer produced into a `String`.
///
/// Both failure modes are unreachable in practice — a `Vec<u8>` has no I/O to
/// fail, and upstream emits UTF-8 — but neither is expressible as infallible, so
/// they surface as [`AtomPubError::Serialize`]. That variant, not `Malformed`,
/// is what keeps a server-side write failure off the client-facing `400` path.
fn to_xml_string(
    written: Result<Vec<u8>, atom_syndication::Error>,
) -> Result<String, AtomPubError> {
    let bytes = written.map_err(|e| AtomPubError::Serialize(e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| AtomPubError::Serialize(e.to_string()))
}

/// Serializes an [`Entry`] to a standalone `AtomPub` `<entry>` document.
///
/// Emits whatever the entry carries: id, title, dates, summary, content (with
/// its `type`), all links, all categories, and any extension markers — the
/// draft flag and `j:slug` among them.
///
/// # Errors
///
/// Returns [`AtomPubError::Serialize`] if the document cannot be written.
pub fn entry_to_xml(entry: &Entry) -> Result<String, AtomPubError> {
    to_xml_string(entry.write_to(Vec::new()))
}

/// Feed-level metadata for an `AtomPub` collection document.
///
/// Used to wrap multiple entries in a `<feed>` with RFC 5005 paging links.
#[derive(Debug, Clone)]
pub struct FeedMeta {
    /// Stable feed id — the absolute collection IRI (#560, require-base).
    pub id: AbsoluteUrl,
    /// Human-readable collection title.
    pub title: String,
    /// Feed `updated` timestamp.
    pub updated: UtcInstant,
    /// `rel="self"` href (the absolute collection URL for this page).
    pub self_url: AbsoluteUrl,
    /// `rel="first"` href, when paging.
    pub first: Option<AbsoluteUrl>,
    /// `rel="next"` href, when a next page exists.
    pub next: Option<AbsoluteUrl>,
    /// `rel="previous"` href, when a previous page exists.
    pub previous: Option<AbsoluteUrl>,
}

/// Builds a `rel`-labelled [`Link`] — feed paging links and entry `edit` links alike.
fn rel_link(rel: &str, href: &AbsoluteUrl) -> Link {
    Link {
        rel: rel.to_string(),
        href: href.to_string(),
        ..Default::default()
    }
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
/// Returns [`AtomPubError::Serialize`] if the document cannot be written.
pub fn render_feed(meta: &FeedMeta, entries: &[Entry]) -> Result<String, AtomPubError> {
    let mut links = vec![rel_link("self", &meta.self_url)];
    // Order matches the previous hand-rolled writer: first, previous, next.
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
        title: Text::plain(meta.title.clone()),
        updated: meta.updated.value().fixed_offset(),
        links,
        entries: entries.to_vec(),
        namespaces: BTreeMap::from([
            ("app".to_string(), APP_NS.to_string()),
            ("j".to_string(), J_NS.to_string()),
        ]),
        ..Default::default()
    };

    to_xml_string(feed.write_to(Vec::new()))
}

/// A media-link entry (RFC 5023 §9.6): the Atom `<entry>` a server returns for
/// an uploaded media resource. Its `<content>` references the binary by `src`
/// rather than embedding it, and it carries both an `edit` link (the member)
/// and an `edit-media` link (the binary).
#[derive(Debug, Clone)]
pub struct MediaLinkEntry {
    /// Stable entry id — the absolute member IRI (#560, require-base).
    pub id: AbsoluteUrl,
    /// The uploaded media's filename, in its **canonical** (percent-encoded) spelling —
    /// the same value the member URLs carry. The renderer decodes it for the entry's
    /// human-readable `<title>`; it is not stored decoded (#720).
    pub title: Filename,
    /// `rel="edit"` href — the media-link member resource.
    pub edit_uri: AbsoluteUrl,
    /// `rel="edit-media"` href — the binary media resource.
    pub edit_media_uri: AbsoluteUrl,
    /// `<content src=...>` — the absolute URL of the binary.
    pub content_src: AbsoluteUrl,
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
/// Returns [`AtomPubError::Serialize`] if the document cannot be written.
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

/// What these tests are for, now that `atom_syndication` owns the XML.
///
/// They do **not** re-verify upstream's parser or writer — that is upstream's
/// job, and duplicating it here would just pin someone else's implementation.
/// What they cover is our side of the boundary:
///
/// - the `app:control/app:draft` and `j:slug` markers, and the `xmlns:*`
///   bookkeeping that goes with them;
/// - the documents assembled from *our* wire structs (`FeedMeta`,
///   `MediaLinkEntry`);
/// - each behaviour delta the migration deliberately accepted, so a future
///   upstream bump that changes one fails here instead of on a client;
/// - the error class a malformed document maps to.
#[cfg(test)]
mod tests {
    use super::*;
    use atom_syndication::Category;

    use crate::test_support::{
        parse_absolute_url, parse_content_type, parse_filename, parse_utc_instant,
    };

    /// The `(type, value)` of an entry's `<content>`. Every caller supplies one, so
    /// an absent element is a broken test rather than a case to report.
    fn content_parts(entry: &Entry) -> (Option<&str>, Option<&str>) {
        let content = entry.content().expect("entry carries <content>");
        (content.content_type(), content.value())
    }

    #[test]
    fn parses_draft_html_entry_with_category() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>Hello</title>
  <summary>sum</summary>
  <content type="html">&lt;p&gt;hi&lt;/p&gt;</content>
  <category term="rust"/>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        assert_eq!(entry.title().as_str(), "Hello");
        assert_eq!(entry.summary().map(Text::as_str), Some("sum"));
        assert_eq!(content_parts(&entry), (Some("html"), Some("<p>hi</p>")));
        assert_eq!(entry.categories().len(), 1);
        assert_eq!(entry.categories()[0].term(), "rust");
        assert!(is_draft(&entry));
    }

    #[test]
    fn an_unsupported_entity_is_passed_through_literally() {
        // The one *loosening* delta of the atom_syndication migration: upstream
        // resolves predefined entities, then char refs, then re-emits anything it
        // cannot resolve verbatim — where the previous hand-rolled reader rejected
        // the document. Lenient ingest, consistent with how `mapping.rs` drops a
        // single bad category term rather than failing the entry (R5).
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>x&bogus;y</title>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        assert_eq!(entry.title().as_str(), "x&bogus;y");
    }

    #[test]
    fn rejects_an_unparseable_updated() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <updated>not-a-date</updated>
</entry>"#;
        assert!(matches!(
            entry_from_xml(xml),
            Err(AtomPubError::Malformed(_))
        ));
    }

    #[test]
    fn rejects_a_title_type_outside_the_text_constructs() {
        // RFC 4287 restricts an atomTextConstruct's `type` to text|html|xhtml.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title type="text/markdown">T</title>
</entry>"#;
        assert!(matches!(
            entry_from_xml(xml),
            Err(AtomPubError::Malformed(_))
        ));
    }

    #[test]
    fn a_media_type_on_content_is_still_accepted() {
        // The ADR-0023 format carrier. `atomContent` — unlike an atomTextConstruct —
        // explicitly permits a media type, so the stricter parsing above must not
        // reach it. This is the test that would catch org/markdown support breaking.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <content type="text/org">* heading</content>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        assert_eq!(content_parts(&entry), (Some("text/org"), Some("* heading")));
    }

    #[test]
    fn a_prefixed_atom_title_does_not_populate_the_title() {
        // Accepted narrowing: upstream matches qualified names, so a prefixed child
        // lands in the extension map instead of the title. Documented in the ADR as
        // an interop cost of deleting the bespoke local-name-matching reader.
        let xml = r#"<entry xmlns:atom="http://www.w3.org/2005/Atom">
  <atom:title>T</atom:title>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        assert_ne!(entry.title().as_str(), "T");
    }

    #[test]
    fn xhtml_content_escapes_literal_apostrophes() {
        // Accepted delta: upstream escapes text events inside xhtml content, so a
        // bare `'` becomes `&apos;`. Semantically identical once parsed.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>X</title>
  <content type="xhtml"><div>it's</div></content>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        let (_, value) = content_parts(&entry);
        let value = value.expect("xhtml value");
        assert!(value.contains("&apos;"), "value: {value}");

        // …and survives the serialize leg, so the escaped form is what a client
        // actually receives — not just what we happen to hold in memory.
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(out.contains("&apos;"), "out: {out}");
    }

    #[test]
    fn xhtml_entity_references_survive_the_round_trip() {
        // NOT a delta: the previous reader resolved `&amp;` to `&` and then re-escaped
        // it on the way into the stored string, so both readers store `&amp;`.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>X</title>
  <content type="xhtml"><div>b &amp; c</div></content>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        let (_, value) = content_parts(&entry);
        assert!(value.expect("xhtml value").contains("&amp;"));
    }

    #[test]
    fn document_without_entry_is_an_error() {
        assert!(matches!(
            entry_from_xml("<?xml version=\"1.0\"?><other/>"),
            Err(AtomPubError::Malformed(_))
        ));
    }

    fn sample_entry() -> Entry {
        Entry {
            id: "tag:example.com,2026:post/1".to_string(),
            title: Text::plain("Hello"),
            updated: chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z").unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn set_and_read_j_slug_round_trips() {
        let mut entry = sample_entry();
        set_j_slug(&mut entry, "my-post");
        assert_eq!(j_slug(&entry), Some("my-post".to_string()));
    }

    #[test]
    fn re_setting_a_marker_replaces_rather_than_accumulates() {
        // Both the extension map and `namespaces` are keyed maps, so a repeated set
        // overwrites its key rather than appending a second marker or a duplicate
        // xmlns declaration.
        let mut entry = sample_entry();
        set_j_slug(&mut entry, "first");
        set_j_slug(&mut entry, "second");
        set_draft(&mut entry, true);
        set_draft(&mut entry, true);

        assert_eq!(j_slug(&entry), Some("second".to_string()));
        let out = entry_to_xml(&entry).expect("serialize");
        assert_eq!(out.matches("<j:slug>").count(), 1, "out: {out}");
        assert_eq!(out.matches("app:control").count(), 2, "out: {out}"); // open + close
        assert_eq!(out.matches("xmlns:j=").count(), 1, "out: {out}");
        assert_eq!(out.matches("xmlns:app=").count(), 1, "out: {out}");
    }

    #[test]
    fn j_slug_is_serialized_with_namespace() {
        let mut entry = sample_entry();
        set_j_slug(&mut entry, "my-post");
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains(r#"xmlns:j="https://jaunder.org/ns/atompub""#),
            "out: {out}"
        );
        assert!(out.contains("<j:slug>my-post</j:slug>"), "out: {out}");
    }

    #[test]
    fn an_entry_with_no_markers_declares_neither_prefix() {
        // A prefix is declared exactly while the marker it labels is present, so a
        // plain entry carries neither declaration.
        let entry = sample_entry();
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(!out.contains("xmlns:j"), "out: {out}");
        assert!(!out.contains("xmlns:app"), "out: {out}");
    }

    #[test]
    fn serializes_text_entry_with_links() {
        let mut entry = sample_entry();
        entry.summary = Some(Text::plain("sum"));
        entry.content = Some(Content {
            content_type: Some("text".to_string()),
            value: Some("# md".to_string()),
            ..Default::default()
        });
        entry.categories = vec![Category {
            term: "rust".to_string(),
            ..Default::default()
        }];
        entry.links = vec![
            Link {
                rel: "edit".to_string(),
                href: "https://h/atompub/alice/posts/1".to_string(),
                ..Default::default()
            },
            Link {
                rel: "alternate".to_string(),
                href: "https://h/~alice/p".to_string(),
                ..Default::default()
            },
        ];
        entry.published =
            Some(chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap());

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(out.contains("type=\"text\""), "out: {out}");
        assert!(out.contains("rel=\"edit\""), "out: {out}");
        assert!(out.contains("rel=\"alternate\""), "out: {out}");
        assert!(out.contains("# md"), "out: {out}");
        assert!(out.contains("term=\"rust\""), "out: {out}");
        assert!(out.contains("<published>"), "out: {out}");
        assert!(!out.contains("app:draft"), "out: {out}");
    }

    #[test]
    fn serializes_draft_entry_with_app_control_and_escapes_html() {
        let mut entry = sample_entry();
        set_draft(&mut entry, true);
        entry.content = Some(Content {
            content_type: Some("html".to_string()),
            value: Some("<p>x</p>".to_string()),
            ..Default::default()
        });
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(out.contains("app:draft"), "out: {out}");
        assert!(out.contains("yes"), "out: {out}");
        assert!(out.contains("type=\"html\""), "out: {out}");
        assert!(out.contains("&lt;p&gt;x&lt;/p&gt;"), "out: {out}");
    }

    #[test]
    fn draft_and_html_round_trip_through_serialize_then_parse() {
        let mut entry = sample_entry();
        entry.title = Text::plain("RT");
        entry.summary = Some(Text::plain("s"));
        entry.content = Some(Content {
            content_type: Some("html".to_string()),
            value: Some("<p>body & more</p>".to_string()),
            ..Default::default()
        });
        entry.categories = vec![
            Category {
                term: "a".to_string(),
                ..Default::default()
            },
            Category {
                term: "b".to_string(),
                ..Default::default()
            },
        ];
        entry.links = vec![Link {
            rel: "edit".to_string(),
            href: "https://h/atompub/alice/posts/1".to_string(),
            ..Default::default()
        }];
        entry.published =
            Some(chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap());
        set_draft(&mut entry, true);
        set_j_slug(&mut entry, "my-post");

        let out = entry_to_xml(&entry).expect("serialize");
        let parsed = entry_from_xml(&out).expect("re-parse");
        assert!(is_draft(&parsed), "draft flag lost; xml: {out}");
        assert_eq!(parsed.title().as_str(), "RT");
        assert_eq!(parsed.summary().map(Text::as_str), Some("s"));
        assert_eq!(parsed.links().len(), 1);
        assert_eq!(parsed.links()[0].rel(), "edit");
        assert_eq!(parsed.links()[0].href(), "https://h/atompub/alice/posts/1");
        assert_eq!(
            parsed
                .published()
                .map(chrono::DateTime::to_rfc3339)
                .as_deref(),
            Some("2026-01-01T00:00:00+00:00")
        );
        assert_eq!(
            parsed.updated().to_rfc3339(),
            entry.updated().to_rfc3339(),
            "updated lost; xml: {out}"
        );
        // The slug had no reader arm at all before this migration, so nothing
        // pinned it round-tripping.
        assert_eq!(j_slug(&parsed), Some("my-post".to_string()));
        assert_eq!(
            content_parts(&parsed),
            (Some("html"), Some("<p>body & more</p>"))
        );
        let terms: Vec<&str> = parsed.categories().iter().map(Category::term).collect();
        assert_eq!(terms, vec!["a", "b"]);
    }

    #[test]
    fn set_draft_false_clears_existing_marker() {
        let mut entry = sample_entry();
        set_draft(&mut entry, true);
        assert!(is_draft(&entry));
        set_draft(&mut entry, false);
        assert!(!is_draft(&entry));
        assert!(!entry_to_xml(&entry)
            .expect("serialize")
            .contains("app:draft"));
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
            id: parse_absolute_url("https://example.com/atompub/alice/posts"),
            title: "Alice's Posts".to_string(),
            updated: parse_utc_instant("2026-05-31T12:00:00Z"),
            self_url: parse_absolute_url("https://example.com/atompub/alice/posts"),
            first: Some(parse_absolute_url(
                "https://example.com/atompub/alice/posts?page=1",
            )),
            next: Some(parse_absolute_url(
                "https://example.com/atompub/alice/posts?page=2",
            )),
            previous: Some(parse_absolute_url(
                "https://example.com/atompub/alice/posts?page=0",
            )),
        };

        let out = render_feed(&meta, &[entry1, entry2]).expect("serialize");

        // Feed structure and metadata
        assert!(out.contains("<feed"), "out: {out}");
        assert!(out.contains("xmlns:app"), "out: {out}");
        assert!(out.contains("Alice"), "out: {out}");
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
            id: parse_absolute_url("https://example.com/atompub/bob/posts"),
            title: "Bob's Posts".to_string(),
            updated: parse_utc_instant("2026-05-31T13:00:00Z"),
            self_url: parse_absolute_url("https://example.com/atompub/bob/posts"),
            first: None,
            next: None,
            previous: None,
        };

        let out = render_feed(&meta, &[entry]).expect("serialize");

        // Required elements present
        assert!(out.contains("<feed"), "out: {out}");
        assert!(out.contains("Bob"), "out: {out}");
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
            id: parse_absolute_url("https://example.com/atompub/alice/posts"),
            title: "Alice's Posts".to_string(),
            updated: parse_utc_instant("2026-05-31T12:00:00Z"),
            self_url: parse_absolute_url("https://example.com/atompub/alice/posts"),
            first: None,
            next: None,
            previous: None,
        };
        let out = render_feed(&meta, &[]).expect("serialize");
        assert!(out.contains("2026-05-31T12:00:00"), "out: {out}");
    }

    /// The media-link sibling of [`sample_entry`]: a PNG member, whose fields the
    /// individual tests override where the case calls for it.
    fn sample_media_link_entry() -> MediaLinkEntry {
        MediaLinkEntry {
            id: parse_absolute_url("https://h/atompub/alice/media/abc/pic.png"),
            title: parse_filename("pic.png"),
            edit_uri: parse_absolute_url("https://h/atompub/alice/media/abc/pic.png"),
            edit_media_uri: parse_absolute_url("https://h/media/upload/ab/c0/abc/pic.png"),
            content_src: parse_absolute_url("https://h/media/upload/ab/c0/abc/pic.png"),
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
        // Upstream writes a paired `<content>`, not the self-closing form the previous
        // hand-rolled writer emitted — a deliberate, consumer-checked delta.
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
            id: parse_absolute_url("https://h/atompub/alice/media/abc/my%20photo.jpg"),
            title: parse_filename("my%20photo.jpg"),
            edit_uri: parse_absolute_url("https://h/atompub/alice/media/abc/my%20photo.jpg"),
            edit_media_uri: parse_absolute_url("https://h/media/upload/ab/c0/abc/my%20photo.jpg"),
            content_src: parse_absolute_url("https://h/media/upload/ab/c0/abc/my%20photo.jpg"),
            content_type: parse_content_type("image/jpeg"),
            ..sample_media_link_entry()
        })
        .expect("serialize");

        assert!(out.contains("<title>my photo.jpg</title>"), "out: {out}");
        // The URLs keep the canonical spelling — one value, two views.
        assert!(out.contains("/my%20photo.jpg\""), "out: {out}");
    }

    #[test]
    fn draft_entry_declares_the_app_namespace_and_clearing_removes_it() {
        let mut entry = sample_entry();
        set_draft(&mut entry, true);
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains(r#"xmlns:app="http://www.w3.org/2007/app""#),
            "out: {out}"
        );

        set_draft(&mut entry, false);
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(!out.contains("xmlns:app"), "out: {out}");
        assert!(!out.contains("app:draft"), "out: {out}");
    }
}
