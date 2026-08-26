//! Writing the Atom documents `AtomPub` exchanges: the standalone `<entry>`
//! (POST to create, PUT to edit, GET a member), the collection `<feed>`, and the
//! RFC 5023 §9.6 media-link entry.
//!
//! The data model *and* the XML are `atom_syndication`'s — `Entry::write_to` and
//! `Feed::write_to` do the serializing (ADR-0089).
//!
//! **There is no reader here.** Parsing is `Entry::from_str` at the call site;
//! a wrapper would be a rename of `parse` and nothing else.
//!
//! What remains is what upstream does not model: the Atom Publishing Protocol
//! control element `app:control/app:draft` (RFC 5023 §B) and jaunder's own
//! `j:slug` (ADR-0023), both stored in the entry's extension map and reached
//! through [`is_draft`] / [`set_draft`] / [`j_slug`] / [`set_j_slug`] — plus the
//! two wire structs, [`FeedMeta`] and [`MediaLinkEntry`], describing documents
//! assembled from more than one Atom element.
//!
//! Each marker helper also owns its namespace prefix in `Entry::namespaces`,
//! because that map is what the writer turns into `xmlns:*` declarations: a
//! prefix is declared exactly while the marker it labels is present. That map is
//! also the *only* thing that says which namespace a prefix named, so every read
//! and write resolves through it rather than matching `app`/`j` literally — see
//! the namespace-bookkeeping helpers below.

use std::collections::BTreeMap;

use atom_syndication::extension::Extension;
use atom_syndication::{Content, Entry, Feed, Link, Text};

use super::title::CollectionFeedTitle;
use super::{APP_NS, AtomPubError, J_NS};
use crate::media::{ContentType, Filename};
use crate::tagged_url::{
    ContentSrcUrl, EditMediaUriUrl, EditUriUrl, EntryIdUrl, FeedUrl, PaginationUrl, TaggedUrl,
    UrlRole,
};
use crate::time::UtcInstant;

// ---------------------------------------------------------------------------
// Namespace bookkeeping
// ---------------------------------------------------------------------------
//
// Upstream's extension map is keyed by the *prefix* a marker was parsed under,
// not by its namespace, and an extension's children are keyed by local name
// alone. So a prefix is meaningless on its own: what it named is whatever
// `xmlns:` declaration was in scope, and only an extension's qualified `name`
// says which prefix a child carried. Every read and write below resolves through
// these helpers rather than matching a prefix literally, so a document that
// spells `app` or `j` for some unrelated namespace is neither misread as ours nor
// trampled.
//
// Two scopes carry declarations. `Entry::namespaces` holds the entry element's.
// A declaration made on the marker element itself survives as an *attribute*:
// upstream keys attributes by local name, so `xmlns:app="…"` on an `<app:control>`
// lands as `attrs["app"]`. (An ordinary attribute named `app` whose value is
// exactly the namespace URI is indistinguishable from that — a collision we
// accept, since the alternative is not reading a scoped declaration at all.)

/// Resolves `prefix` through an innermost-first chain of namespace scopes — an
/// element's own declarations shadow its parent's, which shadow the entry's.
fn resolve<'a>(scopes: &[&'a BTreeMap<String, String>], prefix: &str) -> Option<&'a str> {
    scopes
        .iter()
        .find_map(|scope| scope.get(prefix))
        .map(String::as_str)
}

/// Whether a child extension's qualified name carries a prefix resolving to `uri`,
/// given the declarations its own element, its parent and the entry supply.
///
/// An unprefixed name is in the default namespace — Atom, in these documents —
/// and so is never one of ours.
fn child_in_namespace(
    namespaces: &BTreeMap<String, String>,
    parent_attrs: &BTreeMap<String, String>,
    child: &Extension,
    uri: &str,
) -> bool {
    child.name.split_once(':').is_some_and(|(prefix, _)| {
        resolve(&[&child.attrs, parent_attrs, namespaces], prefix) == Some(uri)
    })
}

