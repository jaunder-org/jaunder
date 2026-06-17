//! Categories Document serializer for `AtomPub` (RFC 5023).
//!
//! A Categories Document lists the category schemes and terms available
//! for entries in a collection. This module provides [`render_categories_document`]
//! to serialize a flat list of category terms to a standalone `app:categories` document.

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
use quick_xml::Writer;

use super::xml::write_empty_element;
use super::{APP_NS, ATOM_NS};

/// Serializes a list of category terms to a standalone `app:categories` document.
///
/// Emits an `app:categories` document (root) with `xmlns="ATOM_NS"`, `xmlns:app="APP_NS"`,
/// and `fixed="no"`, containing one inline `atom:category term="..."` per term.
///
/// Writes into an in-memory buffer, so it is infallible.
#[must_use]
pub fn render_categories_document(terms: &[String]) -> String {
    let mut writer = Writer::new(Vec::new());
    let _ = writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)));

    let mut root = BytesStart::new("app:categories");
    root.push_attribute(("xmlns", ATOM_NS));
    root.push_attribute(("xmlns:app", APP_NS));
    root.push_attribute(("fixed", "no"));
    let _ = writer.write_event(Event::Start(root));

    for term in terms {
        write_empty_element(&mut writer, "atom:category", &[("term", term.as_str())]);
    }

    let _ = writer.write_event(Event::End(BytesEnd::new("app:categories")));

    String::from_utf8_lossy(&writer.into_inner()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_document_contains_fixed_attribute_and_terms() {
        let out =
            render_categories_document(&["rust".into(), "programming".into(), "leptos".into()]);
        assert!(out.contains("app:categories"));
        assert!(out.contains("fixed=\"no\""));
        assert!(out.contains("term=\"rust\""));
        assert!(out.contains("term=\"programming\""));
        assert!(out.contains("term=\"leptos\""));
    }
}
