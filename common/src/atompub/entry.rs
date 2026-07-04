//! Standalone Atom entry (`<entry>`) read/write for `AtomPub`.
//!
//! The data model is `atom_syndication::Entry` — a complete, public Atom entry
//! struct. We do **not** reuse `atom_syndication`'s XML I/O, because its
//! entry-level read/write traits are crate-private; it can only handle whole
//! `<feed>` documents, while `AtomPub` exchanges *standalone* `<entry>` documents
//! (POST to create, PUT to edit, GET a member). So the XML reading and writing
//! is done here with `quick-xml`, populating and reading the canonical
//! `atom_syndication::Entry`.
//!
//! The one piece `atom_syndication` does not model first-class is the Atom
//! Publishing Protocol control element `app:control/app:draft`; it is stored in
//! the entry's extension map and accessed via [`is_draft`] / [`set_draft`].

use std::collections::BTreeMap;

use atom_syndication::extension::Extension;
use atom_syndication::{Category, Content, Entry, Link, Text};
use quick_xml::events::{BytesDecl, BytesEnd, BytesRef, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

use super::xml::{write_empty_element, write_link, write_text_element};
use super::{AtomPubError, APP_NS, ATOM_NS, J_NS};

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
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Accumulator for the simple text-bearing elements of an entry.
#[derive(Default)]
struct Acc {
    title: Option<String>,
    summary: Option<String>,
    id: Option<String>,
    updated: Option<String>,
    published: Option<String>,
    content_value: Option<String>,
    content_type: Option<String>,
    categories: Vec<String>,
    links: Vec<(String, String)>,
    draft: bool,
}

/// The simple text-bearing element whose character data is currently being
/// collected into [`Acc`].
#[derive(Clone, Copy)]
enum Field {
    Title,
    Summary,
    Id,
    Updated,
    Published,
    Content,
    Draft,
}

/// Streaming state for [`entry_from_xml`]. Each SAX event is dispatched to a
/// small method so the top-level read loop stays a flat match.
#[derive(Default)]
struct Parser {
    acc: Acc,
    saw_entry: bool,
    /// The element whose text is currently being routed, if any.
    current: Option<Field>,
    /// True while inside an `app:control` element (scopes the `draft` child).
    in_control: bool,
}

impl Parser {
    /// Handles a start tag. `<content type="xhtml">` is consumed eagerly via
    /// [`read_xhtml_content`]; every other element just arms `current`.
    fn start(&mut self, e: &BytesStart, reader: &mut Reader<&[u8]>) -> Result<(), AtomPubError> {
        match local_name(e).as_str() {
            "entry" => self.saw_entry = true,
            "title" => self.current = Some(Field::Title),
            "summary" => self.current = Some(Field::Summary),
            "id" => self.current = Some(Field::Id),
            "updated" => self.current = Some(Field::Updated),
            "published" => self.current = Some(Field::Published),
            "content" => {
                let ctype = attr_value(e, b"type").unwrap_or_else(|| "text".to_string());
                if ctype == "xhtml" {
                    self.acc.content_value = Some(read_xhtml_content(reader)?);
                } else {
                    self.current = Some(Field::Content);
                }
                self.acc.content_type = Some(ctype);
            }
            "link" => capture_link(e, &mut self.acc),
            "control" => self.in_control = true,
            "draft" if self.in_control => self.current = Some(Field::Draft),
            _ => {}
        }
        Ok(())
    }

    /// Handles an empty (self-closing) tag: `<category>` and `<link>`.
    fn empty(&mut self, e: &BytesStart) {
        match local_name(e).as_str() {
            "category" => {
                if let Some(term) = attr_value(e, b"term") {
                    self.acc.categories.push(term);
                }
            }
            "link" => capture_link(e, &mut self.acc),
            _ => {}
        }
    }

    /// Routes a decoded text/entity piece into the element named by `current`.
    /// Shared by the `Text` and `GeneralRef` event arms.
    fn text(&mut self, piece: &str) {
        match self.current {
            Some(Field::Title) => append(&mut self.acc.title, piece),
            Some(Field::Summary) => append(&mut self.acc.summary, piece),
            Some(Field::Id) => append(&mut self.acc.id, piece),
            Some(Field::Updated) => append(&mut self.acc.updated, piece),
            Some(Field::Published) => append(&mut self.acc.published, piece),
            Some(Field::Content) => append(&mut self.acc.content_value, piece),
            Some(Field::Draft) => self.acc.draft = piece.trim().eq_ignore_ascii_case("yes"),
            None => {}
        }
    }

    /// Handles an end tag, closing the current text element and `app:control`.
    fn end(&mut self, e: &BytesEnd) {
        if local_name_end(e) == "control" {
            self.in_control = false;
        }
        self.current = None;
    }
}

/// Parses a standalone `AtomPub` `<entry>` document into an [`Entry`].
///
/// Server-owned fields a client omits (id, dates, links) are simply left at
/// their defaults; this reader captures whatever the document provides.
///
/// # Errors
///
/// Returns [`AtomPubError::Malformed`] when the bytes are not a well-formed
/// `<entry>` document or contain an unsupported entity reference.
pub fn entry_from_xml(xml: &str) -> Result<Entry, AtomPubError> {
    // Text is NOT trimmed globally — that would strip significant whitespace
    // inside content. Inter-element indentation is harmless because text is
    // only routed when a target element (`current`) is active.
    let mut reader = Reader::from_str(xml);
    let mut parser = Parser::default();

    loop {
        match reader.read_event()? {
            Event::Eof => break,
            Event::Start(e) => parser.start(&e, &mut reader)?,
            Event::Empty(e) => parser.empty(&e),
            Event::Text(e) => parser.text(&decode_text(&e)?),
            // quick-xml 0.39 emits entity references (`&lt;`, `&#60;`) as
            // separate events rather than inlining them into Text.
            Event::GeneralRef(e) => parser.text(&resolve_ref(&e)?),
            Event::End(e) => parser.end(&e),
            _ => {}
        }
    }

    if !parser.saw_entry {
        return Err(AtomPubError::Malformed(
            "document has no <entry> element".to_string(),
        ));
    }

    Ok(build_entry(parser.acc))
}

/// Re-serializes the raw inner markup of an `<content type="xhtml">` element
/// into a string, consuming events up to and including the matching
/// `</content>` end tag.
///
/// Called after the opening `<content>` tag has been read, so `depth` tracks
/// nesting *within* the content; the close at depth 0 terminates capture.
fn read_xhtml_content(reader: &mut Reader<&[u8]>) -> Result<String, AtomPubError> {
    let mut buf = Writer::new(Vec::new());
    let mut depth = 0u32;
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                depth += 1;
                buf.write_event(Event::Start(e.into_owned()))?;
            }
            Event::End(e) => {
                if local_name_end(&e) == "content" && depth == 0 {
                    let html = String::from_utf8(buf.into_inner())
                        .map_err(|err| AtomPubError::Malformed(err.to_string()))?;
                    return Ok(html.trim().to_string());
                }
                depth = depth.saturating_sub(1);
                buf.write_event(Event::End(e.into_owned()))?;
            }
            Event::Empty(e) => buf.write_event(Event::Empty(e.into_owned()))?,
            Event::Text(e) => buf.write_event(Event::Text(e.into_owned()))?,
            Event::GeneralRef(e) => {
                buf.write_event(Event::Text(BytesText::new(&resolve_ref(&e)?)))?;
            }
            Event::CData(e) => buf.write_event(Event::CData(e.into_owned()))?,
            Event::Eof => {
                return Err(AtomPubError::Malformed(
                    "unclosed <content type=\"xhtml\"> element".to_string(),
                ))
            }
            _ => {}
        }
    }
}

