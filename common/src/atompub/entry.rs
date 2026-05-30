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

use super::{AtomPubError, APP_NS, ATOM_NS};

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
    // Remove any existing control element under any namespace prefix.
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

/// Parses a standalone `AtomPub` `<entry>` document into an [`Entry`].
///
/// Server-owned fields a client omits (id, dates, links) are simply left at
/// their defaults; this reader captures whatever the document provides.
///
/// # Errors
///
/// Returns [`AtomPubError::Malformed`] when the bytes are not a well-formed
/// `<entry>` document or contain an unsupported entity reference.
#[allow(clippy::too_many_lines)]
pub fn entry_from_xml(xml: &str) -> Result<Entry, AtomPubError> {
    // Text is NOT trimmed globally — that would strip significant whitespace
    // inside content. Inter-element indentation is harmless because text is
    // only routed when a target element (`current`) is active.
    let mut reader = Reader::from_str(xml);

    let mut acc = Acc::default();
    let mut saw_entry = false;
    let mut current: Option<String> = None;
    // For xhtml content, raw inner markup is re-serialized into this buffer.
    let mut xhtml_buf: Option<Writer<Vec<u8>>> = None;
    let mut xhtml_depth = 0u32;
    let mut in_control = false;

    loop {
        let event = reader.read_event()?;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                if let Some(buf) = xhtml_buf.as_mut() {
                    xhtml_depth += 1;
                    buf.write_event(Event::Start(e.into_owned()))?;
                    continue;
                }
                match local_name(&e).as_str() {
                    "entry" => saw_entry = true,
                    "title" => current = Some("title".to_string()),
                    "summary" => current = Some("summary".to_string()),
                    "id" => current = Some("id".to_string()),
                    "updated" => current = Some("updated".to_string()),
                    "published" => current = Some("published".to_string()),
                    "content" => {
                        let ctype = attr_value(&e, b"type").unwrap_or_else(|| "text".to_string());
                        if ctype == "xhtml" {
                            xhtml_buf = Some(Writer::new(Vec::new()));
                            xhtml_depth = 0;
                        } else {
                            current = Some("content".to_string());
                        }
                        acc.content_type = Some(ctype);
                    }
                    "link" => capture_link(&e, &mut acc),
                    "control" => in_control = true,
                    "draft" if in_control => current = Some("draft".to_string()),
                    _ => {}
                }
            }
            Event::Empty(e) => {
                if let Some(buf) = xhtml_buf.as_mut() {
                    buf.write_event(Event::Empty(e.into_owned()))?;
                    continue;
                }
                match local_name(&e).as_str() {
                    "category" => {
                        if let Some(term) = attr_value(&e, b"term") {
                            acc.categories.push(term);
                        }
                    }
                    "link" => capture_link(&e, &mut acc),
                    _ => {}
                }
            }
            Event::Text(e) => {
                if let Some(buf) = xhtml_buf.as_mut() {
                    buf.write_event(Event::Text(e))?;
                    continue;
                }
                route_text(&mut acc, current.as_deref(), &decode_text(&e)?);
            }
            // quick-xml 0.39 emits entity references (`&lt;`, `&#60;`) as
            // separate events rather than inlining them into Text.
            Event::GeneralRef(e) => {
                let piece = resolve_ref(&e)?;
                if let Some(buf) = xhtml_buf.as_mut() {
                    buf.write_event(Event::Text(BytesText::new(&piece)))?;
                    continue;
                }
                route_text(&mut acc, current.as_deref(), &piece);
            }
            Event::CData(e) => {
                if let Some(buf) = xhtml_buf.as_mut() {
                    buf.write_event(Event::CData(e.into_owned()))?;
                }
            }
            Event::End(e) => {
                let local = local_name_end(&e);
                if let Some(buf) = xhtml_buf.as_mut() {
                    if local == "content" && xhtml_depth == 0 {
                        let inner = buf.get_ref().clone();
                        let html = String::from_utf8(inner)
                            .map_err(|err| AtomPubError::Malformed(err.to_string()))?;
                        acc.content_value = Some(html.trim().to_string());
                        xhtml_buf = None;
                    } else {
                        xhtml_depth = xhtml_depth.saturating_sub(1);
                        buf.write_event(Event::End(e.into_owned()))?;
                    }
                    continue;
                }
                if local == "control" {
                    in_control = false;
                }
                current = None;
            }
            _ => {}
        }
    }

    if !saw_entry {
        return Err(AtomPubError::Malformed(
            "document has no <entry> element".to_string(),
        ));
    }

    Ok(build_entry(acc))
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

