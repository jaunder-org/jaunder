//! Host-compiled state for Media rows in one Post composer session.
//!
//! Composer Media is deliberately transient: it records confirmed uploads for
//! display only and never models a Post-to-Media relationship.

use common::root_relative_url::RootRelativeUrl;

/// One confirmed Media upload displayed by the current composer session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerMedia {
    pub url: RootRelativeUrl,
    pub filename: String,
}

impl ComposerMedia {
    /// Build a display row from the canonical uploaded URL.
    #[must_use]
    pub fn from_uploaded_url(url: RootRelativeUrl) -> Self {
        let filename = String::from(url.clone())
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        Self { url, filename }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_root_relative_url;

    #[test]
    fn media_row_uses_the_filename_from_its_uploaded_url() {
        let media = ComposerMedia::from_uploaded_url(parse_root_relative_url(
            "/media/upload/sha256/photo.png",
        ));

        assert_eq!(media.filename, "photo.png");
    }
}
