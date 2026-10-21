//! Pure display projections for the private Theme editor's package and Media inputs.
//!
//! The CSR component consumes these projections; native tests exercise the same code
//! without mounting the browser UI.

use super::{Draft, LogoFallbackOption, ThemeBindingInput, ThemeEditorPatch, ThemeMediaInput};

/// Builds the displayed text inputs for the complete package editor from one saved draft.
#[must_use]
pub(super) fn draft_package_editor_text(draft: &Draft) -> (String, String) {
    (
        String::from_utf8_lossy(&draft.manifest).into_owned(),
        serde_json::to_string_pretty(&draft.assets).unwrap_or_default(),
    )
}

/// Stable picker identity for an already authorized staged Media reference.
#[must_use]
pub(super) fn staged_media_token(media: &ThemeMediaInput) -> String {
    format!(
        "staged-media:{}:{}:{}",
        media.source, media.sha256, media.filename
    )
}

/// Normal picker options use typed Media identity rather than page-local URLs.
#[must_use]
pub(super) fn normal_media_options(
    patch: &ThemeEditorPatch<ThemeBindingInput>,
    visible: &[ThemeMediaInput],
) -> Vec<LogoFallbackOption> {
    let staged = match patch {
        ThemeEditorPatch::Set(ThemeBindingInput::Media(media)) if !visible.contains(media) => {
            Some(media)
        }
        _ => None,
    };
    visible
        .iter()
        .chain(staged)
        .map(|media| LogoFallbackOption {
            value: staged_media_token(media),
            label: format!("Media: {}", media.filename),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::themes::ThemeEditorInput;
    use common::media::{ContentHash, Filename, MediaSource};

    fn media(digit: char, filename: &str) -> ThemeMediaInput {
        ThemeMediaInput {
            source: MediaSource::Upload,
            sha256: digit.to_string().repeat(64).parse::<ContentHash>().unwrap(),
            filename: filename.parse::<Filename>().unwrap(),
        }
    }

    #[test]
    fn package_editor_text_preserves_saved_manifest_and_serializes_assets() {
        let draft = crate::themes::Draft {
            manifest: br#"{"name":"Stored"}"#.to_vec(),
            stylesheet: b"body {}".to_vec(),
            assets: vec![],
        };
        assert_eq!(
            draft_package_editor_text(&draft),
            (r#"{"name":"Stored"}"#.into(), "[]".into())
        );
    }

    #[test]
    fn staged_media_remains_an_option_off_page_and_while_page_unavailable() {
        let logo = media('a', "logo.png");
        let header = media('b', "header.png");
        let input = ThemeEditorInput {
            name: "Local".into(),
            stylesheet: "unsaved".into(),
            logo: ThemeEditorPatch::Set(ThemeBindingInput::Media(logo.clone())),
            header: ThemeEditorPatch::Set(ThemeBindingInput::Media(header.clone())),
        };
        for visible in [
            vec![header.clone()],
            Vec::new(),
            vec![logo.clone(), header.clone()],
        ] {
            let options = normal_media_options(&input.logo, &visible);
            assert_eq!(
                options
                    .iter()
                    .filter(|option| option.value == staged_media_token(&logo))
                    .count(),
                1
            );
            assert!(
                options
                    .iter()
                    .any(|option| option.label == "Media: logo.png")
            );
            assert_eq!(
                input.logo,
                ThemeEditorPatch::Set(ThemeBindingInput::Media(logo.clone()))
            );
            let options = normal_media_options(&input.header, &visible);
            assert_eq!(
                options
                    .iter()
                    .filter(|option| option.value == staged_media_token(&header))
                    .count(),
                1
            );
        }
        assert!(normal_media_options(&ThemeEditorPatch::Keep, &[]).is_empty());
    }
}