fn build_entry(acc: Acc) -> Entry {
    let mut entry = Entry::default();
    if let Some(title) = trimmed(acc.title) {
        entry.title = Text::plain(title);
    }
    entry.summary = trimmed(acc.summary).map(Text::plain);
    if let Some(value) = acc.content_value {
        entry.content = Some(Content {
            content_type: Some(acc.content_type.unwrap_or_else(|| "text".to_string())),
            value: Some(value),
            ..Default::default()
        });
    }
    entry.categories = acc
        .categories
        .into_iter()
        .map(|term| Category {
            term,
            ..Default::default()
        })
        .collect();
    entry.links = acc
        .links
        .into_iter()
        .map(|(rel, href)| Link {
            rel,
            href,
            ..Default::default()
        })
        .collect();
    if let Some(id) = acc.id {
        entry.id = id;
    }
    if let Some(updated) = acc.updated.as_deref().and_then(parse_dt) {
        entry.updated = updated;
    }
    entry.published = acc.published.as_deref().and_then(parse_dt);
    if acc.draft {
        set_draft(&mut entry, true);
    }
    entry
}

fn parse_dt(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(s.trim()).ok()
}

fn trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn capture_link(e: &BytesStart, acc: &mut Acc) {
    if let (Some(rel), Some(href)) = (
        attr_value(e, b"rel").or_else(|| Some("alternate".to_string())),
        attr_value(e, b"href"),
    ) {
        acc.links.push((rel, href));
    }
}

