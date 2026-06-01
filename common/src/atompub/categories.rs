//! Categories Document serializer for `AtomPub` (RFC 5023).
//!
//! A Categories Document lists the category schemes and terms available
//! for entries in a collection. This module provides [`render_categories_document`]
//! to serialize a flat list of category terms to a standalone `app:categories` document.

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
use quick_xml::Writer;

use super::{AtomPubError, APP_NS, ATOM_NS};

/// Serializes a list of category terms to a standalone `app:categories` document.
///
/// Emits an `app:categories` document (root) with `xmlns="ATOM_NS"`, `xmlns:app="APP_NS"`,
/// and `fixed="no"`, containing one inline `atom:category term="..."` per term.
///
/// # Errors
///
/// Returns [`AtomPubError::Malformed`] if the XML writer fails (which should not occur
/// for valid in-memory inputs).
pub fn render_categories_document(terms: &[String]) -> Result<String, AtomPubError> {
    let mut writer = Writer::new(Vec::new());
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))?;

    let mut root = BytesStart::new("app:categories");
    root.push_attribute(("xmlns", ATOM_NS));
    root.push_attribute(("xmlns:app", APP_NS));
    root.push_attribute(("fixed", "no"));
    writer.write_event(Event::Start(root))?;

    for term in terms {
        let mut cat = BytesStart::new("atom:category");
        cat.push_attribute(("term", term.as_str()));
        writer.write_event(Event::Empty(cat))?;
    }

    writer.write_event(Event::End(BytesEnd::new("app:categories")))?;

    String::from_utf8(writer.into_inner()).map_err(|e| AtomPubError::Malformed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_document_contains_fixed_attribute_and_terms() {
        let out =
            render_categories_document(&["rust".into(), "programming".into(), "leptos".into()])
                .expect("render");
        assert!(out.contains("app:categories"));
        assert!(out.contains("fixed=\"no\""));
        assert!(out.contains("term=\"rust\""));
        assert!(out.contains("term=\"programming\""));
        assert!(out.contains("term=\"leptos\""));
    }
}
