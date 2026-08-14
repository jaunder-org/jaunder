use thiserror::Error;

/// An `AtomPub` document could not be **written**.
///
/// There is deliberately no read counterpart. `atom_syndication` owns parsing, so
/// a document the client sent that will not parse surfaces as [`AtomError`] and
/// each consumer maps it at its own boundary (the server: a `400`). Failing to
/// write a document we composed ourselves is never the client's fault, so the two
/// directions are separate types rather than two variants of one — which is what
/// keeps a serialization failure off the `400` path.
///
/// [`AtomError`]: crate::atompub::AtomError
#[derive(Debug, Error)]
pub enum AtomPubError {
    /// `atom_syndication` could not write the document.
    #[error("failed to serialize AtomPub document: {0}")]
    Writer(#[source] atom_syndication::Error),
    /// The writer returned bytes that were not UTF-8.
    #[error("AtomPub writer returned invalid UTF-8: {0}")]
    Utf8(#[source] std::string::FromUtf8Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn writer_error_displays_its_cause() {
        assert!(
            AtomPubError::Writer(atom_syndication::Error::Eof)
                .to_string()
                .contains("end")
        );
    }

    #[test]
    fn writer_error_retains_atom_syndication_source() {
        let error = AtomPubError::Writer(atom_syndication::Error::Eof);

        let source = error.source().expect("writer failure has a source");
        assert!(matches!(
            source.downcast_ref::<atom_syndication::Error>(),
            Some(atom_syndication::Error::Eof)
        ));
    }

    #[test]
    fn invalid_utf8_error_retains_from_utf8_source() {
        let source = String::from_utf8(vec![0xff]).expect_err("byte is not UTF-8");
        let error = AtomPubError::Utf8(source);

        let source = error.source().expect("UTF-8 failure has a source");
        let source = source
            .downcast_ref::<std::string::FromUtf8Error>()
            .expect("typed FromUtf8Error");
        assert_eq!(source.as_bytes(), &[0xff]);
    }
}
