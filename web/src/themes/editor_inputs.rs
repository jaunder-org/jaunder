//! Pure display projections for the private Theme editor's package and Media inputs.
//!
//! The CSR component consumes these projections; native tests exercise the same code
//! without mounting the browser UI.

use std::sync::atomic::{AtomicBool, Ordering};

use common::{MutationOutcome, ids::ThemeId, theme::ThemeImageRole};
use leptos::prelude::*;

use super::page_state::{
    EditorSnapshot, EditorState, Feedback, Receipt, ThemeChoice, ThemePageState, mutation_receipt,
};
use crate::error::WebError;

use super::{
    CatalogEntry, Draft, LogoFallbackOption, OwnershipScope, ThemeBindingInput, ThemeEditorPatch,
    ThemeEditorSaveResult, ThemeMediaInput, ThemePresentation,
};

/// Failed editor reads distinguish teardown from a changed selection.
pub(super) fn require_editor_lifetime(lifetime: &AtomicBool) -> Result<(), WebError> {
    if lifetime.load(Ordering::Acquire) {
        Ok(())
    } else {
        Err(WebError::server_message("Theme editor has closed"))
    }
}

pub(super) fn editor_catalog_name(
    id: ThemeId,
    entries: &[CatalogEntry],
) -> Result<String, WebError> {
    entries
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.name.clone())
        .ok_or_else(|| WebError::server_message("Theme is no longer available"))
}

impl ThemePageState {
    /// Adopt a loaded source only into the still-selected owner-bound editor.
    pub(super) fn loaded_editor(
        self,
        scope: OwnershipScope,
        target: Option<ThemeId>,
        name: String,
        source: Draft,
        current: Option<&ThemePresentation>,
    ) -> Result<(EditorState, Draft), WebError> {
        let choice = target.map_or(ThemeChoice::New, ThemeChoice::Custom);
        if self.scope.get_untracked() != scope || self.selected.get_untracked() != Some(choice) {
            return Err(WebError::server_message("Theme editor has changed"));
        }
        let editor = self.editor(
            target,
            name,
            String::from_utf8_lossy(&source.stylesheet).into_owned(),
            current.is_some_and(|current| current.shuffle_seed.is_some()),
        );
        Ok((editor, source))
    }

    /// Acknowledgements update buffers only when confirmed; uncertainty offers one diagnostic read.
    pub(super) fn save_feedback(
        self,
        editor: EditorState,
        snapshot: &EditorSnapshot,
        result: &Result<MutationOutcome<ThemeEditorSaveResult>, WebError>,
    ) -> Option<ThemeId> {
        match result {
            Ok(MutationOutcome::Confirmed(saved)) => {
                let clean = editor.confirm(snapshot, saved);
                if editor.target.get_untracked() == Some(saved.id) {
                    self.selected.set(Some(ThemeChoice::Custom(saved.id)));
                }
                self.feedback.set(Feedback::Success);
                let mut message = if saved.applied {
                    "Changes saved. Public appearance updated.".to_owned()
                } else {
                    "Changes saved.".to_owned()
                };
                if !clean {
                    message.push_str(" Newer edits are still unsaved.");
                }
                self.status.set(Some(message));
                None
            }
            Ok(MutationOutcome::CommitIndeterminate(saved)) => {
                self.feedback.set(Feedback::Error);
                self.status.set(Some("The server could not confirm the save. Your edits are still here. Check the saved theme before trying again; no automatic retry ran.".into()));
                Some(saved.id)
            }
            Err(error) => {
                self.feedback.set(Feedback::Error);
                let message = if matches!(mutation_receipt(result), Receipt::Rejected(_)) {
                    format!("Save was rejected: {error}. Your edits are still here.")
                } else {
                    format!(
                        "The server could not confirm the save. It may already have been saved. Your edits are still here. Check available themes and your current setting before trying again; no automatic retry ran. {error}"
                    )
                };
                self.status.set(Some(message));
                None
            }
        }
    }

    pub(super) fn save_diagnostic(self, diagnostic: &Result<Draft, WebError>) {
        if let Err(error) = diagnostic {
            self.status.update(|message| {
                if let Some(message) = message {
                    use std::fmt::Write as _;
                    let _ = write!(message, " Saved content could not be checked: {error}");
                }
            });
        }
    }
}

/// Keyboard navigation changes only the permitted setting, never a pending operation.
pub(super) fn tab_destination(
    pending: bool,
    current: OwnershipScope,
    key: &str,
) -> Option<OwnershipScope> {
    if pending {
        return None;
    }
    match key {
        "ArrowLeft" | "ArrowRight" => Some(match current {
            OwnershipScope::Author => OwnershipScope::Site,
            OwnershipScope::Site => OwnershipScope::Author,
        }),
        "Home" => Some(OwnershipScope::Author),
        "End" => Some(OwnershipScope::Site),
        _ => None,
    }
}

