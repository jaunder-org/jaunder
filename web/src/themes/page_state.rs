//! Host-compiled state folds for the private theme-management page.
//!
//! The component owns asynchronous resources; this leaf makes scope availability and
//! mutation revalidation explicit so an uncertain write can never leave stale catalog
//! state presented as authoritative.

use std::str::FromStr;

use common::{
    MutationOutcome,
    theme::{PublicThemeSelection, Theme},
};

use common::ids::ThemeId;
use leptos::prelude::{GetUntracked, RwSignal, Set};

use super::OwnershipScope;
use crate::reactive::Invalidator;

/// The scopes that the current session may select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeAvailability {
    /// Authentication has not resolved yet.
    Loading,
    /// An author may manage only their own catalog.
    AuthorOnly,
    /// An operator may switch between their author catalog and the site catalog.
    AuthorAndSite,
    /// There is no authenticated owner for the route.
    Anonymous,
}

impl ScopeAvailability {
    /// Builds the available scopes from the session marker rather than any caller input.
    #[must_use]
    pub const fn from_session(authenticated: bool, is_operator: bool) -> Self {
        if !authenticated {
            Self::Anonymous
        } else if is_operator {
            Self::AuthorAndSite
        } else {
            Self::AuthorOnly
        }
    }

    /// Returns whether a requested scope is valid for the visible session state.
    #[must_use]
    pub const fn permits(self, scope: OwnershipScope) -> bool {
        matches!(
            (self, scope),
            (
                Self::AuthorOnly | Self::AuthorAndSite,
                OwnershipScope::Author
            ) | (Self::AuthorAndSite, OwnershipScope::Site)
        )
    }
}

/// Reactive private-page identity state kept separate from browser-only controls.
#[derive(Clone, Copy, Debug)]
pub struct ThemePageState {
    /// Prevents overlapping import/use operations and owner-boundary changes.
    pub pending: RwSignal<bool>,
    /// The server-authorized catalog scope selected by the owner.
    pub scope: RwSignal<OwnershipScope>,
    /// The draft currently being edited.
    pub selected: RwSignal<Option<ThemeId>>,
    /// Invalidates every owner-state read after a write acknowledgement.
    pub refresh: Invalidator,
    /// The accessible feedback message for the last write.
    pub status: RwSignal<Option<String>>,
}

impl Default for ThemePageState {
    fn default() -> Self {
        Self {
            pending: RwSignal::new(false),
            scope: RwSignal::new(OwnershipScope::Author),
            selected: RwSignal::new(None),
            refresh: Invalidator::new(),
            status: RwSignal::new(None),
        }
    }
}

impl ThemePageState {
    /// Changes catalog scope and drops state that cannot cross owner boundaries.
    pub fn select_scope(self, scope: OwnershipScope) {
        if self.pending.get_untracked() {
            return;
        }
        self.scope.set(scope);
        self.selected.set(None);
        self.status.set(None);
    }

    /// Captures one custom draft target before dispatch; duplicate requests are refused.
    #[must_use]
    pub fn begin_use(self) -> Option<(OwnershipScope, ThemeId)> {
        if self.pending.get_untracked() {
            return None;
        }
        let id = self.selected.get_untracked()?;
        self.pending.set(true);
        Some((self.scope.get_untracked(), id))
    }
}

/// Decodes the existing public-selection control values, retaining inheritance for unknown tokens.
#[must_use]
pub fn selection_from_token(token: &str) -> Option<PublicThemeSelection> {
    if token == "inherit" {
        None
    } else if let Ok(theme) = Theme::from_str(token) {
        Some(PublicThemeSelection::BuiltIn(theme))
    } else {
        ThemeId::from_str(token)
            .ok()
            .map(PublicThemeSelection::Custom)
    }
}

/// Blank UI overrides omit the multipart field, allowing the validated manifest default.
#[must_use]
pub fn import_name_override(value: &str) -> Option<&str> {
    (!value.trim().is_empty()).then_some(value)
}

