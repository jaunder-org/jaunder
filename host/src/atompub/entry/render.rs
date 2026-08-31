//! Private upstream Atom rendering helpers.
//!
//! This leaf centralizes fallible XML decoding and role-labelled link creation;
//! document-specific assembly remains with the standalone, Collection, and media
//! rendering leaves.

use atom_syndication::Link;

use super::super::AtomPubError;
use common::tagged_url::{TaggedUrl, UrlRole};

/// Decodes what an `atom_syndication` writer produced into a `String`.
///
/// Both failure modes are unreachable in practice — a `Vec<u8>` has no I/O to
/// fail, and upstream emits UTF-8 — but neither is expressible as infallible.
pub(super) fn to_xml_string(
    written: Result<Vec<u8>, atom_syndication::Error>,
) -> Result<String, AtomPubError> {
    let bytes = written.map_err(AtomPubError::Writer)?;
    String::from_utf8(bytes).map_err(AtomPubError::Utf8)
}

/// Builds a `rel`-labelled [`Link`] — feed paging links and entry `edit` links alike.
///
/// Generic in the role: a link's `rel` attribute is what states which role the href
/// plays, so the renderer accepts any of them.
pub(super) fn rel_link<T: UrlRole>(rel: &str, href: &TaggedUrl<T>) -> Link {
    Link {
        rel: rel.to_string(),
        href: href.to_string(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
