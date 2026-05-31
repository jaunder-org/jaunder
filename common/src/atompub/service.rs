//! Service Document serializer for `AtomPub` (RFC 5023).
//!
//! A Service Document describes the collections a server supports for a given
//! workspace (e.g., one per user). This module provides [`ServiceDocument`] and
//! [`CollectionDecl`] types, plus [`render_service_document`] to serialize them
//! to XML using `quick-xml`.

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;

use super::{AtomPubError, APP_NS, ATOM_NS};

/// Declaration of a single collection (posts or media) in a workspace.
#[derive(Debug, Clone)]
pub struct CollectionDecl {
    /// The collection's IRI reference.
    pub href: String,
    /// User-facing title of the collection.
    pub title: String,
    /// Media types accepted by the collection (e.g. "application/atom+xml;type=entry").
    pub accept: Vec<String>,
    /// Category scheme/terms available for entries in this collection.
    /// When non-empty, an `app:categories` element with `fixed="no"` is emitted.
    pub categories: Vec<String>,
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
/// # Errors
///
/// Returns [`AtomPubError::Malformed`] if the XML writer fails (which should not occur
/// for valid in-memory inputs).
pub fn render_service_document(doc: &ServiceDocument) -> Result<String, AtomPubError> {
    let mut writer = Writer::new(Vec::new());
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))?;

    let mut root = BytesStart::new("app:service");
    root.push_attribute(("xmlns", ATOM_NS));
    root.push_attribute(("xmlns:app", APP_NS));
    writer.write_event(Event::Start(root))?;

    writer.write_event(Event::Start(BytesStart::new("app:workspace")))?;
    write_text_element(&mut writer, "atom:title", &doc.workspace_title)?;

    write_collection(&mut writer, &doc.posts_collection)?;
    write_collection(&mut writer, &doc.media_collection)?;

    writer.write_event(Event::End(BytesEnd::new("app:workspace")))?;
    writer.write_event(Event::End(BytesEnd::new("app:service")))?;

    String::from_utf8(writer.into_inner()).map_err(|e| AtomPubError::Malformed(e.to_string()))
}

fn write_collection(
    writer: &mut Writer<Vec<u8>>,
    coll: &CollectionDecl,
) -> Result<(), AtomPubError> {
    let mut start = BytesStart::new("app:collection");
    start.push_attribute(("href", coll.href.as_str()));
    writer.write_event(Event::Start(start))?;

    write_text_element(writer, "atom:title", &coll.title)?;

    for media_type in &coll.accept {
        write_text_element(writer, "app:accept", media_type)?;
    }

    if !coll.categories.is_empty() {
        let mut cat_elem = BytesStart::new("app:categories");
        cat_elem.push_attribute(("fixed", "no"));
        writer.write_event(Event::Start(cat_elem))?;

        for term in &coll.categories {
            let mut cat = BytesStart::new("atom:category");
            cat.push_attribute(("term", term.as_str()));
            writer.write_event(Event::Empty(cat))?;
        }

        writer.write_event(Event::End(BytesEnd::new("app:categories")))?;
    }

    writer.write_event(Event::End(BytesEnd::new("app:collection")))?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_document_lists_two_collections() {
        let out = render_service_document(&ServiceDocument {
            workspace_title: "Alice".into(),
            posts_collection: CollectionDecl {
                href: "https://h/atompub/alice/posts".into(),
                title: "Posts".into(),
                accept: vec!["application/atom+xml;type=entry".into()],
                categories: vec!["rust".into(), "leptos".into()],
            },
            media_collection: CollectionDecl {
                href: "https://h/atompub/alice/media".into(),
                title: "Media".into(),
                accept: vec![
                    "image/png".into(),
                    "image/jpeg".into(),
                    "image/gif".into(),
                    "image/webp".into(),
                ],
                categories: vec![],
            },
        })
        .expect("render");
        assert!(out.contains("app:service"));
        assert!(out.contains("https://h/atompub/alice/posts"));
        assert!(out.contains("type=entry"));
        assert!(out.contains("image/webp"));
        assert!(out.contains("app:categories"));
        assert!(out.contains("fixed=\"no\""));
    }
}
