use thiserror::Error;

use crate::{feed::FeedFormat, media::ContentType};

/// A rendered public Syndication Feed coupled to the format that produced it.
///
/// The renderer-specific constructors preserve in-memory serializer provenance.
/// [`SyndicationFeedRepresentation::try_from_stored`] is deliberately weaker: it
/// verifies persisted metadata agreement without reparsing the opaque body
/// (#697; ADR-0063).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyndicationFeedRepresentation(Representation);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Representation {
    Rss(String),
    Atom(String),
    Json(String),
}

/// Stored Syndication Feed metadata whose format and media type disagree.
#[derive(Debug, Error)]
#[error("stored {format:?} Syndication Feed metadata conflicts with content type {content_type}")]
pub struct MismatchedStoredSyndicationFeedMetadata {
    format: FeedFormat,
    content_type: ContentType,
}

impl SyndicationFeedRepresentation {
    /// Records provenance for bytes rendered by the RSS serializer.
    #[must_use]
    pub(crate) fn from_rss(body: String) -> Self {
        Self(Representation::Rss(body))
    }

    /// Records provenance for bytes rendered by the Atom serializer.
    #[must_use]
    pub(crate) fn from_atom(body: String) -> Self {
        Self(Representation::Atom(body))
    }

    /// Records provenance for bytes rendered by the JSON Feed serializer.
    #[must_use]
    pub(crate) fn from_json(body: String) -> Self {
        Self(Representation::Json(body))
    }

    /// Reconstructs a stored representation after verifying its metadata agrees.
    ///
    /// This does not parse the body; stored bytes can be syntactically invalid while
    /// still carrying coherent format metadata.
    ///
    /// # Errors
    ///
    /// Returns [`MismatchedStoredSyndicationFeedMetadata`] when `content_type`
    /// does not belong to `format`.
    pub fn try_from_stored(
        format: FeedFormat,
        content_type: ContentType,
        body: String,
    ) -> Result<Self, MismatchedStoredSyndicationFeedMetadata> {
        if content_type != format.content_type() {
            return Err(MismatchedStoredSyndicationFeedMetadata {
                format,
                content_type,
            });
        }

        Ok(match format {
            FeedFormat::Rss => Self::from_rss(body),
            FeedFormat::Atom => Self::from_atom(body),
            FeedFormat::Json => Self::from_json(body),
        })
    }

    /// Format established by the rendering or stored-metadata door.
    #[must_use]
    pub fn format(&self) -> FeedFormat {
        match self.0 {
            Representation::Rss(_) => FeedFormat::Rss,
            Representation::Atom(_) => FeedFormat::Atom,
            Representation::Json(_) => FeedFormat::Json,
        }
    }

    /// Media type derived from the representation's format.
    #[must_use]
    pub fn content_type(&self) -> ContentType {
        self.format().content_type()
    }

    /// Borrows the exact serialized feed bytes as UTF-8 text.
    #[must_use]
    pub fn body(&self) -> &str {
        match &self.0 {
            Representation::Rss(body) | Representation::Atom(body) | Representation::Json(body) => {
                body
            }
        }
    }

    /// Consumes the representation and returns its exact serialized body.
    #[must_use]
    pub fn into_body(self) -> String {
        match self.0 {
            Representation::Rss(body) | Representation::Atom(body) | Representation::Json(body) => {
                body
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::feed::{FeedFormat, SyndicationFeedRepresentation};

    #[test]
    fn stored_representation_exposes_agreed_metadata_and_exact_body() {
        let formats = [FeedFormat::Rss, FeedFormat::Atom, FeedFormat::Json];

        for format in formats {
            let body = format!("<{}>exact body</{}>", format.ext(), format.ext());
            let representation = SyndicationFeedRepresentation::try_from_stored(
                format,
                format.content_type(),
                body.clone(),
            )
            .unwrap();

            assert_eq!(representation.format(), format);
            assert_eq!(representation.content_type(), format.content_type());
            assert_eq!(representation.body(), body);
            assert_eq!(representation.into_body(), body);

            for mismatched_content_type in formats {
                if mismatched_content_type != format {
                    assert!(
                        SyndicationFeedRepresentation::try_from_stored(
                            format,
                            mismatched_content_type.content_type(),
                            "<opaque body>".to_owned(),
                        )
                        .is_err()
                    );
                }
            }
        }
    }
}
