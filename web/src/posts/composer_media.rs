//! Host-compiled state for Media rows in one Post composer session.
//!
//! Composer Media is deliberately transient: it records confirmed uploads for
//! display only and never models a Post-to-Media relationship.

use std::borrow::Cow;

use common::media::{Filename, InvalidFilename};
use common::root_relative_url::RootRelativeUrl;

/// One confirmed Media upload displayed by the current composer session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerMedia {
    pub url: RootRelativeUrl,
    filename: Filename,
}

impl ComposerMedia {
    /// Build a display row from the canonical uploaded URL.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFilename`] when the URL does not end in a canonical Media filename.
    pub fn try_from_uploaded_url(url: RootRelativeUrl) -> Result<Self, InvalidFilename> {
        let filename = String::from(url.clone())
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .parse()?;
        Ok(Self { url, filename })
    }

    /// Return the decoded filename users recognize.
    #[must_use]
    pub fn display_filename(&self) -> Cow<'_, str> {
        self.filename.decoded()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_root_relative_url;

    #[test]
    fn media_row_uses_the_filename_from_its_uploaded_url() {
        let media = ComposerMedia::try_from_uploaded_url(parse_root_relative_url(
            "/media/upload/sha256/photo.png",
        ))
        .expect("uploaded URL should carry a canonical filename");

        assert_eq!(media.display_filename(), "photo.png");
    }

    #[test]
    fn media_row_decodes_the_canonical_filename_for_display() {
        let media = ComposerMedia::try_from_uploaded_url(parse_root_relative_url(
            "/media/upload/sha256/my%20photo.png",
        ))
        .expect("uploaded URL should carry a canonical filename");

        assert_eq!(media.display_filename(), "my photo.png");
    }
}
