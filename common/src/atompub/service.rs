//! Service Document serializer for `AtomPub` (RFC 5023).
//!
//! A Service Document describes the collections a server supports for a given
//! workspace (e.g., one per user). This module provides [`ServiceDocument`] and
//! [`CollectionDecl`] types, plus [`render_service_document`] to serialize them
//! to XML using `quick-xml`.

use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};

use super::title::{CollectionTitle, WorkspaceTitle};
use super::xml::{write_empty_element, write_text_element};
use super::{APP_NS, ATOM_NS, J_NS};
use crate::tag::Tag;
use crate::tagged_url::CollectionHrefUrl;

/// Media range advertised for an `AtomPub` collection in a Service Document.
///
/// These discovery values are distinct from the concrete content types carried
/// by uploaded media.
#[macros::text_enum(
    error = InvalidCollectionAccept,
    message = "collection accept must be \"application/atom+xml;type=entry\" or \"*/*\""
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionAccept {
    /// Atom Entry documents.
    #[strum(serialize = "application/atom+xml;type=entry")]
    AtomEntry,
    /// Media resources of any concrete content type.
    #[strum(serialize = "*/*")]
    AnyMediaType,
}

/// Declaration of a single collection (posts or media) in a workspace.
#[derive(Debug, Clone)]
pub struct CollectionDecl {
    /// The collection's absolute IRI (#560, require-base).
    pub href: CollectionHrefUrl,
    /// User-facing title of the collection.
    pub title: CollectionTitle,
    /// Media ranges accepted by the collection.
    pub accept: Vec<CollectionAccept>,
    /// Category scheme/terms available for entries in this collection.
    /// When non-empty, an `app:categories` element with `fixed="no"` is emitted.
    pub categories: Vec<Tag>,
}

/// A complete Service Document describing the publishing surface for one workspace.
#[derive(Debug, Clone)]
pub struct ServiceDocument {
    /// Workspace title (typically a username).
    pub workspace_title: WorkspaceTitle,
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
        write_text_element(writer, "app:accept", media_type.as_ref());
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
    use crate::test_support::parse_url;

    /// A representative two-collection service document used by the serializer tests.
    fn sample_doc() -> ServiceDocument {
        let username = "alice".parse().unwrap();
        ServiceDocument {
            workspace_title: crate::atompub::WorkspaceTitle::for_user(&username),
            posts_collection: CollectionDecl {
                href: parse_url("https://h/atompub/alice/posts"),
                title: crate::atompub::CollectionTitle::posts(),
                accept: vec![CollectionAccept::AtomEntry],
                categories: vec!["rust".parse().unwrap(), "leptos".parse().unwrap()],
            },
            media_collection: CollectionDecl {
                href: parse_url("https://h/atompub/alice/media"),
                title: crate::atompub::CollectionTitle::media(),
                accept: vec![CollectionAccept::AnyMediaType],
                categories: vec![],
            },
        }
    }

    fn collection_xml<'a>(out: &'a str, href: &str) -> &'a str {
        let opening = format!(r#"<app:collection href="{href}">"#);
        out.split_once(&opening)
            .unwrap()
            .1
            .split_once("</app:collection>")
            .unwrap()
            .0
    }

    fn accept_values(collection: &str) -> Vec<&str> {
        collection
            .split("<app:accept>")
            .skip(1)
            .map(|rest| rest.split_once("</app:accept>").unwrap().0)
            .collect()
    }

    #[test]
    fn service_document_lists_each_collection_accept_range() {
        let out = render_service_document(&sample_doc());
        let posts = collection_xml(&out, "https://h/atompub/alice/posts");
        let media = collection_xml(&out, "https://h/atompub/alice/media");

        assert_eq!(
            accept_values(posts),
            vec!["application/atom+xml;type=entry"]
        );
        assert_eq!(accept_values(media), vec!["*/*"]);
        assert!(!media.contains("image/"), "media collection: {media}");
        assert!(posts.contains("app:categories"));
        assert!(posts.contains("fixed=\"no\""));
    }

    #[test]
    fn service_document_serializes_exact_workspace_and_collection_titles() {
        let out = render_service_document(&sample_doc());
        assert!(out.contains("<atom:title>alice</atom:title>"), "out: {out}");
        assert!(out.contains("<atom:title>Posts</atom:title>"), "out: {out}");
        assert!(out.contains("<atom:title>Media</atom:title>"), "out: {out}");
        assert_eq!(out.matches("<atom:title>").count(), 3, "out: {out}");
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
