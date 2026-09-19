use common::{ids::ThemeId, pagination::PageOffset};

use super::{ThemeMediaInput, ThemePoolInput, ThemePresentation};

/// Number of owned Media records shown on one Theme Studio picker page.
pub const MEDIA_PAGE_SIZE: usize = 50;
const MEDIA_PAGE_SIZE_I64: i64 = 50;

/// Editable header-pool Media state that survives unrelated presentation refreshes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderMediaDraft {
    theme: Option<ThemeId>,
    package_paths: String,
    entries: Vec<ThemeMediaInput>,
    dirty: bool,
}

/// A current logo binding that is not one of the picker page's normal options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogoFallbackOption {
    /// Select option value used to preserve the controlled value.
    pub value: String,
    /// Human-readable description of the current binding.
    pub label: String,
}

impl HeaderMediaDraft {
    /// Seeds a clean draft, preserving dirty entries until the selected Theme changes.
    pub fn sync(&mut self, theme: Option<ThemeId>, presentation: Option<&ThemePresentation>) {
        if self.theme != theme {
            self.theme = theme;
            self.dirty = false;
        }
        if !self.dirty {
            self.package_paths = presentation.map(package_paths).unwrap_or_default();
            self.entries = presentation.map(header_media).unwrap_or_default();
        }
    }

    /// Replaces the newline-delimited package paths and marks the draft dirty.
    pub fn set_package_paths(&mut self, package_paths: String) {
        if self.package_paths != package_paths {
            self.package_paths = package_paths;
            self.dirty = true;
        }
    }

    /// Returns the newline-delimited package paths.
    #[must_use]
    pub fn package_paths(&self) -> &str {
        &self.package_paths
    }

    /// Adds a unique Media entry and marks the draft dirty.
    pub fn add(&mut self, media: ThemeMediaInput) {
        if !self.entries.contains(&media) {
            self.entries.push(media);
            self.dirty = true;
        }
    }

    /// Removes a Media entry and marks the draft dirty when it changed.
    pub fn remove(&mut self, media: &ThemeMediaInput) {
        let prior_len = self.entries.len();
        self.entries.retain(|entry| entry != media);
        self.dirty |= self.entries.len() != prior_len;
    }

    /// Returns the current ordered Media entries.
    #[must_use]
    pub fn entries(&self) -> &[ThemeMediaInput] {
        &self.entries
    }
}

/// Converts a zero-based picker page into its bounded Media-list offset.
#[must_use]
pub fn media_page_offset(page: u32) -> PageOffset {
    let offset = i64::from(page) * MEDIA_PAGE_SIZE_I64;
    PageOffset::try_from(offset).unwrap_or_default()
}

/// Describes a current package-asset or off-page Media logo for the controlled picker.
#[must_use]
pub fn logo_fallback_option(
    presentation: Option<&ThemePresentation>,
    visible_media: &[ThemeMediaInput],
) -> Option<LogoFallbackOption> {
    match presentation?.logo.as_ref()? {
        super::ThemeBindingInput::PackageAsset(path) => Some(LogoFallbackOption {
            value: format!("package-asset:{path}"),
            label: format!("Package asset: {path}"),
        }),
        super::ThemeBindingInput::Media(media) if !visible_media.contains(media) => {
            Some(LogoFallbackOption {
                value: "unavailable-media".into(),
                label: format!("Unavailable Media: {}", media.filename),
            })
        }
        _ => None,
    }
}

fn package_paths(presentation: &ThemePresentation) -> String {
    presentation
        .header_pool
        .iter()
        .filter_map(|entry| match entry {
            ThemePoolInput::PackageAsset(path) => Some(path.as_str()),
            ThemePoolInput::Media(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn header_media(presentation: &ThemePresentation) -> Vec<ThemeMediaInput> {
    presentation
        .header_pool
        .iter()
        .filter_map(|entry| match entry {
            ThemePoolInput::Media(media) => Some(media.clone()),
            ThemePoolInput::PackageAsset(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::media::{ContentHash, Filename, MediaSource};

    fn media(digit: char, filename: &str) -> ThemeMediaInput {
        ThemeMediaInput {
            source: MediaSource::Upload,
            sha256: digit.to_string().repeat(64).parse::<ContentHash>().unwrap(),
            filename: filename.parse::<Filename>().unwrap(),
        }
    }

    fn presentation(entries: Vec<ThemePoolInput>) -> ThemePresentation {
        ThemePresentation {
            logo: None,
            header: None,
            header_pool: entries,
            shuffle_seed: None,
        }
    }

    #[test]
    fn dirty_header_media_survives_same_theme_revalidation_and_reseeds_on_theme_change() {
        let first = media('a', "first.png");
        let unsaved = media('b', "unsaved.png");
        let second = media('c', "second.png");
        let theme_one = ThemeId::from(1);
        let theme_two = ThemeId::from(2);
        let mut draft = HeaderMediaDraft::default();

        draft.sync(
            Some(theme_one),
            Some(&presentation(vec![
                ThemePoolInput::PackageAsset("assets/default.png".into()),
                ThemePoolInput::Media(first.clone()),
            ])),
        );
        draft.set_package_paths(String::new());
        draft.add(unsaved.clone());
        draft.sync(
            Some(theme_one),
            Some(&presentation(vec![
                ThemePoolInput::PackageAsset("assets/default.png".into()),
                ThemePoolInput::Media(first),
            ])),
        );
        assert_eq!(draft.package_paths(), "");
        assert_eq!(draft.entries(), &[media('a', "first.png"), unsaved]);

        draft.sync(
            Some(theme_two),
            Some(&presentation(vec![ThemePoolInput::Media(second.clone())])),
        );
        assert_eq!(draft.package_paths(), "");
        assert_eq!(draft.entries(), &[second]);
    }

    #[test]
    fn removing_an_entry_marks_the_draft_dirty() {
        let first = media('a', "first.png");
        let mut draft = HeaderMediaDraft::default();
        draft.sync(
            Some(ThemeId::from(1)),
            Some(&presentation(vec![ThemePoolInput::Media(first.clone())])),
        );
        draft.remove(&first);
        draft.sync(
            Some(ThemeId::from(1)),
            Some(&presentation(vec![ThemePoolInput::Media(first)])),
        );
        assert!(draft.entries().is_empty());
    }

    #[test]
    fn logo_fallback_describes_package_assets_and_only_off_page_media() {
        let bound = media('a', "logo.png");
        let mut current = presentation(Vec::new());
        current.logo = Some(super::super::ThemeBindingInput::PackageAsset(
            "assets/logo.png".into(),
        ));
        assert_eq!(
            logo_fallback_option(Some(&current), &[]),
            Some(LogoFallbackOption {
                value: "package-asset:assets/logo.png".into(),
                label: "Package asset: assets/logo.png".into(),
            }),
        );

        current.logo = Some(super::super::ThemeBindingInput::Media(bound.clone()));
        assert!(logo_fallback_option(Some(&current), std::slice::from_ref(&bound)).is_none());
        assert_eq!(
            logo_fallback_option(Some(&current), &[]),
            Some(LogoFallbackOption {
                value: "unavailable-media".into(),
                label: "Unavailable Media: logo.png".into(),
            }),
        );
    }

    #[test]
    fn media_page_offsets_advance_by_the_bounded_page_size() {
        assert_eq!(i64::from(media_page_offset(0)), 0);
        assert_eq!(i64::from(media_page_offset(1)), 50);
        assert_eq!(i64::from(media_page_offset(3)), 150);
    }
}
