//! Host-compiled state for Media rows in one Post composer session.
//!
//! Composer Media is deliberately transient: it records confirmed uploads for
//! display only and never models a Post-to-Media relationship.

use std::borrow::Cow;

use common::media::{Filename, InvalidFilename};
use common::root_relative_url::RootRelativeUrl;
use leptos::prelude::{Get, RwSignal, Set, Update};

const INVALID_MEDIA_URL: &str = "The uploaded Media URL was invalid.";
const COPY_FAILED: &str = "Could not copy the Media URL.";

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

/// Reactive state and transitions for one composer's temporary Media rows.
#[derive(Clone, Copy)]
pub struct ComposerMediaState {
    rows: RwSignal<Vec<ComposerMedia>>,
    error: RwSignal<Option<String>>,
}

impl ComposerMediaState {
    /// Create an empty composer Media session.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: RwSignal::new(Vec::new()),
            error: RwSignal::new(None),
        }
    }

    /// Record a confirmed upload or surface an invalid returned URL.
    pub fn record_uploaded(&self, url: RootRelativeUrl) {
        match ComposerMedia::try_from_uploaded_url(url) {
            Ok(item) => {
                self.rows.update(|rows| rows.push(item));
                self.error.set(None);
            }
            Err(_) => self.error.set(Some(INVALID_MEDIA_URL.to_owned())),
        }
    }

    /// Surface an upload failure supplied by the upload widget.
    pub fn record_error(&self, message: String) {
        self.error.set(Some(message));
    }

    /// Remove one temporary row without touching its persistent Media Record.
    pub fn dismiss(&self, index: usize) {
        self.rows.update(|rows| {
            if index < rows.len() {
                rows.remove(index);
            }
        });
    }

    /// Record whether the browser clipboard write succeeded.
    pub fn settle_copy(&self, copied: bool) {
        self.error.set((!copied).then(|| COPY_FAILED.to_owned()));
    }

    /// Snapshot the rows for reactive rendering.
    #[must_use]
    pub fn rows(&self) -> Vec<ComposerMedia> {
        self.rows.get()
    }

    /// Snapshot the current user-visible error for reactive rendering.
    #[must_use]
    pub fn error(&self) -> Option<String> {
        self.error.get()
    }
}

impl Default for ComposerMediaState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_root_relative_url;
    use leptos::prelude::Owner;

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

    #[test]
    fn session_records_and_dismisses_multiple_uploads() {
        Owner::new().with(|| {
            let state = ComposerMediaState::default();
            state.record_uploaded(parse_root_relative_url("/media/upload/sha256/first.png"));
            state.record_uploaded(parse_root_relative_url("/media/upload/sha256/second.png"));

            assert_eq!(state.rows().len(), 2);
            assert_eq!(state.error(), None);

            state.dismiss(0);
            assert_eq!(state.rows().len(), 1);
            assert_eq!(state.rows()[0].display_filename(), "second.png");
            state.dismiss(99);
            assert_eq!(state.rows().len(), 1);
        });
    }

    #[test]
    fn session_surfaces_upload_and_clipboard_failures() {
        Owner::new().with(|| {
            let state = ComposerMediaState::new();
            state.record_uploaded(parse_root_relative_url("/media/upload/sha256/photo%2Epng"));
            assert_eq!(state.error().as_deref(), Some(INVALID_MEDIA_URL));

            state.record_error("Upload failed.".to_owned());
            assert_eq!(state.error().as_deref(), Some("Upload failed."));

            state.settle_copy(false);
            assert_eq!(state.error().as_deref(), Some(COPY_FAILED));
            state.settle_copy(true);
            assert_eq!(state.error(), None);
        });
    }
}
