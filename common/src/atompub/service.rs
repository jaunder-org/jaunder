//! Service Document serializer for `AtomPub` (RFC 5023).
//!
//! A Service Document describes the collections a server supports for a given
//! workspace (e.g., one per user). This module provides [`ServiceDocument`] and
//! [`CollectionDecl`] types, plus [`render_service_document`] to serialize them
//! to XML using `quick-xml`.

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
use quick_xml::Writer;

use super::xml::{write_empty_element, write_text_element};
use super::{APP_NS, ATOM_NS, J_NS};
use crate::absolute_url::AbsoluteUrl;
use crate::tag::Tag;

/// Declaration of a single collection (posts or media) in a workspace.
#[derive(Debug, Clone)]
pub struct CollectionDecl {
    /// The collection's absolute IRI (#560, require-base).
    pub href: AbsoluteUrl,
    /// User-facing title of the collection.
    pub title: String,
    /// Media types accepted by the collection (e.g. "application/atom+xml;type=entry").
    pub accept: Vec<String>,
    /// Category scheme/terms available for entries in this collection.
    /// When non-empty, an `app:categories` element with `fixed="no"` is emitted.
    pub categories: Vec<Tag>,
}

/// A complete Service Document describing the publishing surface for one workspace.
#[derive(Debug, Clone)]
pub struct ServiceDocument {
    /// Workspace title (typically a username).
    pub workspace_title: String,
    /// The entries/posts collection.
    pub posts_collection: CollectionDecl,
    /// The media collection.
    pub media_collection: CollectionDecl,
}

/// Serializes a [`ServiceDocument`] to XML suitable for `AtomPub` discovery.
///
/// Emits an `app:service` document (root) with `xmlns="ATOM_NS"` and `xmlns:app="APP_NS"`,
/// containing one `app:workspace` with an `atom:title`, containing two `app:collection` elements
/// (posts and media). Each collection has an `href` attribute, an `atom:title` child,
/// one `app:accept` element per accept media type, and — when `categories` is non-empty —
/// an `app:categories fixed="no"` element with one inline `atom:category term="..."` per term.
///
/// Writes into an in-memory buffer, so it is infallible.
#[must_use]
pub fn render_service_document(doc: &ServiceDocument) -> String {
    let mut writer = Writer::new(Vec::new());
    let _ = writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)));

    let mut root = BytesStart::new("app:service");
    root.push_attribute(("xmlns", ATOM_NS));
    root.push_attribute(("xmlns:app", APP_NS));
    // Declare the Jaunder foreign-markup namespace so the `j:extension`
    // capability marker below is well-formed (ADR-0023).
    root.push_attribute(("xmlns:j", J_NS));
    let _ = writer.write_event(Event::Start(root));

    let _ = writer.write_event(Event::Start(BytesStart::new("app:workspace")));
    write_text_element(&mut writer, "atom:title", &doc.workspace_title);

    // Capability discovery (ADR-0023): advertise the Jaunder wire extensions this
    // server understands so clients can detect support before relying on them.
    let mut ext = BytesStart::new("j:extension");
    ext.push_attribute(("version", "1"));
    ext.push_attribute(("features", "format-media-type slug"));
    let _ = writer.write_event(Event::Empty(ext));

    write_collection(&mut writer, &doc.posts_collection);
    write_collection(&mut writer, &doc.media_collection);

    let _ = writer.write_event(Event::End(BytesEnd::new("app:workspace")));
    let _ = writer.write_event(Event::End(BytesEnd::new("app:service")));

    String::from_utf8_lossy(&writer.into_inner()).into_owned()
}

fn write_collection(writer: &mut Writer<Vec<u8>>, coll: &CollectionDecl) {
    let mut start = BytesStart::new("app:collection");
    start.push_attribute(("href", coll.href.as_ref()));
    let _ = writer.write_event(Event::Start(start));

    write_text_element(writer, "atom:title", &coll.title);

    for media_type in &coll.accept {
        write_text_element(writer, "app:accept", media_type);
    }

    if !coll.categories.is_empty() {
        let mut cat_elem = BytesStart::new("app:categories");
        cat_elem.push_attribute(("fixed", "no"));
        let _ = writer.write_event(Event::Start(cat_elem));

        for term in &coll.categories {
            write_empty_element(writer, "atom:category", &[("term", term)]);
        }

        let _ = writer.write_event(Event::End(BytesEnd::new("app:categories")));
    }

    let _ = writer.write_event(Event::End(BytesEnd::new("app:collection")));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::parse_absolute_url;

    /// A representative two-collection service document used by the serializer tests.
    fn sample_doc() -> ServiceDocument {
        ServiceDocument {
            workspace_title: "Alice".into(),
            posts_collection: CollectionDecl {
                href: parse_absolute_url("https://h/atompub/alice/posts"),
                title: "Posts".into(),
                accept: vec!["application/atom+xml;type=entry".into()],
                categories: vec!["rust".parse().unwrap(), "leptos".parse().unwrap()],
            },
            media_collection: CollectionDecl {
                href: parse_absolute_url("https://h/atompub/alice/media"),
                title: "Media".into(),
                accept: vec![
                    "image/png".into(),
                    "image/jpeg".into(),
                    "image/gif".into(),
                    "image/webp".into(),
                ],
                categories: vec![],
            },
        }
    }

    #[test]
    fn service_document_lists_two_collections() {
        let out = render_service_document(&sample_doc());
        assert!(out.contains("app:service"));
        assert!(out.contains("https://h/atompub/alice/posts"));
        assert!(out.contains("type=entry"));
        assert!(out.contains("image/webp"));
        assert!(out.contains("app:categories"));
        assert!(out.contains("fixed=\"no\""));
    }

    #[test]
    fn service_document_advertises_jaunder_extension() {
        let out = render_service_document(&sample_doc());
        assert!(
            out.contains(r#"xmlns:j="https://jaunder.org/ns/atompub""#),
            "out: {out}"
        );
        assert!(
            out.contains(r#"<j:extension version="1" features="format-media-type slug""#),
            "out: {out}"
        );
    }
}