/// The entry's `uri`-namespaced extensions with local name `local`, paired with the
/// prefix each was spelled under — under whatever prefix (or prefixes) the document
/// used, and whichever scope declared it.
fn markers_in<'a>(
    entry: &'a Entry,
    uri: &'a str,
    local: &'a str,
) -> impl Iterator<Item = (&'a str, &'a Extension)> {
    entry
        .extensions
        .iter()
        .flat_map(move |(prefix, by_local)| {
            by_local
                .get(local)
                .into_iter()
                .flatten()
                .map(move |ext| (prefix.as_str(), ext))
        })
        .filter(move |(prefix, ext)| resolve(&[&ext.attrs, &entry.namespaces], prefix) == Some(uri))
}

/// The prefix to write a `uri`-namespaced `local` marker under: the one already
/// labelling the entry's own such marker, so a re-set lands in the element the
/// entry already has rather than beside it; else [`writable_prefix`].
fn marker_prefix(entry: &Entry, uri: &str, local: &str, preferred: &str) -> String {
    markers_in(entry, uri, local)
        .next()
        .map_or_else(|| writable_prefix(entry, uri, preferred), |(p, _)| p.into())
}

/// The prefix to declare a `uri` marker under when the entry has none: its existing
/// binding for the namespace, else `preferred` — or a numbered variant of it when
/// some *other* namespace already owns that spelling, since redeclaring it would
/// silently relabel that namespace's markers.
fn writable_prefix(entry: &Entry, uri: &str, preferred: &str) -> String {
    if let Some((prefix, _)) = entry.namespaces.iter().find(|(_, bound)| *bound == uri) {
        return prefix.clone();
    }
    // A prefix some other extension is spelled with is spoken for even when the
    // entry element doesn't declare it — the declaration may be on that element.
    let taken = |p: &str| entry.namespaces.contains_key(p) || entry.extensions.contains_key(p);
    if !taken(preferred) {
        return preferred.to_string();
    }
    let mut n = 1;
    loop {
        let candidate = format!("{preferred}{n}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Drops the `xmlns:` declaration of every `uri`-bound prefix that is left
/// labelling nothing.
///
/// The writer emits `Entry::namespaces` verbatim, so a declaration has to track
/// the extensions that need it: a stale one is noise on the wire, and dropping
/// one too early would emit unbound-prefix XML.
fn prune_bindings(entry: &mut Entry, uri: &str) {
    let extensions = &entry.extensions;
    entry
        .namespaces
        .retain(|prefix, bound| bound != uri || extensions.contains_key(prefix));
}

/// Drops every extension map left empty by a removal, so `prune_bindings` and
/// `extensions_in` see a prefix exactly while it still labels something.
fn prune_empty_extensions(entry: &mut Entry) {
    entry.extensions.retain(|_, by_local| !by_local.is_empty());
}

// ---------------------------------------------------------------------------
// Draft flag (app:control/app:draft) helpers
// ---------------------------------------------------------------------------

/// Returns the explicit `app:control/app:draft` marker when present.
///
/// `Some(true)` is RFC 5023's `yes` value; any other explicit marker value is
/// `Some(false)`. This preserves the distinction between an explicit
/// non-draft marker and no marker, which callers that merge lifecycle sources
/// need. Multiple valid markers retain [`is_draft`]'s established meaning:
/// `yes` wins.
#[must_use]
pub fn draft_marker(entry: &Entry) -> Option<bool> {
    markers_in(entry, APP_NS, "control")
        .filter_map(|(_, control)| control_draft_marker(&entry.namespaces, control))
        .reduce(|draft, marker| draft || marker)
}

/// Returns true when the entry carries `app:control/app:draft = yes`.
#[must_use]
pub fn is_draft(entry: &Entry) -> bool {
    draft_marker(entry).unwrap_or(false)
}

fn control_draft_marker(
    namespaces: &BTreeMap<String, String>,
    control: &Extension,
) -> Option<bool> {
    let drafts = control.children.get("draft")?;
    let mut found = false;
    let mut is_draft = false;
    for draft in drafts {
        if child_in_namespace(namespaces, &control.attrs, draft, APP_NS) {
            found = true;
            is_draft |= draft
                .value
                .as_deref()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("yes"));
        }
    }
    found.then_some(is_draft)
}

/// Sets or clears the `app:control/app:draft` marker on an entry.
///
/// `app:draft` is the only child RFC 5023 defines for `app:control`, but it is
/// explicitly not the only one that may appear there, so this clears the draft
/// child alone: any other control child the document carried survives, and the
/// `app:control` wrapper is dropped only once nothing is left inside it.
pub fn set_draft(entry: &mut Entry, draft: bool) {
    // Reuse the prefix the entry's own control element is spelled with, so a document
    // that spells APP another way keeps its spelling and the flag lands *in* that
    // control — RFC 5023 §B gives an entry one `app:control`, not one per prefix.
    // Chosen *before* the removal, which prunes a binding once nothing is left under
    // it — including the one naming that spelling.
    let prefix = draft.then(|| marker_prefix(entry, APP_NS, "control", "app"));
    remove_draft_marker(entry);

    if let Some(prefix) = prefix {
        let draft_ext = Extension {
            name: format!("{prefix}:draft"),
            value: Some("yes".to_string()),
            ..Extension::default()
        };
        let by_local = entry.extensions.entry(prefix.clone()).or_default();
        let mut controls = by_local.remove("control").unwrap_or_default();
        match controls.first_mut() {
            Some(control) => {
                control
                    .children
                    .insert("draft".to_string(), vec![draft_ext]);
                // The prefix is declared on the entry element below, so a declaration
                // this element carried is redundant — and upstream would re-emit it as
                // a plain attribute rather than an `xmlns:`, which is worse than none.
                control
                    .attrs
                    .retain(|key, value| key != &prefix || value != APP_NS);
            }
            None => controls.push(Extension {
                name: format!("{prefix}:control"),
                children: BTreeMap::from([("draft".to_string(), vec![draft_ext])]),
                ..Extension::default()
            }),
        }
        by_local.insert("control".to_string(), controls);
        entry.namespaces.insert(prefix, APP_NS.to_string());
    }
}

/// Strips the draft child from every `app:control` the entry carries, whatever
/// prefix that namespace was spelled with, so setting the flag replaces rather
/// than accumulates and clearing it leaves nothing stale behind.
fn remove_draft_marker(entry: &mut Entry) {
    let namespaces = &entry.namespaces;
    for (declared, by_local) in &mut entry.extensions {
        let Some(controls) = by_local.get_mut("control") else {
            continue;
        };
        controls.retain_mut(|control| {
            if resolve(&[&control.attrs, namespaces], declared) != Some(APP_NS) {
                return true;
            }
            let parent_attrs = &control.attrs;
            if let Some(drafts) = control.children.get_mut("draft") {
                drafts.retain(|d| !child_in_namespace(namespaces, parent_attrs, d, APP_NS));
            }
            control.children.retain(|_, children| !children.is_empty());
            // A control element that carried nothing but the draft flag has said all
            // it has to say; one with other children, a value, or an attribute beyond
            // its own namespace declaration has not.
            !control.children.is_empty()
                || control.value.is_some()
                || control
                    .attrs
                    .iter()
                    .any(|(key, value)| key != declared || value != APP_NS)
        });
        by_local.retain(|_, exts| !exts.is_empty());
    }
    prune_empty_extensions(entry);
    prune_bindings(entry, APP_NS);
}

// ---------------------------------------------------------------------------
// Slug marker (j:slug) helpers
// ---------------------------------------------------------------------------

/// Read the read-only server slug from a `j:slug` extension, if present.
#[must_use]
pub fn j_slug(entry: &Entry) -> Option<String> {
    markers_in(entry, J_NS, "slug").find_map(|(_, ext)| ext.value.clone())
}

/// Set (idempotently replace) the `j:slug` extension. Emitted on every outgoing
/// entry; the server never reads an incoming one.
pub fn set_j_slug(entry: &mut Entry, slug: &str) {
    // As in `set_draft`: the prefix is chosen before the removal that may prune the
    // binding naming the document's own spelling of the namespace.
    let prefix = marker_prefix(entry, J_NS, "slug", "j");

    // Idempotent replace: drop any slug already in our namespace, then re-add one —
    // so re-setting never leaves a stale or duplicate marker behind. A `slug` in
    // someone else's namespace is not ours to remove.
    let namespaces = &entry.namespaces;
    for (declared, by_local) in &mut entry.extensions {
        if let Some(slugs) = by_local.get_mut("slug") {
            slugs.retain(|ext| resolve(&[&ext.attrs, namespaces], declared) != Some(J_NS));
        }
        by_local.retain(|_, exts| !exts.is_empty());
    }
    prune_empty_extensions(entry);
    prune_bindings(entry, J_NS);

    let ext = Extension {
        name: format!("{prefix}:slug"),
        value: Some(slug.to_string()),
        ..Extension::default()
    };
    // Pushed, not inserted: the prefix may still label a `slug` in someone else's
    // namespace, and replacing the whole vec would be the trampling this avoids.
    entry
        .extensions
        .entry(prefix.clone())
        .or_default()
        .entry("slug".to_string())
        .or_default()
        .push(ext);
    // As with `set_draft`, this helper owns its prefix's `xmlns:j` declaration.
    // There is no clearing counterpart: a slug is set on every outgoing entry.
    entry.namespaces.insert(prefix, J_NS.to_string());
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

/// Decodes what an `atom_syndication` writer produced into a `String`.
///
/// Both failure modes are unreachable in practice — a `Vec<u8>` has no I/O to
/// fail, and upstream emits UTF-8 — but neither is expressible as infallible.
fn to_xml_string(
    written: Result<Vec<u8>, atom_syndication::Error>,
) -> Result<String, AtomPubError> {
    let bytes = written.map_err(AtomPubError::Writer)?;
    String::from_utf8(bytes).map_err(AtomPubError::Utf8)
}

/// Serializes an [`Entry`] to a standalone `AtomPub` `<entry>` document.
///
/// Emits whatever the entry carries: id, title, dates, summary, content (with
/// its `type`), all links, all categories, and any extension markers — the
/// draft flag and `j:slug` among them.
///
/// # Errors
///
/// Returns [`AtomPubError`] if the document cannot be written.
pub fn entry_to_xml(entry: &Entry) -> Result<String, AtomPubError> {
    to_xml_string(entry.write_to(Vec::new()))
}

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

/// Builds a `rel`-labelled [`Link`] — feed paging links and entry `edit` links alike.
///
/// Generic in the role: a link's `rel` attribute is what states which role the href
/// plays, so the renderer accepts any of them.
fn rel_link<T: UrlRole>(rel: &str, href: &TaggedUrl<T>) -> Link {
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
///
/// `edit_uri` (the member) and `edit_media_uri` (the binary) carry distinct roles, so
/// transposing them is a compile error rather than an entry whose `edit` link overwrites
/// the binary (#875):
///
/// ```compile_fail
/// # use common::atompub::entry::MediaLinkEntry;
/// # fn f(a: MediaLinkEntry, b: MediaLinkEntry) -> MediaLinkEntry {
/// MediaLinkEntry { edit_uri: b.edit_media_uri, edit_media_uri: b.edit_uri, ..a }
/// # }
/// ```
///
/// The correct assignment compiles — same fixture, so the negative above can only be
/// failing for the transposition:
///
/// ```
/// # use common::atompub::entry::MediaLinkEntry;
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
/// - each behaviour delta ADR-0089 deliberately accepted, so a future
///   upstream bump that changes one fails here instead of on a client;
/// - the error class a malformed document maps to.
#[cfg(test)]
mod tests {
    use super::*;
    use atom_syndication::Category;

    use crate::test_support::{parse_content_type, parse_filename, parse_url, parse_utc_instant};

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
        let entry = xml.parse::<Entry>().expect("parse");
        assert_eq!(entry.title().as_str(), "Hello");
        assert_eq!(entry.summary().map(Text::as_str), Some("sum"));
        assert_eq!(content_parts(&entry), (Some("html"), Some("<p>hi</p>")));
        assert_eq!(entry.categories().len(), 1);
        assert_eq!(entry.categories()[0].term(), "rust");
        assert!(is_draft(&entry));
    }

    #[test]
    fn an_unsupported_entity_is_passed_through_literally() {
        // The one *loosening* delta ADR-0089 accepts: upstream resolves predefined
        // entities, then char refs, then re-emits anything it cannot resolve
        // verbatim. Lenient ingest, consistent with how `mapping.rs` drops a single
        // bad category term rather than failing the entry (R5).
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>x&bogus;y</title>
</entry>"#;
        let entry = xml.parse::<Entry>().expect("parse");
        assert_eq!(entry.title().as_str(), "x&bogus;y");
    }

    #[test]
    fn rejects_an_unparseable_updated() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <updated>not-a-date</updated>
</entry>"#;
        assert!(xml.parse::<Entry>().is_err());
    }

    #[test]
    fn rejects_a_title_type_outside_the_text_constructs() {
        // RFC 4287 restricts an atomTextConstruct's `type` to text|html|xhtml.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title type="text/markdown">T</title>
</entry>"#;
        assert!(xml.parse::<Entry>().is_err());
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
        let entry = xml.parse::<Entry>().expect("parse");
        assert_eq!(content_parts(&entry), (Some("text/org"), Some("* heading")));
    }

    #[test]
    fn a_prefixed_atom_title_does_not_populate_the_title() {
        // Accepted narrowing (ADR-0089): upstream matches qualified names, so a
        // prefixed child lands in the extension map instead of the title.
        let xml = r#"<entry xmlns:atom="http://www.w3.org/2005/Atom">
  <atom:title>T</atom:title>
</entry>"#;
        let entry = xml.parse::<Entry>().expect("parse");
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
        let entry = xml.parse::<Entry>().expect("parse");
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
        let entry = xml.parse::<Entry>().expect("parse");
        let (_, value) = content_parts(&entry);
        assert!(value.expect("xhtml value").contains("&amp;"));
    }

    #[test]
    fn document_without_entry_is_an_error() {
        assert!("<?xml version=\"1.0\"?><other/>".parse::<Entry>().is_err());
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
        let parsed = out.parse::<Entry>().expect("re-parse");
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
        assert!(
            !entry_to_xml(&entry)
                .expect("serialize")
                .contains("app:draft")
        );
    }

    #[test]
    fn draft_marker_preserves_explicit_non_draft_presence() {
        let absent = r#"<entry xmlns="http://www.w3.org/2005/Atom"><title>T</title></entry>"#
            .parse::<Entry>()
            .expect("parse");
        let explicit_no =
            r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><app:draft>no</app:draft></app:control>
</entry>"#
                .parse::<Entry>()
                .expect("parse");
        let explicit_yes =
            r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#
                .parse::<Entry>()
                .expect("parse");

        assert_eq!(draft_marker(&absent), None);
        assert_eq!(draft_marker(&explicit_no), Some(false));
        assert_eq!(draft_marker(&explicit_yes), Some(true));
        assert!(!is_draft(&explicit_no));
        assert!(is_draft(&explicit_yes));
    }

    #[test]
    fn a_control_element_outside_the_app_namespace_is_not_a_draft_flag() {
        // `app` is a conventional prefix, not a reserved one: the same spelling bound
        // to someone else's namespace is someone else's element.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="https://example.com/ns/other">
  <title>T</title>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#;
        let entry = xml.parse::<Entry>().expect("parse");
        assert!(!is_draft(&entry));
    }

    #[test]
    fn an_unprefixed_draft_child_is_not_the_app_draft_flag() {
        // An unprefixed child is in the default namespace — Atom here, not APP.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><draft>yes</draft></app:control>
</entry>"#;
        let entry = xml.parse::<Entry>().expect("parse");
        assert!(!is_draft(&entry));
    }

    #[test]
    fn clearing_the_draft_flag_leaves_a_foreign_control_element_alone() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="https://example.com/ns/other">
  <title>T</title>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, false);

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(out.contains("app:control"), "out: {out}");
        assert!(out.contains("app:draft"), "out: {out}");
        assert!(
            out.contains(r#"xmlns:app="https://example.com/ns/other""#),
            "out: {out}"
        );
    }

    #[test]
    fn clearing_the_draft_flag_keeps_the_rest_of_the_control_element() {
        // RFC 5023 defines `app:draft` as the only *current* child of `app:control`,
        // not as the only possible one — so clearing the flag clears the flag.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><app:draft>yes</app:draft><app:review>pending</app:review></app:control>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        assert!(is_draft(&entry));
        set_draft(&mut entry, false);
        assert!(!is_draft(&entry));

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains("<app:review>pending</app:review>"),
            "out: {out}"
        );
        assert!(out.contains("app:control"), "out: {out}");
        assert!(!out.contains("app:draft"), "out: {out}");
        // The surviving child still needs its prefix declared.
        assert!(
            out.contains(r#"xmlns:app="http://www.w3.org/2007/app""#),
            "out: {out}"
        );
    }

    #[test]
    fn setting_the_draft_flag_reuses_the_entrys_own_control_element() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><app:review>pending</app:review></app:control>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        assert!(is_draft(&entry));

        let out = entry_to_xml(&entry).expect("serialize");
        assert_eq!(out.matches("app:control").count(), 2, "out: {out}"); // open + close
        assert!(
            out.contains("<app:review>pending</app:review>"),
            "out: {out}"
        );
    }

    #[test]
    fn a_marker_is_written_under_the_prefix_the_entry_already_binds() {
        // The namespace is what matters; the prefix spelling is the document's.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom"
       xmlns:pub="http://www.w3.org/2007/app"
       xmlns:jaunder="https://jaunder.org/ns/atompub">
  <title>T</title>
  <pub:control><pub:draft>no</pub:draft></pub:control>
  <jaunder:slug>old</jaunder:slug>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        set_j_slug(&mut entry, "new");

        assert!(is_draft(&entry));
        assert_eq!(j_slug(&entry), Some("new".to_string()));

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(out.contains("<pub:draft>yes</pub:draft>"), "out: {out}");
        assert!(
            out.contains("<jaunder:slug>new</jaunder:slug>"),
            "out: {out}"
        );
        assert_eq!(out.matches("pub:control").count(), 2, "out: {out}"); // open + close
        assert_eq!(out.matches("jaunder:slug").count(), 2, "out: {out}"); // open + close
        assert!(!out.contains("xmlns:app="), "out: {out}");
        assert!(!out.contains("xmlns:j="), "out: {out}");
    }

    #[test]
    fn a_marker_never_takes_a_prefix_another_namespace_owns() {
        // Redeclaring `app` here would silently relabel `<app:thing>` as APP.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="https://example.com/ns/other">
  <title>T</title>
  <app:thing>x</app:thing>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains(r#"xmlns:app="https://example.com/ns/other""#),
            "out: {out}"
        );
        assert!(out.contains("<app:thing>x</app:thing>"), "out: {out}");
        // …and the draft flag still lands, under a prefix of its own.
        let parsed = out.parse::<Entry>().expect("re-parse");
        assert!(is_draft(&parsed), "out: {out}");
    }

    #[test]
    fn a_marker_skips_past_every_prefix_already_taken() {
        // The numbered fallback has to keep counting: `app1` is spoken for too.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom"
       xmlns:app="https://example.com/ns/other"
       xmlns:app1="https://example.com/ns/another">
  <title>T</title>
  <app:thing>x</app:thing>
  <app1:thing>y</app1:thing>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains(r#"xmlns:app2="http://www.w3.org/2007/app""#),
            "out: {out}"
        );
        assert!(out.contains("<app:thing>x</app:thing>"), "out: {out}");
        assert!(out.contains("<app1:thing>y</app1:thing>"), "out: {out}");
        let parsed = out.parse::<Entry>().expect("re-parse");
        assert!(is_draft(&parsed), "out: {out}");
    }

    #[test]
    fn setting_the_draft_flag_leaves_the_namespaces_other_extensions_alone() {
        // An APP-namespaced extension that simply isn't a control element.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:edited>2026-01-02T00:00:00Z</app:edited>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        assert!(is_draft(&entry));

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains("<app:edited>2026-01-02T00:00:00Z</app:edited>"),
            "out: {out}"
        );
        assert_eq!(out.matches("app:control").count(), 2, "out: {out}"); // open + close
    }

    #[test]
    fn a_marker_declaring_its_namespace_on_its_own_element_is_ours() {
        // A client may scope the declaration to the marker rather than the entry.
        // Upstream keeps it as an attribute, which is the only record of what the
        // prefix meant — matching on `Entry::namespaces` alone would drop both of
        // these on the floor.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <app:control xmlns:app="http://www.w3.org/2007/app"><app:draft>yes</app:draft></app:control>
  <j:slug xmlns:j="https://jaunder.org/ns/atompub">theirs</j:slug>
</entry>"#;
        let entry = xml.parse::<Entry>().expect("parse");
        assert!(is_draft(&entry));
        assert_eq!(j_slug(&entry), Some("theirs".to_string()));
    }

    #[test]
    fn re_setting_an_element_scoped_marker_replaces_it_and_hoists_the_declaration() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <app:control xmlns:app="http://www.w3.org/2007/app"><app:draft>yes</app:draft></app:control>
  <j:slug xmlns:j="https://jaunder.org/ns/atompub">old</j:slug>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        set_j_slug(&mut entry, "new");

        let out = entry_to_xml(&entry).expect("serialize");
        // One of each marker, and the prefixes now declared where the writer can emit
        // them — an element-scoped declaration comes back out as a plain attribute.
        assert_eq!(out.matches("app:control").count(), 2, "out: {out}"); // open + close
        assert_eq!(out.matches("j:slug").count(), 2, "out: {out}"); // open + close
        assert!(
            out.contains(r#"xmlns:app="http://www.w3.org/2007/app""#),
            "out: {out}"
        );
        assert!(
            out.contains(r#"xmlns:j="https://jaunder.org/ns/atompub""#),
            "out: {out}"
        );
        let parsed = out.parse::<Entry>().expect("re-parse");
        assert!(is_draft(&parsed), "out: {out}");
        assert_eq!(j_slug(&parsed), Some("new".to_string()));
    }

    #[test]
    fn the_draft_flag_lands_in_the_entrys_control_whichever_prefix_spells_it() {
        // Both prefixes are APP, but only one labels the entry's control element —
        // RFC 5023 §B gives an entry one `app:control`, not one per prefix.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom"
       xmlns:a="http://www.w3.org/2007/app"
       xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><app:review>pending</app:review></app:control>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        assert!(is_draft(&entry));

        let out = entry_to_xml(&entry).expect("serialize");
        assert_eq!(out.matches(":control").count(), 2, "out: {out}"); // open + close
        assert!(out.contains("<app:draft>yes</app:draft>"), "out: {out}");
    }

    #[test]
    fn a_slug_outside_the_jaunder_namespace_is_neither_read_nor_replaced() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:j="https://example.com/ns/other">
  <title>T</title>
  <j:slug>theirs</j:slug>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        assert_eq!(j_slug(&entry), None);

        set_j_slug(&mut entry, "ours");
        assert_eq!(j_slug(&entry), Some("ours".to_string()));

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(out.contains("<j:slug>theirs</j:slug>"), "out: {out}");
        assert!(
            out.contains(r#"xmlns:j="https://example.com/ns/other""#),
            "out: {out}"
        );
        let parsed = out.parse::<Entry>().expect("re-parse");
        assert_eq!(j_slug(&parsed), Some("ours".to_string()));
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

    #[test]
    fn serialization_writer_failure_retains_typed_source() {
        let error = to_xml_string(Err(atom_syndication::Error::Eof))
            .expect_err("writer failure must propagate");

        let source = std::error::Error::source(&error).expect("writer source");
        assert!(matches!(
            source.downcast_ref::<atom_syndication::Error>(),
            Some(atom_syndication::Error::Eof)
        ));
    }

    #[test]
    fn serialization_invalid_utf8_retains_typed_source() {
        let error = to_xml_string(Ok(vec![0xff])).expect_err("invalid UTF-8 must propagate");

        let source = std::error::Error::source(&error).expect("UTF-8 source");
        let source = source
            .downcast_ref::<std::string::FromUtf8Error>()
            .expect("typed FromUtf8Error");
        assert_eq!(source.as_bytes(), &[0xff]);
    }
}