/// Resolves a general or character entity reference to its string value.
fn resolve_ref(e: &BytesRef) -> Result<String, AtomPubError> {
    if e.is_char_ref() {
        return match e.resolve_char_ref()? {
            Some(c) => Ok(c.to_string()),
            None => Ok(String::new()), // cov:ignore
        };
    }
    let name =
        std::str::from_utf8(e.as_ref()).map_err(|err| AtomPubError::Malformed(err.to_string()))?;
    let resolved = match name {
        "lt" => "<",
        "gt" => ">",
        "amp" => "&",
        "quot" => "\"",
        "apos" => "'",
        other => {
            return Err(AtomPubError::Malformed(format!(
                "unsupported entity reference &{other};"
            )))
        }
    };
    Ok(resolved.to_string())
}

fn append(target: &mut Option<String>, text: &str) {
    match target {
        Some(existing) => existing.push_str(text),
        None => *target = Some(text.to_string()),
    }
}

fn local_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

fn local_name_end(e: &BytesEnd) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

fn attr_value(e: &BytesStart, key: &[u8]) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.local_name().as_ref() == key {
            let raw = std::str::from_utf8(a.value.as_ref()).ok()?;
            quick_xml::escape::unescape(raw)
                .ok()
                .map(std::borrow::Cow::into_owned)
        } else {
            None
        }
    })
}

/// Decodes a text event's bytes (UTF-8) and resolves XML entities.
fn decode_text(e: &BytesText) -> Result<String, AtomPubError> {
    let decoded = e
        .decode()
        .map_err(|err| AtomPubError::Malformed(err.to_string()))?;
    Ok(quick_xml::escape::unescape(&decoded)
        .map_err(|err| AtomPubError::Malformed(err.to_string()))?
        .into_owned())
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

/// Serializes an [`Entry`] to a standalone `AtomPub` `<entry>` document.
///
/// Emits whatever the entry carries: id, title, dates, summary, content (with
/// its `type`), all links, all categories, and the draft marker when set.
///
/// Serialization writes into an in-memory buffer, which cannot fail, so this is
/// infallible and returns a `String` directly.
#[must_use]
pub fn entry_to_xml(entry: &Entry) -> String {
    let mut writer = Writer::new(Vec::new());
    let _ = writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)));
    write_entry(&mut writer, entry, true);
    String::from_utf8_lossy(&writer.into_inner()).into_owned()
}