/// Routes a decoded text/entity piece into the element currently being
/// collected. Shared by the `Text` and `GeneralRef` event arms.
fn route_text(acc: &mut Acc, current: Option<&str>, piece: &str) {
    match current {
        Some("title") => append(&mut acc.title, piece),
        Some("summary") => append(&mut acc.summary, piece),
        Some("id") => append(&mut acc.id, piece),
        Some("updated") => append(&mut acc.updated, piece),
        Some("published") => append(&mut acc.published, piece),
        Some("content") => append(&mut acc.content_value, piece),
        Some("draft") => acc.draft = piece.trim().eq_ignore_ascii_case("yes"),
        _ => {}
    }
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
            None => Ok(String::new()),
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
/// # Errors
///
/// Returns [`AtomPubError::Malformed`] if the XML writer fails (which should
/// not occur for valid in-memory inputs).
pub fn entry_to_xml(entry: &Entry) -> Result<String, AtomPubError> {
    let mut writer = Writer::new(Vec::new());
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))?;

    let draft = is_draft(entry);
    let mut root = BytesStart::new("entry");
    root.push_attribute(("xmlns", ATOM_NS));
    if draft {
        root.push_attribute(("xmlns:app", APP_NS));
    }
    writer.write_event(Event::Start(root))?;

    write_text_element(&mut writer, "id", entry.id())?;
    write_text_element(&mut writer, "title", entry.title().as_str())?;
    write_text_element(&mut writer, "updated", &entry.updated().to_rfc3339())?;
    if let Some(published) = entry.published() {
        write_text_element(&mut writer, "published", &published.to_rfc3339())?;
    }
    if let Some(summary) = entry.summary() {
        write_text_element(&mut writer, "summary", summary.as_str())?;
    }
    if let Some(content) = entry.content() {
        let mut start = BytesStart::new("content");
        start.push_attribute(("type", content.content_type().unwrap_or("text")));
        writer.write_event(Event::Start(start))?;
        writer.write_event(Event::Text(BytesText::new(content.value().unwrap_or(""))))?;
        writer.write_event(Event::End(BytesEnd::new("content")))?;
    }
    for link in entry.links() {
        write_link(&mut writer, link.rel(), link.href())?;
    }
    for category in entry.categories() {
        let mut start = BytesStart::new("category");
        start.push_attribute(("term", category.term()));
        writer.write_event(Event::Empty(start))?;
    }
    if draft {
        writer.write_event(Event::Start(BytesStart::new("app:control")))?;
        write_text_element(&mut writer, "app:draft", "yes")?;
        writer.write_event(Event::End(BytesEnd::new("app:control")))?;
    }

    writer.write_event(Event::End(BytesEnd::new("entry")))?;

    String::from_utf8(writer.into_inner()).map_err(|e| AtomPubError::Malformed(e.to_string()))
}

fn write_text_element(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    text: &str,
) -> Result<(), AtomPubError> {
    writer.write_event(Event::Start(BytesStart::new(name)))?;
    writer.write_event(Event::Text(BytesText::new(text)))?;
    writer.write_event(Event::End(BytesEnd::new(name)))?;
    Ok(())
}

fn write_link(writer: &mut Writer<Vec<u8>>, rel: &str, href: &str) -> Result<(), AtomPubError> {
    let mut link = BytesStart::new("link");
    link.push_attribute(("rel", rel));
    link.push_attribute(("href", href));
    writer.write_event(Event::Empty(link))?;
    Ok(())
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
        let mut entry = Entry::default();
        entry.id = "tag:example.com,2026:post/1".to_string();
        entry.title = Text::plain("Hello");
        entry.updated = chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z").unwrap();
        entry
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
    fn serializes_xhtml_content_type() {
        let mut entry = sample_entry();
        entry.content = Some(Content {
            content_type: Some("xhtml".to_string()),
            value: Some("<div><p>hi</p></div>".to_string()),
            ..Default::default()
        });
        let out = entry_to_xml(&entry).expect("serialize");
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

        let out = entry_to_xml(&entry).expect("serialize");
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
        assert!(!entry_to_xml(&entry)
            .expect("serialize")
            .contains("app:draft"));
    }
}
