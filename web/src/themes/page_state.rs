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
use crate::{error::WebError, reactive::Invalidator};

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
    /// Prevents overlapping theme operations and owner-boundary changes.
    pub pending: RwSignal<bool>,
    /// The server-authorized catalog scope selected by the owner.
    pub scope: RwSignal<OwnershipScope>,
    /// The draft currently being edited.
    pub selected: RwSignal<Option<ThemeId>>,
    /// Invalidates every owner-state read after a write acknowledgement.
    pub refresh: Invalidator,
    /// The accessible feedback message for the last write.
    pub status: RwSignal<Option<String>>,
    /// Places import feedback beside its picker rather than below Advanced authoring.
    pub feedback_in_import: RwSignal<bool>,
    /// Styles failed or uncertain operations without implying rollback.
    pub feedback_error: RwSignal<bool>,
}

impl Default for ThemePageState {
    fn default() -> Self {
        Self {
            pending: RwSignal::new(false),
            scope: RwSignal::new(OwnershipScope::Author),
            selected: RwSignal::new(None),
            refresh: Invalidator::new(),
            status: RwSignal::new(None),
            feedback_in_import: RwSignal::new(false),
            feedback_error: RwSignal::new(false),
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
        self.feedback_in_import.set(false);
        self.feedback_error.set(false);
    }

    /// Serializes every theme operation in this page so draft writes cannot race activation.
    #[must_use]
    pub fn begin_mutation(self) -> bool {
        if self.pending.get_untracked() {
            return false;
        }
        self.pending.set(true);
        self.feedback_in_import.set(false);
        self.feedback_error.set(false);
        true
    }

    /// Captures one custom draft target before dispatch; duplicate requests are refused.
    #[must_use]
    pub fn begin_use(self) -> Option<(OwnershipScope, ThemeId)> {
        if self.pending.get_untracked() {
            return None;
        }
        let id = self.selected.get_untracked()?;
        self.begin_mutation()
            .then_some((self.scope.get_untracked(), id))
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

/// Acknowledgement known at the browser boundary; uncertainty never becomes rejection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Receipt {
    Confirmed,
    Rejected(String),
    Uncertain(String),
}

/// Existing operations remain separate stages; applying may follow a confirmed publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationStage {
    Import,
    Publish,
    PublishOnly,
    Apply { published: bool },
}

/// Distinguishes typed admission rejection from uncertain dispatch/commit acknowledgement.
#[must_use]
pub fn mutation_receipt<T>(result: &Result<MutationOutcome<T>, WebError>) -> Receipt {
    match result {
        Ok(MutationOutcome::Confirmed(_)) => Receipt::Confirmed,
        Ok(MutationOutcome::CommitIndeterminate(_)) => {
            Receipt::Uncertain("The database acknowledgement is uncertain.".into())
        }
        Err(
            error @ (WebError::Unauthorized
            | WebError::Forbidden { .. }
            | WebError::NotFound { .. }
            | WebError::Validation { .. }
            | WebError::Conflict { .. }),
        ) => Receipt::Rejected(error.to_string()),
        Err(error) => Receipt::Uncertain(error.to_string()),
    }
}

/// Truthful stage-specific feedback, independent of DOM and transport implementation.
#[must_use]
pub fn mutation_message(stage: MutationStage, receipt: &Receipt) -> String {
    let message = match (stage, receipt) {
        (MutationStage::Import, Receipt::Confirmed) => {
            "Imported as a private draft. Use this theme to publish and apply it."
        }
        (MutationStage::Import, Receipt::Rejected(_)) => {
            "Import rejected. Choose a valid Theme Package ZIP; use Advanced to override a reserved, too-long or duplicate catalog label."
        }
        (MutationStage::Import, Receipt::Uncertain(_)) => {
            "Import outcome is uncertain. Review the reread catalog before importing again; no automatic retry ran."
        }
        (MutationStage::Publish, Receipt::Confirmed) => "Publication confirmed. Applying theme…",
        (MutationStage::PublishOnly, Receipt::Confirmed) => {
            "Publication confirmed. No selection request was sent; an already-selected theme now uses the new revision."
        }
        (MutationStage::Publish | MutationStage::PublishOnly, Receipt::Rejected(_)) => {
            "Publication rejected. Selection was not requested. Fix the draft in Advanced before using this theme again."
        }
        (MutationStage::Publish | MutationStage::PublishOnly, Receipt::Uncertain(_)) => {
            "Publication outcome is uncertain. Selection was not requested. An already-selected theme may have advanced. Review reread state before trying again; no automatic retry ran."
        }
        (MutationStage::Apply { published: true }, Receipt::Confirmed) => {
            "Theme published and applied. Private management and Home are unchanged."
        }
        (MutationStage::Apply { published: false }, Receipt::Confirmed) => {
            "Public selection applied. Private management and Home are unchanged."
        }
        (MutationStage::Apply { published: true }, Receipt::Rejected(_)) => {
            "Publication confirmed, but applying was rejected. Publication cannot be undone by selection failure; an already-selected theme now uses the new revision. Review current selection before choosing another theme."
        }
        (MutationStage::Apply { published: false }, Receipt::Rejected(_)) => {
            "Applying was rejected. Review current selection and choose another theme."
        }
        (MutationStage::Apply { published: true }, Receipt::Uncertain(_)) => {
            "Publication confirmed, but applying outcome is uncertain. Publication cannot be undone by selection failure; an already-selected theme now uses the new revision. Review reread state before trying again; no automatic retry ran."
        }
        (MutationStage::Apply { published: false }, Receipt::Uncertain(_)) => {
            "Applying outcome is uncertain. Review reread state before trying again; no automatic retry ran."
        }
    };
    match receipt {
        Receipt::Confirmed => message.into(),
        Receipt::Rejected(detail) | Receipt::Uncertain(detail) => format!("{message} {detail}"),
    }
}

/// Rereads report current state, never proof that an uncertain operation committed or rolled back.
#[must_use]
pub fn reconciliation_message(
    catalog: &Result<Vec<super::CatalogEntry>, WebError>,
    selection: &Result<Option<PublicThemeSelection>, WebError>,
    target: Option<ThemeId>,
) -> String {
    let catalog_message = match catalog {
        Ok(entries) => {
            let target_message = target.map(|id| match entries.iter().find(|entry| entry.id == id) {
                Some(entry) if entry.published => "Chosen theme currently has a published revision.",
                Some(_) => "Chosen theme currently has no published revision.",
                None => "Chosen theme is absent from the reread catalog; reopen Themes before continuing.",
            }).unwrap_or_default();
            format!("Catalog reread: {} themes. {target_message}", entries.len())
        }
        Err(error) => format!(
            "Catalog reread failed: {error}. Publication state is unknown; reopen Themes to review it."
        ),
    };
    let selection_message = match selection {
        Ok(selection) => format!(
            "Current selection from reread: {}.",
            selection_token(*selection)
        ),
        Err(error) => format!(
            "Selection reread failed: {error}. Current selection is unknown; reopen Themes to review it."
        ),
    };
    format!("{catalog_message} {selection_message}")
}

/// Unread or failed current selection is an empty, disabled UI placeholder, not inheritance.
/// The caller renders loading/errors separately and permits changes only after a successful read.
#[must_use]
pub fn selection_control_token(
    state: Option<&Result<Option<PublicThemeSelection>, WebError>>,
) -> String {
    match state {
        Some(Ok(selection)) => selection_token(*selection),
        None | Some(Err(_)) => String::new(),
    }
}

/// Native select values preserve the existing bundled/custom/inheritance wire choices.
#[must_use]
pub fn selection_token(selection: Option<PublicThemeSelection>) -> String {
    match selection {
        None => "inherit".into(),
        Some(PublicThemeSelection::BuiltIn(theme)) => theme.to_string(),
        Some(PublicThemeSelection::Custom(id)) => id.to_string(),
    }
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
    /// The operation returned an error; this alone does not establish whether it wrote.
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
            page.feedback_in_import.set(true);
            page.feedback_error.set(true);

            page.select_scope(OwnershipScope::Site);

            assert_eq!(page.scope.get_untracked(), OwnershipScope::Site);
            assert_eq!(page.selected.get_untracked(), None);
            assert_eq!(page.status.get_untracked(), None);
            assert!(!page.feedback_in_import.get_untracked());
            assert!(!page.feedback_error.get_untracked());
        });
    }

    #[test]
    fn pending_use_keeps_its_catalog_scope_and_draft_target() {
        Owner::new().with(|| {
            let page = ThemePageState::default();
            page.selected.set(Some(common::ids::ThemeId::from(42)));
            page.pending.set(true);
            page.feedback_in_import.set(true);
            page.feedback_error.set(true);
            page.select_scope(OwnershipScope::Site);
            assert_eq!(page.scope.get_untracked(), OwnershipScope::Author);
            assert_eq!(
                page.selected.get_untracked(),
                Some(common::ids::ThemeId::from(42))
            );
            assert!(page.feedback_in_import.get_untracked());
            assert!(page.feedback_error.get_untracked());
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
    fn pending_draft_operation_blocks_activation_and_other_operations() {
        Owner::new().with(|| {
            let page = ThemePageState::default();
            page.selected.set(Some(common::ids::ThemeId::from(42)));
            page.feedback_error.set(true);
            page.feedback_in_import.set(true);
            assert!(page.begin_mutation());
            assert!(!page.feedback_error.get_untracked());
            assert!(!page.feedback_in_import.get_untracked());
            assert!(!page.begin_mutation());
            assert_eq!(page.begin_use(), None);
            page.select_scope(OwnershipScope::Site);
            assert_eq!(page.scope.get_untracked(), OwnershipScope::Author);
            page.pending.set(false);
            assert!(page.begin_use().is_some());
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
    fn selection_control_distinguishes_unknown_reads_from_known_inheritance() {
        use super::selection_control_token;
        use crate::error::WebError;
        use common::theme::{PublicThemeSelection, Theme};
        assert_eq!(selection_control_token(None), "");
        assert_eq!(
            selection_control_token(Some(&Err(WebError::server_function("read failed")))),
            ""
        );
        assert_eq!(selection_control_token(Some(&Ok(None))), "inherit");
        assert_eq!(
            selection_control_token(Some(&Ok(Some(PublicThemeSelection::Custom(
                common::ids::ThemeId::from(42)
            ))))),
            "42"
        );
        assert_eq!(
            selection_control_token(Some(&Ok(Some(PublicThemeSelection::BuiltIn(
                Theme::Reader
            ))))),
            "reader"
        );
    }

    #[test]
    fn receipts_distinguish_rejection_from_commit_transport_and_decode_uncertainty() {
        use super::{Receipt, mutation_receipt};
        use crate::error::WebError;
        assert_eq!(
            mutation_receipt(&Ok::<_, WebError>(MutationOutcome::Confirmed(()))),
            Receipt::Confirmed
        );
        assert!(matches!(
            mutation_receipt(&Ok::<_, WebError>(MutationOutcome::CommitIndeterminate(()))),
            Receipt::Uncertain(_)
        ));
        for error in [
            WebError::Unauthorized,
            WebError::forbidden("site scope"),
            WebError::not_found("draft"),
            WebError::validation("invalid package"),
            WebError::conflict("duplicate name"),
        ] {
            assert_eq!(
                mutation_receipt::<()>(&Err(error.clone())),
                Receipt::Rejected(error.to_string())
            );
        }
        for error in [
            WebError::Storage {
                message: "database unavailable".into(),
            },
            WebError::server_message("unexpected failure"),
            WebError::server_function("lost response"),
            WebError::ServerFunctionInput {
                message: "unclassified response".into(),
            },
        ] {
            assert_eq!(
                mutation_receipt::<()>(&Err(error.clone())),
                Receipt::Uncertain(error.to_string())
            );
        }
    }

    #[test]
    fn stage_feedback_never_equates_rejection_uncertainty_and_application() {
        use super::{MutationStage, Receipt, mutation_message};
        for (stage, expected) in [
            (MutationStage::Import, "Imported as a private draft"),
            (MutationStage::Publish, "Publication confirmed"),
            (MutationStage::PublishOnly, "Publication confirmed"),
            (
                MutationStage::Apply { published: true },
                "Theme published and applied",
            ),
            (
                MutationStage::Apply { published: false },
                "Public selection applied",
            ),
        ] {
            assert!(mutation_message(stage, &Receipt::Confirmed).starts_with(expected));
            let rejected = mutation_message(stage, &Receipt::Rejected("detail".into()));
            assert!(
                rejected.contains("rejected") && rejected.ends_with("detail"),
                "{rejected}"
            );
            let uncertain = mutation_message(stage, &Receipt::Uncertain("lost response".into()));
            assert!(
                uncertain.contains("uncertain") && uncertain.contains("no automatic retry"),
                "{uncertain}"
            );
            assert!(!uncertain.contains("Theme published and applied"));
        }
        let rejected = mutation_message(MutationStage::Publish, &Receipt::Rejected("CSS".into()));
        assert!(
            rejected.contains("Selection was not requested") && rejected.contains("Fix the draft")
        );
        assert!(!rejected.contains("may have advanced"));
        let applied = mutation_message(
            MutationStage::Apply { published: true },
            &Receipt::Rejected("selection".into()),
        );
        assert!(
            applied.contains("Publication confirmed")
                && applied.contains("cannot be undone")
                && applied.contains("already-selected")
        );
    }

    #[test]
    fn rereads_report_current_publication_and_selection_without_resolving_uncertain_write() {
        use super::reconciliation_message;
        use crate::{error::WebError, themes::CatalogEntry};
        use common::theme::{PublicThemeSelection, Theme};
        let id = common::ids::ThemeId::from(42);
        for (published, expected) in [
            (true, "has a published revision"),
            (false, "has no published revision"),
        ] {
            let catalog = Ok(vec![CatalogEntry {
                id,
                name: "Night".into(),
                published,
            }]);
            let message = reconciliation_message(
                &catalog,
                &Ok(Some(PublicThemeSelection::Custom(id))),
                Some(id),
            );
            assert!(
                message.contains(expected)
                    && message.contains("Current selection from reread: 42."),
                "{message}"
            );
            assert!(!message.contains("confirmed"));
        }
        let missing = reconciliation_message(&Ok(Vec::new()), &Ok(None), Some(id));
        assert!(missing.contains("absent") && missing.contains("inherit"));
        let bundled = reconciliation_message(
            &Ok(Vec::new()),
            &Ok(Some(PublicThemeSelection::BuiltIn(Theme::Reader))),
            None,
        );
        assert!(bundled.contains("reader"));
        let failure = Err(WebError::server_function("reread unavailable"));
        let both = reconciliation_message(
            &failure,
            &Err(WebError::server_function("read failed")),
            Some(id),
        );
        assert!(
            both.contains("Publication state is unknown")
                && both.contains("Current selection is unknown")
        );
        assert!(!both.contains("inherit"));
        assert!(
            reconciliation_message(&failure, &Ok(None), None)
                .contains("Current selection from reread: inherit")
        );
        assert!(
            reconciliation_message(
                &Ok(Vec::new()),
                &Err(WebError::server_function("read failed")),
                None
            )
            .contains("Selection reread failed")
        );
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