/// Writes an `<entry>…</entry>` element to the provided writer.
///
/// # Parameters
///
/// - `writer`: The XML writer to emit events to.
/// - `entry`: The entry to serialize.
/// - `declare_namespaces`: If `true`, emits `xmlns` and `xmlns:app` attributes on
///   the entry element. If `false`, assumes the namespaces are already declared
///   by an enclosing element (e.g., a `<feed>`).
///
/// Writes into an in-memory buffer, so it is infallible.
fn write_entry(writer: &mut Writer<Vec<u8>>, entry: &Entry, declare_namespaces: bool) {
    let draft = is_draft(entry);
    let mut root = BytesStart::new("entry");
    if declare_namespaces {
        root.push_attribute(("xmlns", ATOM_NS));
        if draft {
            root.push_attribute(("xmlns:app", APP_NS));
        }
        if j_slug(entry).is_some() {
            root.push_attribute(("xmlns:j", J_NS));
        }
    }
    let _ = writer.write_event(Event::Start(root));

    write_text_element(writer, "id", entry.id());
    write_text_element(writer, "title", entry.title().as_str());
    write_text_element(writer, "updated", &entry.updated().to_rfc3339());
    if let Some(published) = entry.published() {
        write_text_element(writer, "published", &published.to_rfc3339());
    }
    if let Some(summary) = entry.summary() {
        write_text_element(writer, "summary", summary.as_str());
    }
    if let Some(content) = entry.content() {
        let mut start = BytesStart::new("content");
        start.push_attribute(("type", content.content_type().unwrap_or("text")));
        let _ = writer.write_event(Event::Start(start));
        let _ = writer.write_event(Event::Text(BytesText::new(content.value().unwrap_or(""))));
        let _ = writer.write_event(Event::End(BytesEnd::new("content")));
    }
    for link in entry.links() {
        write_link(writer, link.rel(), link.href());
    }
    for category in entry.categories() {
        write_empty_element(writer, "category", &[("term", category.term())]);
    }
    if draft {
        let _ = writer.write_event(Event::Start(BytesStart::new("app:control")));
        write_text_element(writer, "app:draft", "yes");
        let _ = writer.write_event(Event::End(BytesEnd::new("app:control")));
    }
    // Read-only server slug (ADR-0023): emitted on every outgoing entry.
    if let Some(slug) = j_slug(entry) {
        write_text_element(writer, "j:slug", &slug);
    }

    let _ = writer.write_event(Event::End(BytesEnd::new("entry")));
}

/// Feed-level metadata for an `AtomPub` collection document.
///
/// Used to wrap multiple entries in a `<feed>` with RFC 5005 paging links.
#[derive(Debug, Clone)]
pub struct FeedMeta {
    /// Stable feed id (an IRI).
    pub id: String,
    /// Human-readable collection title.
    pub title: String,
    /// Feed `updated` timestamp, RFC 3339.
    pub updated_rfc3339: String,
    /// `rel="self"` href (the collection URL for this page).
    pub self_url: String,
    /// `rel="first"` href, when paging.
    pub first: Option<String>,
    /// `rel="next"` href, when a next page exists.
    pub next: Option<String>,
    /// `rel="previous"` href, when a previous page exists.
    pub previous: Option<String>,
}

