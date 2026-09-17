//! Host-compiled disclosure ownership for the Post composer control grid.

use common::root_relative_url::RootRelativeUrl;
use common::visibility::AudienceBase;
use leptos::prelude::{Get, RwSignal, Set};

use super::composer_media::ComposerMediaState;

/// One compact control in the Post composer rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposerControl {
    Media,
    Format,
    Slug,
    Publish,
    Audience,
}

/// Local single-open disclosure state shared by one mounted Post composer.
#[derive(Clone, Copy)]
pub struct ComposerDisclosureState {
    open: RwSignal<Option<ComposerControl>>,
}

impl ComposerDisclosureState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            open: RwSignal::new(None),
        }
    }

    pub fn toggle(self, control: ComposerControl) {
        self.open
            .set((self.open.get() != Some(control)).then_some(control));
    }

    pub fn open(self, control: ComposerControl) {
        self.open.set(Some(control));
    }

    #[must_use]
    pub fn open_control(self) -> Option<ComposerControl> {
        self.open.get()
    }

    #[must_use]
    pub fn is_open(self, control: ComposerControl) -> bool {
        self.open.get() == Some(control)
    }
}

impl Default for ComposerDisclosureState {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use]
pub fn slug_disclosure_value(value: &str) -> String {
    if value.trim().is_empty() {
        "auto".to_owned()
    } else {
        value.to_owned()
    }
}

#[must_use]
pub fn publish_disclosure_value(value: &str) -> String {
    if value.trim().is_empty() {
        "Now".to_owned()
    } else {
        value.replace('T', " ")
    }
}

#[must_use]
pub const fn audience_disclosure_value(base: AudienceBase) -> &'static str {
    match base {
        AudienceBase::Private => "Private",
        AudienceBase::Public => "Public",
        AudienceBase::Subscribers => "Subscribers",
    }
}

pub fn record_media_upload(
    media: ComposerMediaState,
    disclosures: ComposerDisclosureState,
    url: RootRelativeUrl,
) {
    if !media.record_uploaded(url) {
        disclosures.open(ComposerControl::Media);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_root_relative_url;
    use leptos::prelude::Owner;

    #[test]
    fn one_composer_control_is_open_at_a_time() {
        Owner::new().with(|| {
            let state = ComposerDisclosureState::default();
            assert_eq!(state.open_control(), None);

            state.toggle(ComposerControl::Format);
            assert_eq!(state.open_control(), Some(ComposerControl::Format));

            state.toggle(ComposerControl::Audience);
            assert_eq!(state.open_control(), Some(ComposerControl::Audience));

            state.toggle(ComposerControl::Audience);
            assert_eq!(state.open_control(), None);
        });
    }

    #[test]
    fn media_failure_can_force_media_open() {
        Owner::new().with(|| {
            let state = ComposerDisclosureState::new();
            state.toggle(ComposerControl::Slug);
            state.open(ComposerControl::Media);

            assert_eq!(state.open_control(), Some(ComposerControl::Media));
            assert!(state.is_open(ComposerControl::Media));
            assert!(!state.is_open(ComposerControl::Slug));
        });
    }

    #[test]
    fn invalid_uploaded_media_forces_media_open() {
        Owner::new().with(|| {
            let media = ComposerMediaState::new();
            let disclosures = ComposerDisclosureState::new();
            record_media_upload(
                media,
                disclosures,
                parse_root_relative_url("/media/upload/sha256/photo.png"),
            );
            assert_eq!(disclosures.open_control(), None);

            record_media_upload(
                media,
                disclosures,
                parse_root_relative_url("/media/upload/sha256/photo%2Epng"),
            );
            assert_eq!(media.summary(), "Upload failed");
            assert_eq!(disclosures.open_control(), Some(ComposerControl::Media));
        });
    }

    #[test]
    fn compact_values_preserve_non_default_inputs() {
        assert_eq!(slug_disclosure_value("  "), "auto");
        assert_eq!(slug_disclosure_value("custom-slug"), "custom-slug");
        assert_eq!(publish_disclosure_value(""), "Now");
        assert_eq!(
            publish_disclosure_value("2026-09-18T14:30"),
            "2026-09-18 14:30"
        );
        assert_eq!(audience_disclosure_value(AudienceBase::Private), "Private");
        assert_eq!(audience_disclosure_value(AudienceBase::Public), "Public");
        assert_eq!(
            audience_disclosure_value(AudienceBase::Subscribers),
            "Subscribers"
        );
    }
}
