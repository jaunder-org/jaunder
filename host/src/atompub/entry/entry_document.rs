//! Standalone `AtomPub` Member Entry serialization.
//!
//! This leaf delegates entry XML writing to `atom_syndication`; extension-marker
//! ownership remains in the sibling `foreign_markers` leaf.

use atom_syndication::Entry;

use super::super::AtomPubError;
use super::render::to_xml_string;

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

#[cfg(test)]
mod tests {
    use super::super::foreign_markers::is_draft;
    use super::*;
    use atom_syndication::extension::ExtensionContent;
    use atom_syndication::{Category, Content, Link, Text};

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
        // prefixed child is retained as a foreign extension rather than the title.
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

    #[test]
    fn standalone_entry_preserves_nested_rebound_extension_names() {
        // The inner declaration rebinds `x`; equality and explicit names prove
        // serialization keeps URI identity rather than treating prefixes as names.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <x:outer xmlns:x="urn:outer"><x:inner xmlns:x="urn:inner">value</x:inner></x:outer>
</entry>"#;
        let entry = xml.parse::<Entry>().expect("parse");
        let original = entry.extensions().to_vec();
        let reparsed = entry_to_xml(&entry)
            .expect("serialize")
            .parse::<Entry>()
            .expect("reparse");

        assert_eq!(reparsed.extensions(), original.as_slice());
        let outer = &reparsed.extensions()[0];
        assert_eq!(outer.name.namespace_uri.as_deref(), Some("urn:outer"));
        assert_eq!(outer.name.local_name, "outer");
        let ExtensionContent::Element(inner) = &outer.content[0] else {
            unreachable!("nested extension was parsed as text")
        };
        assert_eq!(inner.name.namespace_uri.as_deref(), Some("urn:inner"));
        assert_eq!(inner.name.local_name, "inner");
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
}