/// Serializes a collection `<feed>` wrapping the given entries, with RFC 5005
/// paging links.
///
/// The entries are embedded without redeclaring the Atom namespace; the `<feed>`
/// root declares both `xmlns` and `xmlns:app`.
///
/// Writes into an in-memory buffer, so it is infallible.
#[must_use]
pub fn render_feed(meta: &FeedMeta, entries: &[Entry]) -> String {
    let mut writer = Writer::new(Vec::new());
    let _ = writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)));

    let mut root = BytesStart::new("feed");
    root.push_attribute(("xmlns", ATOM_NS));
    root.push_attribute(("xmlns:app", APP_NS));
    // Every embedded entry carries a read-only j:slug (ADR-0023) and is written
    // with declare_namespaces=false, so the feed root declares xmlns:j for them.
    root.push_attribute(("xmlns:j", J_NS));
    let _ = writer.write_event(Event::Start(root));

    write_text_element(&mut writer, "id", &meta.id);
    write_text_element(&mut writer, "title", &meta.title);
    write_text_element(&mut writer, "updated", &meta.updated_rfc3339);
    write_link(&mut writer, "self", &meta.self_url);
    if let Some(href) = &meta.first {
        write_link(&mut writer, "first", href);
    }
    if let Some(href) = &meta.previous {
        write_link(&mut writer, "previous", href);
    }
    if let Some(href) = &meta.next {
        write_link(&mut writer, "next", href);
    }

    for entry in entries {
        write_entry(&mut writer, entry, false);
    }

    let _ = writer.write_event(Event::End(BytesEnd::new("feed")));
    String::from_utf8_lossy(&writer.into_inner()).into_owned()
}

/// A media-link entry (RFC 5023 §9.6): the Atom `<entry>` a server returns for
/// an uploaded media resource. Its `<content>` references the binary by `src`
/// rather than embedding it, and it carries both an `edit` link (the member)
/// and an `edit-media` link (the binary).
#[derive(Debug, Clone)]
pub struct MediaLinkEntry {
    /// Stable entry id (an IRI).
    pub id: String,
    /// Human-readable title (typically the filename).
    pub title: String,
    /// `rel="edit"` href — the media-link member resource.
    pub edit_uri: String,
    /// `rel="edit-media"` href — the binary media resource.
    pub edit_media_uri: String,
    /// `<content src=...>` — the absolute URL of the binary.
    pub content_src: String,
    /// MIME type of the binary.
    pub content_type: String,
    /// Publication timestamp, RFC 3339.
    pub published_rfc3339: String,
    /// Last-update timestamp, RFC 3339.
    pub updated_rfc3339: String,
}

/// Serializes a [`MediaLinkEntry`] to a standalone `<entry>` document.
///
/// Writes into an in-memory buffer, so it is infallible.
#[must_use]
pub fn render_media_link_entry(entry: &MediaLinkEntry) -> String {
    let mut writer = Writer::new(Vec::new());
    let _ = writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)));

    let mut root = BytesStart::new("entry");
    root.push_attribute(("xmlns", ATOM_NS));
    let _ = writer.write_event(Event::Start(root));

    write_text_element(&mut writer, "id", &entry.id);
    write_text_element(&mut writer, "title", &entry.title);
    write_text_element(&mut writer, "updated", &entry.updated_rfc3339);
    write_text_element(&mut writer, "published", &entry.published_rfc3339);

    let content_attrs = [
        ("type", entry.content_type.as_str()),
        ("src", entry.content_src.as_str()),
    ];
    write_empty_element(&mut writer, "content", &content_attrs);

    write_link(&mut writer, "edit", &entry.edit_uri);
    write_link(&mut writer, "edit-media", &entry.edit_media_uri);

    let _ = writer.write_event(Event::End(BytesEnd::new("entry")));
    String::from_utf8_lossy(&writer.into_inner()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content_parts(entry: &Entry) -> (Option<&str>, Option<&str>) {
        match entry.content() {
            Some(c) => (c.content_type(), c.value()),
            None => (None, None),
        }
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
    fn parses_text_entry_not_draft() {
        let xml = r#"<?xml version="1.0"?>
<entry xmlns="http://www.w3.org/2005/Atom">
  <title>Note</title>
  <content type="text"># markdown</content>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        assert_eq!(entry.title().as_str(), "Note");
        assert_eq!(content_parts(&entry), (Some("text"), Some("# markdown")));
        assert!(entry.summary().is_none());
        assert!(!is_draft(&entry));
        assert!(entry.categories().is_empty());
    }

    #[test]
    fn parses_id_and_timestamps() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <id>tag:example.com,2026:post/7</id>
  <published>2026-01-01T00:00:00Z</published>
  <updated>2026-01-02T03:04:05Z</updated>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        assert_eq!(entry.id(), "tag:example.com,2026:post/7");
        assert_eq!(
            entry.published().map(chrono::DateTime::to_rfc3339),
            Some("2026-01-01T00:00:00+00:00".to_string())
        );
        assert_eq!(entry.updated().to_rfc3339(), "2026-01-02T03:04:05+00:00");
    }

    #[test]
    fn parses_numeric_and_named_char_refs_across_pieces() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>A&#66;C &quot;q&quot; &apos;a&apos;</title>
  <content type="text">x</content>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        assert_eq!(entry.title().as_str(), "ABC \"q\" 'a'");
    }

    #[test]
    fn unsupported_entity_is_an_error() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>x&bogus;y</title>