/// Describes saved provenance without turning it into a replacement grant.
pub(super) fn kept_image_label(role: ThemeImageRole, current: Option<ThemePresentation>) -> String {
    let Some(current) = current else {
        return "Keep current image".into();
    };
    if role == ThemeImageRole::Header && current.shuffle_seed.is_some() {
        return "Keep rotating header images".into();
    }
    let binding = match role {
        ThemeImageRole::Logo => current.logo,
        ThemeImageRole::Header => current.header,
    };
    match binding {
        None | Some(ThemeBindingInput::PackagedDefault) => "Keep theme default".into(),
        Some(ThemeBindingInput::ExplicitAbsent) => "Keep no image".into(),
        Some(ThemeBindingInput::PackageAsset(path)) => format!("Keep theme image: {path}"),
        Some(ThemeBindingInput::Media(media)) => format!("Keep current image: {}", media.filename),
    }
}

/// The picker round-trips the staged patch, including an explicit Keep.
pub(super) fn image_patch_token(patch: &ThemeEditorPatch<ThemeBindingInput>) -> String {
    match patch {
        ThemeEditorPatch::Keep => "keep".into(),
        ThemeEditorPatch::Set(ThemeBindingInput::PackagedDefault) => "package-default".into(),
        ThemeEditorPatch::Set(ThemeBindingInput::ExplicitAbsent) => "none".into(),
        ThemeEditorPatch::Set(ThemeBindingInput::PackageAsset(asset_path)) => {
            format!("package-asset:{asset_path}")
        }
        ThemeEditorPatch::Set(ThemeBindingInput::Media(media)) => staged_media_token(media),
    }
}

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

    fn draft() -> Draft {
        Draft {
            manifest: Vec::new(),
            stylesheet: b"body {}".to_vec(),
            assets: Vec::new(),
        }
    }

    #[test]
    fn picker_tokens_and_saved_labels_preserve_every_provenance() {
        assert_eq!(image_patch_token(&ThemeEditorPatch::Keep), "keep");
        assert_eq!(
            kept_image_label(ThemeImageRole::Logo, None),
            "Keep current image"
        );
        for (binding, token, label) in [
            (
                None,
                "package-default".to_owned(),
                "Keep theme default".to_owned(),
            ),
            (
                Some(ThemeBindingInput::PackagedDefault),
                "package-default".into(),
                "Keep theme default".into(),
            ),
            (
                Some(ThemeBindingInput::ExplicitAbsent),
                "none".into(),
                "Keep no image".into(),
            ),
            (
                Some(ThemeBindingInput::PackageAsset("logo.png".into())),
                "package-asset:logo.png".into(),
                "Keep theme image: logo.png".into(),
            ),
            (
                Some(ThemeBindingInput::Media(media('a', "mine.png"))),
                staged_media_token(&media('a', "mine.png")),
                "Keep current image: mine.png".into(),
            ),
        ] {
            if let Some(binding) = &binding {
                assert_eq!(
                    image_patch_token(&ThemeEditorPatch::Set(binding.clone())),
                    token
                );
            }
            let current = ThemePresentation {
                logo: binding.clone(),
                header: binding,
                header_pool: Vec::new(),
                shuffle_seed: None,
            };
            for role in [ThemeImageRole::Logo, ThemeImageRole::Header] {
                assert_eq!(kept_image_label(role, Some(current.clone())), label);
            }
            let rotating = ThemePresentation {
                shuffle_seed: Some([0; 32]),
                ..current
            };
            assert_eq!(
                kept_image_label(ThemeImageRole::Header, Some(rotating.clone())),
                "Keep rotating header images"
            );
            assert_eq!(
                kept_image_label(ThemeImageRole::Logo, Some(rotating)),
                label
            );
        }
    }

    #[test]
    fn tab_keys_refuse_pending_and_preserve_arrow_home_end_behavior() {
        for current in [OwnershipScope::Author, OwnershipScope::Site] {
            for key in ["ArrowLeft", "ArrowRight", "Home", "End", "Enter"] {
                assert_eq!(tab_destination(true, current, key), None);
            }
            let other = if current == OwnershipScope::Author {
                OwnershipScope::Site
            } else {
                OwnershipScope::Author
            };
            assert_eq!(tab_destination(false, current, "ArrowLeft"), Some(other));
            assert_eq!(tab_destination(false, current, "ArrowRight"), Some(other));
            assert_eq!(
                tab_destination(false, current, "Home"),
                Some(OwnershipScope::Author)
            );
            assert_eq!(
                tab_destination(false, current, "End"),
                Some(OwnershipScope::Site)
            );
            assert_eq!(tab_destination(false, current, "Enter"), None);
        }
    }

    #[test]
    fn loaded_editor_requires_selected_scope_and_retains_cached_unsaved_input() {
        let id = ThemeId::from(42);
        let entries = vec![CatalogEntry {
            id,
            name: "Saved".into(),
            published: false,
        }];
        assert_eq!(editor_catalog_name(id, &entries).unwrap(), "Saved");
        assert!(editor_catalog_name(ThemeId::from(43), &entries).is_err());
        let owner = Owner::new();
        let lifetime = owner.with(|| {
            let page = ThemePageState::default();
            page.select_choice(ThemeChoice::Custom(id));
            let current = ThemePresentation {
                logo: None,
                header: None,
                header_pool: Vec::new(),
                shuffle_seed: Some([0; 32]),
            };
            let (editor, source) = page
                .loaded_editor(
                    OwnershipScope::Author,
                    Some(id),
                    "Saved".into(),
                    draft(),
                    Some(&current),
                )
                .unwrap();
            assert_eq!(source, draft());
            assert!(editor.rotating.get_untracked());
            editor.stylesheet.set("unsaved".into());
            let (same, _) = page
                .loaded_editor(
                    OwnershipScope::Author,
                    Some(id),
                    "Reloaded".into(),
                    draft(),
                    None,
                )
                .unwrap();
            assert_eq!(same.stylesheet.get_untracked(), "unsaved");
            assert!(!same.rotating.get_untracked());
            assert!(
                page.loaded_editor(OwnershipScope::Site, Some(id), String::new(), draft(), None)
                    .is_err()
            );
            assert!(
                page.loaded_editor(OwnershipScope::Author, None, String::new(), draft(), None)
                    .is_err()
            );
            let lifetime = page.lifetime();
            assert!(require_editor_lifetime(&lifetime).is_ok());
            lifetime
        });
        owner.cleanup();
        assert!(require_editor_lifetime(&lifetime).is_err());
    }

    #[test]
    fn save_feedback_adopts_only_confirmed_ids_and_keeps_newer_edits() {
        Owner::new().with(|| {
            let page = ThemePageState::default();
            page.select_choice(ThemeChoice::New);
            let editor = page.editor(None, "Local".into(), "style".into(), false);
            let snapshot = editor.snapshot();
            let saved = ThemeEditorSaveResult {
                id: ThemeId::from(42),
                name: "Server".into(),
                applied: false,
            };
            assert_eq!(
                page.save_feedback(
                    editor,
                    &snapshot,
                    &Ok(MutationOutcome::CommitIndeterminate(saved.clone()))
                ),
                Some(saved.id)
            );
            assert_eq!(editor.target.get_untracked(), None);
            assert_eq!(page.selected.get_untracked(), Some(ThemeChoice::New));
            assert_eq!(page.feedback.get_untracked(), Feedback::Error);
            page.save_diagnostic(&Ok(draft()));
            page.save_diagnostic(&Err(WebError::server_message("read failed")));
            assert!(
                page.status
                    .get_untracked()
                    .unwrap()
                    .contains("Saved content could not be checked")
            );
            page.status.set(None);
            page.save_diagnostic(&Err(WebError::server_message("read failed")));
            for error in [
                WebError::Validation {
                    message: "invalid".into(),
                },
                WebError::server_message("transport"),
            ] {
                assert_eq!(page.save_feedback(editor, &snapshot, &Err(error)), None);
                assert_eq!(editor.target.get_untracked(), None);
                assert_eq!(editor.stylesheet.get_untracked(), "style");
            }
            editor.stylesheet.set("newer".into());
            editor.changed();
            assert_eq!(
                page.save_feedback(
                    editor,
                    &snapshot,
                    &Ok(MutationOutcome::Confirmed(saved.clone()))
                ),
                None
            );
            assert_eq!(editor.target.get_untracked(), Some(saved.id));
            assert_eq!(
                page.selected.get_untracked(),
                Some(ThemeChoice::Custom(saved.id))
            );
            assert_eq!(editor.stylesheet.get_untracked(), "newer");
            assert!(
                page.status
                    .get_untracked()
                    .unwrap()
                    .ends_with("Newer edits are still unsaved.")
            );
            for applied in [false, true] {
                let snapshot = editor.snapshot();
                let saved = ThemeEditorSaveResult {
                    applied,
                    ..saved.clone()
                };
                page.save_feedback(editor, &snapshot, &Ok(MutationOutcome::Confirmed(saved)));
                assert_eq!(page.feedback.get_untracked(), Feedback::Success);
                assert!(!page.status.get_untracked().unwrap().contains("unsaved"));
            }
            let mut stale = editor.snapshot();
            stale.target = Some(ThemeId::from(99));
            let other = ThemeEditorSaveResult {
                id: ThemeId::from(99),
                ..saved
            };
            page.save_feedback(editor, &stale, &Ok(MutationOutcome::Confirmed(other)));
            assert_eq!(editor.target.get_untracked(), Some(ThemeId::from(42)));
        });
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