/// Selection is allowed only after the server confirms publication.
#[must_use]
pub fn publication_confirmed<T, E>(result: &Result<MutationOutcome<T>, E>) -> bool {
    matches!(result, Ok(MutationOutcome::Confirmed(_)))
}

/// Rebuilds a complete draft from the three editable package fields.
///
/// # Errors
///
/// Returns an error when the package asset editor is not valid JSON.
pub fn draft_from_editor(
    manifest: String,
    stylesheet: String,
    assets: &str,
) -> Result<super::Draft, String> {
    let assets = serde_json::from_str(assets)
        .map_err(|error| format!("Package assets must be valid JSON: {error}"))?;
    Ok(super::Draft {
        manifest: manifest.into_bytes(),
        stylesheet: stylesheet.into_bytes(),
        assets,
    })
}

/// The UI consequence of a completed write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Revalidation {
    /// The server confirmed the write; reload affected owner state.
    Confirmed,
    /// The commit acknowledgement was lost; reload and tell the owner why.
    Indeterminate,
    /// Nothing was written; retain current state and show this actionable error.
    Failed(String),
}

/// Folds the shared storage outcome contract into private-page behavior.
#[must_use]
pub fn revalidation<T, E: ToString>(result: Result<MutationOutcome<T>, E>) -> Revalidation {
    match result {
        Ok(MutationOutcome::Confirmed(_)) => Revalidation::Confirmed,
        Ok(MutationOutcome::CommitIndeterminate(_)) => Revalidation::Indeterminate,
        Err(error) => Revalidation::Failed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Revalidation, ScopeAvailability, ThemePageState, draft_from_editor, import_name_override,
        publication_confirmed, revalidation,
    };
    use crate::themes::OwnershipScope;
    use common::MutationOutcome;
    use leptos::prelude::{GetUntracked, Owner, Set};

    #[test]
    fn session_scope_availability_rejects_anonymous_and_allows_author_scope() {
        assert_eq!(
            ScopeAvailability::from_session(false, true),
            ScopeAvailability::Anonymous
        );
        assert!(!ScopeAvailability::Anonymous.permits(OwnershipScope::Author));
        assert!(ScopeAvailability::from_session(true, false).permits(OwnershipScope::Author));
        assert_eq!(
            ScopeAvailability::from_session(true, true),
            ScopeAvailability::AuthorAndSite
        );
        assert!(ScopeAvailability::AuthorAndSite.permits(OwnershipScope::Site));
    }

    #[test]
    fn selecting_scope_resets_draft_identity_and_feedback() {
        Owner::new().with(|| {
            let page = ThemePageState::default();
            page.selected.set(Some(common::ids::ThemeId::from(42)));
            page.status.set(Some("saved".into()));

            page.select_scope(OwnershipScope::Site);

            assert_eq!(page.scope.get_untracked(), OwnershipScope::Site);
            assert_eq!(page.selected.get_untracked(), None);
            assert_eq!(page.status.get_untracked(), None);
        });
    }

    #[test]
    fn pending_use_keeps_its_catalog_scope_and_draft_target() {
        Owner::new().with(|| {
            let page = ThemePageState::default();
            page.selected.set(Some(common::ids::ThemeId::from(42)));
            page.pending.set(true);
            page.select_scope(OwnershipScope::Site);
            assert_eq!(page.scope.get_untracked(), OwnershipScope::Author);
            assert_eq!(
                page.selected.get_untracked(),
                Some(common::ids::ThemeId::from(42))
            );
        });
    }

    #[test]
    fn blank_import_override_omits_name_but_explicit_input_is_preserved() {
        assert_eq!(import_name_override(""), None);
        assert_eq!(import_name_override(" \t\n"), None);
        assert_eq!(import_name_override("  My label  "), Some("  My label  "));
    }

    #[test]
    fn use_captures_one_target_and_refuses_duplicate_dispatch() {
        Owner::new().with(|| {
            let page = ThemePageState::default();
            assert_eq!(page.begin_use(), None);
            assert!(!page.pending.get_untracked());
            page.selected.set(Some(common::ids::ThemeId::from(42)));
            assert_eq!(
                page.begin_use(),
                Some((OwnershipScope::Author, common::ids::ThemeId::from(42)))
            );
            assert_eq!(page.begin_use(), None);
            page.pending.set(false);
            page.select_scope(OwnershipScope::Site);
            page.selected.set(Some(common::ids::ThemeId::from(7)));
            assert_eq!(
                page.begin_use(),
                Some((OwnershipScope::Site, common::ids::ThemeId::from(7)))
            );
        });
    }

    #[test]
    fn public_selection_tokens_preserve_inheritance_bundled_and_custom_choices() {
        use common::theme::{PublicThemeSelection, Theme};
        assert_eq!(super::selection_from_token("inherit"), None);
        assert_eq!(
            super::selection_from_token("studio"),
            Some(PublicThemeSelection::BuiltIn(Theme::Studio))
        );
        assert_eq!(
            super::selection_from_token("terminal"),
            Some(PublicThemeSelection::BuiltIn(Theme::Terminal))
        );
        assert_eq!(
            super::selection_from_token("reader"),
            Some(PublicThemeSelection::BuiltIn(Theme::Reader))
        );
        assert_eq!(
            super::selection_from_token("42"),
            Some(PublicThemeSelection::Custom(common::ids::ThemeId::from(42)))
        );
        assert_eq!(super::selection_from_token("not-a-theme"), None);
        assert_eq!(super::selection_from_token(""), None);
        assert_eq!(super::selection_from_token(" 42 "), None);
    }

    #[test]
    fn publication_acknowledgement_is_required_before_selection() {
        assert!(publication_confirmed(&Ok::<_, String>(
            MutationOutcome::Confirmed(())
        )));
        assert!(!publication_confirmed(&Ok::<_, String>(
            MutationOutcome::CommitIndeterminate(())
        )));
        assert!(!publication_confirmed(&Err::<MutationOutcome<()>, _>(
            "lost response"
        )));
    }

    #[test]
    fn editor_draft_preserves_utf8_source_and_decodes_assets() {
        let draft = draft_from_editor(
            r#"{"schema":1}"#.into(),
            ".theme { color: green; }".into(),
            r#"[{"path":"logo.png","mime":"image/png","bytes":[1,2,3]}]"#,
        )
        .unwrap();

        assert_eq!(draft.manifest, br#"{"schema":1}"#);
        assert_eq!(draft.stylesheet, b".theme { color: green; }");
        assert_eq!(draft.assets.len(), 1);
        assert_eq!(draft.assets[0].path, "logo.png");
        assert_eq!(draft.assets[0].bytes, [1, 2, 3]);
    }

    #[test]
    fn editor_draft_reports_invalid_asset_json_before_mutation() {
        let error = draft_from_editor("{}".into(), String::new(), "{").unwrap_err();

        assert!(
            error.starts_with("Package assets must be valid JSON:"),
            "{error}"
        );
    }

    #[test]
    fn confirmed_and_failed_writes_have_distinct_revalidation() {
        assert_eq!(
            revalidation::<(), String>(Ok(MutationOutcome::Confirmed(()))),
            Revalidation::Confirmed
        );
        assert_eq!(
            revalidation::<(), _>(Err("write rejected")),
            Revalidation::Failed("write rejected".into())
        );
    }

    #[test]
    fn indeterminate_write_requires_a_reread() {
        assert_eq!(
            revalidation::<(), String>(Ok(MutationOutcome::CommitIndeterminate(()))),
            Revalidation::Indeterminate
        );
    }
}