</entry>"#;
        assert!(entry_from_xml(xml).is_err());
    }

    #[test]
    fn parses_xhtml_with_empty_element_entity_and_cdata() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>X</title>
  <content type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml">a<br/>b &amp; c<![CDATA[ d ]]></div></content>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        let (ctype, value) = content_parts(&entry);
        assert_eq!(ctype, Some("xhtml"));
        let value = value.expect("xhtml value");
        assert!(value.contains("<br"), "value: {value}");
        assert!(value.contains('b'), "value: {value}");
    }

    #[test]
    fn parses_nested_xhtml_preserving_inner_markup() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>X</title>
  <content type="xhtml"><div><p>one</p><!-- note --><ul><li>two</li></ul></div></content>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        let (ctype, value) = content_parts(&entry);
        assert_eq!(ctype, Some("xhtml"));
        let value = value.expect("xhtml value");
        assert!(value.contains("<ul>"), "value: {value}");
        assert!(value.contains("<li>two</li>"), "value: {value}");
        assert!(value.contains("</div>"), "value: {value}");
    }

    #[test]
    fn unclosed_xhtml_content_is_an_error() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>X</title>
  <content type="xhtml"><div>oops"#;
        assert!(entry_from_xml(xml).is_err());
    }

    #[test]
    fn parses_links_with_rel_and_href() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>L</title>
  <link rel="edit" href="https://h/atompub/alice/posts/1"/>
  <link href="https://h/~alice/p"/>
</entry>"#;
        let entry = entry_from_xml(xml).expect("parse");
        assert_eq!(entry.links().len(), 2);
        assert_eq!(entry.links()[0].rel(), "edit");
        assert_eq!(entry.links()[0].href(), "https://h/atompub/alice/posts/1");
        // A link without rel defaults to "alternate".
        assert_eq!(entry.links()[1].rel(), "alternate");
        // The entry has no <content>, so content_parts yields the empty pair.
        assert_eq!(content_parts(&entry), (None, None));
    }

    #[test]
    fn malformed_xml_is_an_error() {
        assert!(entry_from_xml("<entry><unclosed></entry>").is_err());
    }

    #[test]
    fn document_without_entry_is_an_error() {
        assert!(entry_from_xml("<?xml version=\"1.0\"?><other/>").is_err());
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
    fn j_slug_is_serialized_with_namespace() {
        let mut entry = sample_entry();
        set_j_slug(&mut entry, "my-post");
        let out = entry_to_xml(&entry);
        assert!(
            out.contains(r#"xmlns:j="https://jaunder.org/ns/atompub""#),
            "out: {out}"
        );
        assert!(out.contains("<j:slug>my-post</j:slug>"), "out: {out}");
    }

    #[test]
    fn no_j_slug_means_no_namespace_declared() {
        let entry = sample_entry();
        let out = entry_to_xml(&entry);
        assert!(!out.contains("xmlns:j"), "out: {out}");
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

        let out = entry_to_xml(&entry);
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
        let out = entry_to_xml(&entry);
        assert!(out.contains("app:draft"), "out: {out}");
        assert!(out.contains("yes"), "out: {out}");
        assert!(out.contains("type=\"html\""), "out: {out}");
        assert!(out.contains("&lt;p&gt;x&lt;/p&gt;"), "out: {out}");
    }

    #[test]
    fn serializes_xhtml_content_type() {
        let mut entry = sample_entry();
        entry.content = Some(Content {
            content_type: Some("xhtml".to_string()),
            value: Some("<div><p>hi</p></div>".to_string()),
            ..Default::default()
        });
        let out = entry_to_xml(&entry);
        assert!(out.contains("type=\"xhtml\""), "out: {out}");
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
        set_draft(&mut entry, true);

        let out = entry_to_xml(&entry);
        let parsed = entry_from_xml(&out).expect("re-parse");
        assert!(is_draft(&parsed), "draft flag lost; xml: {out}");
        assert_eq!(parsed.title().as_str(), "RT");
        assert_eq!(parsed.summary().map(Text::as_str), Some("s"));
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
        assert!(!entry_to_xml(&entry).contains("app:draft"));
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
            id: "tag:example.com,2026:collection/user/alice".to_string(),
            title: "Alice's Posts".to_string(),
            updated_rfc3339: "2026-05-31T12:00:00Z".to_string(),
            self_url: "https://example.com/atompub/alice/posts".to_string(),
            first: Some("https://example.com/atompub/alice/posts?page=1".to_string()),
            next: Some("https://example.com/atompub/alice/posts?page=2".to_string()),
            previous: Some("https://example.com/atompub/alice/posts?page=0".to_string()),
        };

        let out = render_feed(&meta, &[entry1, entry2]);

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

        // Entry titles present
        assert!(out.contains(">First<"), "out: {out}");
        assert!(out.contains(">Second<"), "out: {out}");

        // Embedded entries should NOT redeclare the Atom namespace on their own
        // They should not have xmlns="..." as an attribute on the entry element
        let entry_with_xmlns = out.contains("<entry xmlns=\"");
        assert!(
            !entry_with_xmlns,
            "Entries should not redeclare xmlns; out: {out}"
        );

        // Feed closing tag present
        assert!(out.contains("</feed>"), "out: {out}");
    }

    #[test]
    fn render_feed_without_paging_omits_optional_links() {
        let mut entry = sample_entry();
        entry.title = Text::plain("Single");

        let meta = FeedMeta {
            id: "tag:example.com,2026:collection/user/bob".to_string(),
            title: "Bob's Posts".to_string(),
            updated_rfc3339: "2026-05-31T13:00:00Z".to_string(),
            self_url: "https://example.com/atompub/bob/posts".to_string(),
            first: None,
            next: None,
            previous: None,
        };

        let out = render_feed(&meta, &[entry]);

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
    fn render_media_link_entry_references_binary_by_src() {
        let out = render_media_link_entry(&MediaLinkEntry {
            id: "https://h/atompub/alice/media/abc/pic.png".to_string(),
            title: "pic.png".to_string(),
            edit_uri: "https://h/atompub/alice/media/abc/pic.png".to_string(),
            edit_media_uri: "https://h/media/upload/ab/c0/abc/pic.png".to_string(),
            content_src: "https://h/media/upload/ab/c0/abc/pic.png".to_string(),
            content_type: "image/png".to_string(),
            published_rfc3339: "2026-06-01T00:00:00Z".to_string(),
            updated_rfc3339: "2026-06-01T00:00:00Z".to_string(),
        });

        assert!(out.contains("<entry"), "out: {out}");
        assert!(out.contains("type=\"image/png\""), "out: {out}");
        assert!(
            out.contains("src=\"https://h/media/upload/ab/c0/abc/pic.png\""),
            "out: {out}"
        );
        assert!(out.contains("rel=\"edit-media\""), "out: {out}");
        assert!(out.contains("rel=\"edit\""), "out: {out}");
        assert!(out.contains(">pic.png<"), "out: {out}");
    }
}
