//! Host-compiled state folds for the private theme-management page.
//!
//! The component owns asynchronous resources; this leaf makes scope availability and
//! mutation revalidation explicit so an uncertain write can never leave stale catalog
//! state presented as authoritative.

use std::{
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use common::{
    MutationOutcome,
    theme::{PublicThemeSelection, Theme},
};

use common::ids::ThemeId;

use leptos::prelude::{
    Action, Get, GetUntracked, GetValue, MappedSignal, Memo, Owner, RwSignal, Set, StoredValue,
    Update, WithValue, on_cleanup,
};

/// An editor choice is not a persisted public selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    New,
    FollowSite,
    Bundled(Theme),
    Custom(ThemeId),
}

impl ThemeChoice {
    #[must_use]
    pub const fn custom_id(self) -> Option<ThemeId> {
        match self {
            Self::Custom(id) => Some(id),
            Self::New | Self::FollowSite | Self::Bundled(_) => None,
        }
    }

    #[must_use]
    pub const fn selection(self) -> Option<PublicThemeSelection> {
        match self {
            Self::New | Self::FollowSite => None,
            Self::Bundled(theme) => Some(PublicThemeSelection::BuiltIn(theme)),
            Self::Custom(id) => Some(PublicThemeSelection::Custom(id)),
        }
    }
}

/// The owner, installed choice and saved-state generation captured for one preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewTarget {
    pub scope: OwnershipScope,
    pub theme: PublicThemeSelection,
    pub generation: u32,
}

/// Inheritance is guidance, not an installed theme or a cross-owner preview read.
#[must_use]
pub fn preview_target(
    scope: OwnershipScope,
    choice: Option<ThemeChoice>,
    generation: u32,
) -> Option<PreviewTarget> {
    Some(PreviewTarget {
        scope,
        theme: choice?.selection()?,
        generation,
    })
}

/// Late reads may only supply output for the exact current owner/choice/save generation.
#[must_use]
pub fn preview_is_current(
    current: Option<PreviewTarget>,
    completed: Option<PreviewTarget>,
) -> bool {
    current.is_some() && current == completed
}

/// Only confirmed reads may mark a card current. The site resolver uses Studio
/// for an unset site selection; a failed read never implies that default.
#[must_use]
pub fn choice_is_current(
    choice: ThemeChoice,
    scope: OwnershipScope,
    selection: Option<&Result<Option<PublicThemeSelection>, WebError>>,
) -> bool {
    match selection {
        Some(Ok(None)) if scope == OwnershipScope::Site => {
            choice == ThemeChoice::Bundled(Theme::Studio)
        }
        Some(Ok(value)) => choice.selection() == *value,
        None | Some(Err(_)) => false,
    }
}

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

/// Presentation of theme feedback, independent of message wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feedback {
    /// A dispatched operation has not completed.
    Progress,
    /// The requested operation completed with confirmation.
    Success,
    /// Rejection, uncertainty or a local failure needs attention.
    Error,
}

/// One source-bound local normal form. Stored image values are never resubmitted as grants.
#[derive(Clone, Copy)]
pub struct EditorState {
    pub scope: OwnershipScope,
    pub target: RwSignal<Option<ThemeId>>,
    pub name: RwSignal<String>,
    pub stylesheet: RwSignal<String>,
    pub logo: RwSignal<super::ThemeEditorPatch<super::ThemeBindingInput>>,
    pub header: RwSignal<super::ThemeEditorPatch<super::ThemeBindingInput>>,
    pub generation: RwSignal<u64>,
    pub saved: RwSignal<super::ThemeEditorInput>,
    pub rotating: RwSignal<bool>,
    /// Preview state shares the cached form's lifetime, including rekey teardown.
    pub preview: Action<EditorSnapshot, EditorPreviewResult>,
    pub preview_pending: Memo<bool>,
    pub preview_value: MappedSignal<Option<EditorPreviewResult>>,
}

impl EditorState {
    #[must_use]
    pub fn new(
        scope: OwnershipScope,
        target: Option<ThemeId>,
        name: String,
        stylesheet: String,
        rotating: bool,
    ) -> Self {
        let patch = if target.is_some() {
            super::ThemeEditorPatch::Keep
        } else {
            super::ThemeEditorPatch::Set(super::ThemeBindingInput::PackagedDefault)
        };
        let saved = super::ThemeEditorInput {
            name: name.clone(),
            stylesheet: stylesheet.clone(),
            logo: patch.clone(),
            header: patch.clone(),
        };
        let preview = Action::new(|snapshot: &EditorSnapshot| {
            let snapshot = snapshot.clone();
            async move {
                let result = super::api::preview_editor(
                    snapshot.scope,
                    snapshot.target,
                    snapshot.input.clone(),
                )
                .await;
                (snapshot, result)
            }
        });
        Self {
            scope,
            target: RwSignal::new(target),
            name: RwSignal::new(name),
            stylesheet: RwSignal::new(stylesheet),
            logo: RwSignal::new(patch.clone()),
            header: RwSignal::new(patch),
            generation: RwSignal::new(0),
            saved: RwSignal::new(saved),
            rotating: RwSignal::new(rotating),
            preview_pending: preview.pending(),
            preview_value: preview.value(),
            preview,
        }
    }

    #[must_use]
    pub fn input(self) -> super::ThemeEditorInput {
        super::ThemeEditorInput {
            name: self.name.get(),
            stylesheet: self.stylesheet.get(),
            logo: self.logo.get(),
            header: self.header.get(),
        }
    }

    pub fn changed(self) {
        self.generation
            .update(|value| *value = value.wrapping_add(1));
    }

    #[must_use]
    pub fn snapshot(self) -> EditorSnapshot {
        EditorSnapshot {
            scope: self.scope,
            target: self.target.get_untracked(),
            input: self.input(),
            generation: self.generation.get_untracked(),
        }
    }

    #[must_use]
    pub fn matches(self, snapshot: &EditorSnapshot) -> bool {
        self.scope == snapshot.scope
            && self.target.get_untracked() == snapshot.target
            && self.generation.get_untracked() == snapshot.generation
            && self.input() == snapshot.input
    }

    /// Retain the confirmed identity, but only mark the exact acknowledged buffer clean.
    pub fn confirm(self, snapshot: &EditorSnapshot, saved: &super::ThemeEditorSaveResult) -> bool {
        let clean = self.matches(snapshot);
        if self.scope != snapshot.scope || self.target.get_untracked() != snapshot.target {
            return false;
        }
        self.target.set(Some(saved.id));
        if !clean {
            // A confirmed creation must not remain an unsaved target on the next Save.
            let mut acknowledged = snapshot.input.clone();
            acknowledged.name.clone_from(&saved.name);
            self.saved.set(acknowledged);
            return false;
        }
        self.name.set(saved.name.clone());
        if matches!(snapshot.input.header, super::ThemeEditorPatch::Set(_)) {
            self.rotating.set(false);
        }
        self.logo.set(super::ThemeEditorPatch::Keep);
        self.header.set(super::ThemeEditorPatch::Keep);
        self.saved.set(self.input());
        self.changed();
        true
    }
}

pub type EditorPreviewResult = (EditorSnapshot, Result<super::ThemePreview, WebError>);

/// All request fields are captured before the first await.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorSnapshot {
    pub scope: OwnershipScope,
    pub target: Option<ThemeId>,
    pub input: super::ThemeEditorInput,
    pub generation: u64,
}

const fn scope_index(scope: OwnershipScope) -> usize {
    match scope {
        OwnershipScope::Author => 0,
        OwnershipScope::Site => 1,
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
    pub selected: RwSignal<Option<ThemeChoice>>,
    /// Invalidates every owner-state read after a write acknowledgement.
    pub refresh: Invalidator,
    /// The accessible feedback message for the last write.
    pub status: RwSignal<Option<String>>,
    /// Places import feedback beside its picker rather than below Advanced authoring.
    pub feedback_in_import: RwSignal<bool>,
    /// Distinguishes confirmed completion from progress and failed or uncertain operations.
    pub feedback: RwSignal<Feedback>,
    /// Copy source selection never changes the editor or public selection.
    pub copy_open: RwSignal<bool>,
    pub copy_source: RwSignal<Option<PublicThemeSelection>>,
    /// Local buffers survive owner navigation and catalog revalidation.
    pub editors: RwSignal<Vec<EditorState>>,
    // Cached buffers belong to the page, not a refetchable Suspend or keyed editor view.
    editor_owner: StoredValue<Owner>,
    alive: StoredValue<Arc<AtomicBool>>,
    pub owner_choices: RwSignal<[Option<ThemeChoice>; 2]>,
}

impl Default for ThemePageState {
    fn default() -> Self {
        let alive = Arc::new(AtomicBool::new(true));
        let cleanup_alive = Arc::clone(&alive);
        on_cleanup(move || cleanup_alive.store(false, Ordering::Release));
        Self {
            pending: RwSignal::new(false),
            scope: RwSignal::new(OwnershipScope::Author),
            selected: RwSignal::new(None),
            refresh: Invalidator::new(),
            status: RwSignal::new(None),
            feedback_in_import: RwSignal::new(false),
            feedback: RwSignal::new(Feedback::Progress),
            copy_open: RwSignal::new(false),
            copy_source: RwSignal::new(None),
            editors: RwSignal::new(Vec::new()),
            editor_owner: StoredValue::new(Owner::new()),
            alive: StoredValue::new(alive),
            owner_choices: RwSignal::new([None, None]),
        }
    }
}

impl ThemePageState {
    /// Capture before awaiting; a detached callback must not touch disposed page handles.
    #[must_use]
    pub fn lifetime(self) -> Arc<AtomicBool> {
        self.alive.get_value()
    }

    /// Reopening a target preserves its local buffer instead of reseeding from a refresh.
    #[must_use]
    pub fn editor(
        self,
        target: Option<ThemeId>,
        name: String,
        stylesheet: String,
        rotating: bool,
    ) -> EditorState {
        let scope = self.scope.get_untracked();
        if let Some(editor) = self
            .editors
            .get_untracked()
            .into_iter()
            .find(|editor| editor.scope == scope && editor.target.get_untracked() == target)
        {
            // Saved presentation metadata changes independently of the staged normal fields.
            editor.rotating.set(rotating);
            return editor;
        }
        let editor = self.editor_owner.with_value(|owner| {
            owner.with(|| EditorState::new(scope, target, name, stylesheet, rotating))
        });
        self.editors.update(|editors| editors.push(editor));
        editor
    }

    /// Revalidate an explicit saved-context operation without adopting normal form values.
    pub fn context_revalidation<T>(
        self,
        scope: OwnershipScope,
        target: ThemeId,
        result: Result<MutationOutcome<T>, WebError>,
    ) -> Revalidation {
        let outcome = revalidation(result);
        if matches!(
            outcome,
            Revalidation::Confirmed | Revalidation::Indeterminate
        ) {
            for editor in self.editors.get_untracked() {
                if editor.scope == scope && editor.target.get_untracked() == Some(target) {
                    editor.changed();
                }
            }
        }
        outcome
    }

    #[must_use]
    pub fn editor_is_current(self, snapshot: &EditorSnapshot) -> bool {
        let choice = snapshot
            .target
            .map_or(ThemeChoice::New, ThemeChoice::Custom);
        self.scope.get_untracked() == snapshot.scope
            && self.selected.get_untracked() == Some(choice)
    }

    /// Opens the source chooser without retargeting the editor.
    pub fn open_copy(self) {
        if !self.pending.get_untracked() {
            self.copy_source.set(None);
            self.copy_open.set(true);
        }
    }

    pub fn choose_copy(self, source: PublicThemeSelection) {
        if self.copy_open.get_untracked() && !self.pending.get_untracked() {
            self.copy_source.set(Some(source));
        }
    }

    pub fn close_copy(self) {
        if !self.pending.get_untracked() {
            self.copy_open.set(false);
            self.copy_source.set(None);
        }
    }

    /// Captures one explicit same-library intent; uncertainty cannot replay it.
    #[must_use]
    pub fn begin_copy(self) -> Option<(OwnershipScope, PublicThemeSelection)> {
        if !self.copy_open.get_untracked() {
            return None;
        }
        let source = self.copy_source.get_untracked()?;
        self.begin_mutation()
            .then_some((self.scope.get_untracked(), source))
    }

    /// Opening a card only changes the editor; a write in flight holds its context.
    pub fn select_choice(self, choice: ThemeChoice) {
        if !self.pending.get_untracked() {
            self.selected.set(Some(choice));
        }
    }

    /// Changes catalog scope and drops state that cannot cross owner boundaries.
    pub fn select_scope(self, scope: OwnershipScope) {
        if self.pending.get_untracked() {
            return;
        }
        let previous = scope_index(self.scope.get_untracked());
        self.owner_choices
            .update(|choices| choices[previous] = self.selected.get_untracked());
        self.scope.set(scope);
        self.copy_open.set(false);
        self.copy_source.set(None);
        self.selected
            .set(self.owner_choices.get_untracked()[scope_index(scope)]);
        self.status.set(None);
        self.feedback_in_import.set(false);
        self.feedback.set(Feedback::Progress);
    }

    /// Serializes every theme operation in this page so draft writes cannot race activation.
    #[must_use]
    pub fn begin_mutation(self) -> bool {
        if self.pending.get_untracked() {
            return false;
        }
        self.pending.set(true);
        self.feedback_in_import.set(false);
        self.feedback.set(Feedback::Progress);
        true
    }

    /// Captures one custom draft target before dispatch; duplicate requests are refused.
    #[must_use]
    pub fn begin_use(self) -> Option<(OwnershipScope, ThemeId)> {
        if self.pending.get_untracked() {
            return None;
        }
        let id = self.selected.get_untracked()?.custom_id()?;
        self.begin_mutation()
            .then_some((self.scope.get_untracked(), id))
    }
}

/// Names only settings established by the existing owner reads. A missing custom
/// entry cannot supply a name, and an unset site setting is not an invented default.
#[must_use]
pub fn current_theme_summary(
    scope: OwnershipScope,
    selection: Option<&Result<Option<PublicThemeSelection>, WebError>>,
    catalog: Option<&Result<Vec<super::CatalogEntry>, WebError>>,
) -> String {
    match selection {
        None => "Checking current theme…".into(),
        Some(Err(_)) => "Current theme could not be checked".into(),
        Some(Ok(None)) => match scope {
            OwnershipScope::Author => "Using the site theme".into(),
            OwnershipScope::Site => "studio (site default)".into(),
        },
        Some(Ok(Some(PublicThemeSelection::BuiltIn(theme)))) => theme.to_string(),
        Some(Ok(Some(PublicThemeSelection::Custom(id)))) => match catalog {
            None => "Checking current theme…".into(),
            Some(Ok(entries)) => entries.iter().find(|entry| entry.id == *id).map_or_else(
                || "Current theme could not be checked".into(),
                |entry| entry.name.clone(),
            ),
            Some(Err(_)) => "Current theme could not be checked".into(),
        },
    }
}

/// Reviews a saved choice without confusing opening, preparing, and switching.
#[must_use]
pub fn review_choice(
    chosen: Option<ThemeId>,
    catalog: Option<&Result<Vec<super::CatalogEntry>, WebError>>,
    selection: Option<&Result<Option<PublicThemeSelection>, WebError>>,
) -> (String, &'static str) {
    let Some(id) = chosen else {
        return (
            "Choose a theme above".into(),
            "Uploading or choosing a theme does not change the current setting.",
        );
    };
    let entries = match catalog {
        Some(Ok(entries)) => entries,
        None => {
            return (
                "Checking your choice…".into(),
                "Your choice could not yet be checked.",
            );
        }
        Some(Err(_)) => {
            return (
                "Choice could not be checked".into(),
                "Current theme could not be checked. This choice may already be in use.",
            );
        }
    };
    let Some(entry) = entries.iter().find(|entry| entry.id == id) else {
        return (
            "Choice could not be checked".into(),
            "Choose an available theme before continuing.",
        );
    };
    let state = match selection {
        Some(Ok(Some(PublicThemeSelection::Custom(current)))) if *current == id => {
            "Currently in use. Getting this theme ready can update your public pages, even if switching later fails."
        }
        Some(Ok(_)) if !entry.published => "Saved, not live yet",
        Some(Ok(_)) => "Ready to use. Getting it ready includes any edits made in Advanced.",
        None | Some(Err(_)) => {
            "Current theme could not be checked. This choice may already be in use."
        }
    };
    (entry.name.clone(), state)
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
    Copied,
    Publish,
    PublishOnly,
    Apply { published: bool },
}

/// Publication within Use is progress until its separate selection stage completes.
#[must_use]
pub fn mutation_feedback(stage: MutationStage, receipt: &Receipt) -> Feedback {
    match (stage, receipt) {
        (MutationStage::Publish, Receipt::Confirmed) => Feedback::Progress,
        (_, Receipt::Confirmed) => Feedback::Success,
        (_, Receipt::Rejected(_) | Receipt::Uncertain(_)) => Feedback::Error,
    }
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
        (MutationStage::Copied, Receipt::Confirmed) => {
            "Theme copied privately. Choose its tile to Use it."
        }
        (MutationStage::Copied, Receipt::Rejected(_)) => {
            "Copy rejected. No destination was confirmed. Check that the saved theme and all its images are available to you."
        }
        (MutationStage::Copied, Receipt::Uncertain(_)) => {
            "The Copy outcome is uncertain. Check available themes before copying again; no automatic retry ran."
        }
        (MutationStage::Import, Receipt::Confirmed) => {
            "Theme saved privately. Choose its tile to Use it."
        }
        (MutationStage::Import, Receipt::Rejected(_)) => {
            "Upload rejected. Choose a valid theme ZIP file and check its manifest name."
        }
        (MutationStage::Import, Receipt::Uncertain(_)) => {
            "The upload outcome is uncertain. Check available themes before uploading again; no automatic retry ran."
        }
        (MutationStage::Publish, Receipt::Confirmed) => "Applying your theme…",
        (MutationStage::PublishOnly, Receipt::Confirmed) => {
            "Publication confirmed. No selection request was sent; an already-selected theme now uses the new revision."
        }
        (MutationStage::Publish | MutationStage::PublishOnly, Receipt::Rejected(_)) => {
            "Getting your theme ready was rejected. We did not try to switch themes. Fix the theme in Advanced before using it again."
        }
        (MutationStage::Publish | MutationStage::PublishOnly, Receipt::Uncertain(_)) => {
            "We could not confirm your theme is ready; the outcome is uncertain. We did not try to switch themes. If this theme was already in use, your public pages may have changed. Check the refreshed setting before trying again; no automatic retry ran."
        }
        (MutationStage::Apply { .. }, Receipt::Confirmed) => {
            "Your theme is now in use. Private management and Home are unchanged."
        }
        (MutationStage::Apply { published: true }, Receipt::Rejected(_)) => {
            "Your updated theme is ready, but switching was rejected. If it was already in use, the new version is already live; a failed switch cannot undo that update. Check the current theme before choosing another."
        }
        (MutationStage::Apply { published: false }, Receipt::Rejected(_)) => {
            "Switching was rejected. Check the current theme and choose another theme."
        }
        (MutationStage::Apply { published: true }, Receipt::Uncertain(_)) => {
            "Your updated theme is ready, but the switching outcome is uncertain. If it was already in use, the new version is already live; a failed switch cannot undo that update. Check the refreshed setting before trying again; no automatic retry ran."
        }
        (MutationStage::Apply { published: false }, Receipt::Uncertain(_)) => {
            "The switching outcome is uncertain. Check the refreshed setting before trying again; no automatic retry ran."
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
    scope: OwnershipScope,
    catalog: &Result<Vec<super::CatalogEntry>, WebError>,
    selection: &Result<Option<PublicThemeSelection>, WebError>,
    target: Option<ThemeId>,
) -> String {
    let catalog_message = match catalog {
        Ok(entries) => {
            let target_message = target
                .map(|id| match entries.iter().find(|entry| entry.id == id) {
                    Some(entry) if entry.published => "Your choice has a ready version.",
                    Some(_) => "Your choice has no ready version.",
                    None => "Your choice is no longer available; reopen Themes before continuing.",
                })
                .unwrap_or_default();
            format!(
                "Available themes checked: {}. {target_message}",
                entries.len()
            )
        }
        Err(error) => format!(
            "Available themes could not be checked: {error}. Theme readiness is unknown; reopen Themes to review it."
        ),
    };
    let selection_message = match selection {
        Ok(_) => format!(
            "Current theme: {}.",
            current_theme_summary(scope, Some(selection), Some(catalog))
        ),
        Err(error) => format!(
            "Current theme could not be checked: {error}. The current setting is unknown; reopen Themes to review it."
        ),
    };
    format!("{catalog_message} {selection_message}")
}

/// The Advanced selector contains published custom IDs only. Other current settings
/// select its real placeholder rather than assigning a value absent from its options.
#[must_use]
pub fn saved_version_control_token(
    selection: Option<&Result<Option<PublicThemeSelection>, WebError>>,
    catalog: Option<&Result<Vec<super::CatalogEntry>, WebError>>,
) -> String {
    match (selection, catalog) {
        (Some(Ok(Some(PublicThemeSelection::Custom(id)))), Some(Ok(entries)))
            if entries
                .iter()
                .any(|entry| entry.id == *id && entry.published) =>
        {
            id.to_string()
        }
        _ => String::new(),
    }
}

/// An unread or failed current setting has no token, never a fabricated inheritance.
/// The view names the state separately and permits recovery only after a successful read.
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
        Feedback, Revalidation, ScopeAvailability, ThemeChoice, ThemePageState, draft_from_editor,
        publication_confirmed, revalidation,
    };
    use crate::themes::OwnershipScope;
    use common::MutationOutcome;
    use leptos::prelude::{GetUntracked, Owner, Set};

    #[test]
    fn confirmed_new_save_retains_id_and_newer_edits_without_marking_them_clean() {
        let owner = Owner::new();
        owner.with(|| {
            let page = ThemePageState::default();
            page.select_choice(ThemeChoice::New);
            let editor = page.editor(None, "River".into(), "original".into(), false);
            let snapshot = editor.snapshot();
            editor.name.set("River revised".into());
            editor.stylesheet.set("newer".into());
            editor.changed();
            let id = common::ids::ThemeId::from(43);
            assert!(!editor.confirm(
                &snapshot,
                &crate::themes::ThemeEditorSaveResult {
                    id,
                    name: "River".into(),
                    applied: false,
                }
            ));
            assert_eq!(editor.target.get_untracked(), Some(id));
            assert_eq!(editor.name.get_untracked(), "River revised");
            assert_eq!(editor.stylesheet.get_untracked(), "newer");
            assert_eq!(editor.saved.get_untracked(), snapshot.input);
            assert_ne!(editor.input(), editor.saved.get_untracked());
            assert_eq!(editor.snapshot().target, Some(id));
        });
    }

    #[test]
    fn saved_context_revalidation_stales_only_affected_preview_and_preserves_buffers() {
        use crate::themes::{ThemeBindingInput, ThemeEditorPatch};
        Owner::new().with(|| {
            let page = ThemePageState::default();
            let id = common::ids::ThemeId::from(42);
            let other = common::ids::ThemeId::from(43);
            let editor = page.editor(Some(id), "Local".into(), "saved".into(), false);
            let other_editor = page.editor(Some(other), "Other".into(), "other".into(), false);
            page.select_scope(OwnershipScope::Site);
            let site_editor = page.editor(Some(id), "Site".into(), "site".into(), false);
            page.select_scope(OwnershipScope::Author);
            page.select_choice(ThemeChoice::Custom(id));
            editor.stylesheet.set("unsaved CSS".into());
            editor
                .logo
                .set(ThemeEditorPatch::Set(ThemeBindingInput::ExplicitAbsent));
            let input = editor.input();
            let baseline = editor.saved.get_untracked();
            let untouched = [other_editor.snapshot(), site_editor.snapshot()];
            for result in [
                Ok(MutationOutcome::Confirmed(())),
                Ok(MutationOutcome::CommitIndeterminate(())),
            ] {
                let preview = editor.snapshot();
                page.context_revalidation(OwnershipScope::Author, id, result);
                assert!(
                    !editor.matches(&preview),
                    "saved context must invalidate the old preview"
                );
                assert_eq!(editor.input(), input);
                assert_eq!(editor.saved.get_untracked(), baseline);
                assert_eq!(page.selected.get_untracked(), Some(ThemeChoice::Custom(id)));
                assert!(other_editor.matches(&untouched[0]));
                assert!(site_editor.matches(&untouched[1]));
            }
            let preview = editor.snapshot();
            page.context_revalidation::<()>(
                OwnershipScope::Author,
                id,
                Err(crate::error::WebError::server_message("Rejected")),
            );
            assert!(editor.matches(&preview));
            let reopened = page.editor(Some(id), "server name".into(), "server CSS".into(), true);
            assert!(reopened.rotating.get_untracked());
            assert_eq!(reopened.input(), input);
            assert_eq!(reopened.saved.get_untracked(), baseline);
        });
    }

    #[test]
    fn cached_preview_survives_new_target_rekey_and_page_cleanup_fences_save() {
        let owner = Owner::new();
        let (page, editor, lifetime) = owner.with(|| {
            let page = ThemePageState::default();
            page.select_choice(ThemeChoice::New);
            let loading_owner = Owner::new();
            let editor =
                loading_owner.with(|| page.editor(None, "River".into(), "style".into(), false));
            let preview = editor.preview;
            let snapshot = editor.snapshot();
            assert!(editor.confirm(
                &snapshot,
                &crate::themes::ThemeEditorSaveResult {
                    id: common::ids::ThemeId::from(43),
                    name: "River".into(),
                    applied: false,
                }
            ));
            page.selected
                .set(Some(ThemeChoice::Custom(common::ids::ThemeId::from(43))));
            // The removed form's pending/value readers can run at the rekey boundary.
            loading_owner.cleanup();
            assert!(!editor.preview_pending.get_untracked());
            assert!(editor.preview_value.get_untracked().is_none());
            let reopened = page.editor(
                Some(common::ids::ThemeId::from(43)),
                "ignored".into(),
                String::new(),
                false,
            );
            assert_eq!(reopened.preview_pending, editor.preview_pending);
            assert!(!preview.pending().get_untracked());
            assert!(!page.editor_is_current(&snapshot));
            let lifetime = page.lifetime();
            assert!(lifetime.load(std::sync::atomic::Ordering::Acquire));
            (page, editor, lifetime)
        });
        owner.cleanup();
        assert!(!lifetime.load(std::sync::atomic::Ordering::Acquire));
        assert!(page.pending.try_get_untracked().is_none());
        assert!(editor.target.try_get_untracked().is_none());
    }

    #[test]
    fn cached_editor_survives_loading_owner_cleanup_and_reopens_after_save() {
        let owner = Owner::new();
        owner.with(|| {
            let page = ThemePageState::default();
            let loading_owner = Owner::new();
            let id = common::ids::ThemeId::from(42);
            let editor =
                loading_owner.with(|| page.editor(Some(id), "Ocean".into(), "style".into(), false));
            loading_owner.cleanup();
            assert_eq!(editor.target.try_get_untracked(), Some(Some(id)));
            page.select_choice(ThemeChoice::Custom(id));
            let snapshot = editor.snapshot();
            assert!(editor.confirm(
                &snapshot,
                &crate::themes::ThemeEditorSaveResult {
                    id,
                    name: "Ocean".into(),
                    applied: true,
                }
            ));
            assert_eq!(
                page.editor(Some(id), "ignored".into(), String::new(), false)
                    .input(),
                editor.input()
            );
            page.select_choice(ThemeChoice::New);
            let new = page.editor(None, String::new(), String::new(), false);
            assert_eq!(new.target.get_untracked(), None);
            assert!(new.name.get_untracked().is_empty());
        });
    }

    #[test]
    fn copy_source_and_cancel_preserve_editor_and_capture_one_owner_intent() {
        use common::theme::{PublicThemeSelection, Theme};
        Owner::new().with(|| {
            let page = ThemePageState::default();
            let editor = ThemeChoice::Custom(common::ids::ThemeId::from(42));
            let source = PublicThemeSelection::BuiltIn(Theme::Reader);
            page.select_choice(editor);
            assert_eq!(page.begin_copy(), None);
            page.open_copy();
            assert_eq!(page.begin_copy(), None);
            page.choose_copy(source);
            assert_eq!(page.selected.get_untracked(), Some(editor));
            page.close_copy();
            assert_eq!(page.begin_copy(), None);
            assert_eq!(page.selected.get_untracked(), Some(editor));
            page.open_copy();
            page.choose_copy(source);
            assert_eq!(page.begin_copy(), Some((OwnershipScope::Author, source)));
            assert_eq!(page.begin_copy(), None);
            page.select_scope(OwnershipScope::Site);
            page.close_copy();
            page.choose_copy(PublicThemeSelection::Custom(common::ids::ThemeId::from(7)));
            assert_eq!(page.scope.get_untracked(), OwnershipScope::Author);
            assert_eq!(page.copy_source.get_untracked(), Some(source));
            page.pending.set(false);
            page.select_scope(OwnershipScope::Site);
            assert!(!page.copy_open.get_untracked());
            assert_eq!(page.copy_source.get_untracked(), None);
            page.open_copy();
            page.choose_copy(source);
            assert_eq!(page.begin_copy(), Some((OwnershipScope::Site, source)));
        });
    }

    #[test]
    fn preview_results_cannot_cross_owner_choice_or_saved_state() {
        use super::{preview_is_current, preview_target};
        use common::{ids::ThemeId, theme::Theme};
        let choice = Some(ThemeChoice::Bundled(Theme::Studio));
        let author = preview_target(OwnershipScope::Author, choice, 1);
        assert!(preview_is_current(author, author));
        assert!(!preview_is_current(
            preview_target(OwnershipScope::Site, choice, 1),
            author
        ));
        assert!(!preview_is_current(
            preview_target(
                OwnershipScope::Author,
                Some(ThemeChoice::Bundled(Theme::Reader)),
                1
            ),
            author
        ));
        assert!(!preview_is_current(
            preview_target(
                OwnershipScope::Author,
                Some(ThemeChoice::Custom(ThemeId::from(42))),
                1
            ),
            author
        ));
        assert!(!preview_is_current(
            preview_target(OwnershipScope::Author, choice, 2),
            author
        ));
        assert!(!preview_is_current(author, None));
        assert!(!preview_is_current(None, author));
        assert!(!preview_is_current(None, None));
        assert_eq!(preview_target(OwnershipScope::Author, None, 0), None);
        assert_eq!(
            preview_target(OwnershipScope::Author, Some(ThemeChoice::FollowSite), 0),
            None
        );
    }

    #[test]
    fn grid_current_and_editing_are_independent_and_unknown_never_highlights() {
        use super::{ThemeChoice, choice_is_current};
        use crate::error::WebError;
        use common::{
            ids::ThemeId,
            theme::{PublicThemeSelection, Theme},
        };
        let custom = ThemeChoice::Custom(ThemeId::from(42));
        assert!(!choice_is_current(
            custom,
            OwnershipScope::Author,
            Some(&Ok(None))
        ));
        assert!(choice_is_current(
            ThemeChoice::FollowSite,
            OwnershipScope::Author,
            Some(&Ok(None))
        ));
        assert!(choice_is_current(
            ThemeChoice::Bundled(Theme::Studio),
            OwnershipScope::Site,
            Some(&Ok(None))
        ));
        assert!(!choice_is_current(
            ThemeChoice::Bundled(Theme::Studio),
            OwnershipScope::Site,
            None
        ));
        assert!(choice_is_current(
            ThemeChoice::Bundled(Theme::Reader),
            OwnershipScope::Author,
            Some(&Ok(Some(PublicThemeSelection::BuiltIn(Theme::Reader))))
        ));
        assert_eq!(ThemeChoice::FollowSite.custom_id(), None);
        assert_eq!(ThemeChoice::Bundled(Theme::Reader).custom_id(), None);
        assert!(!choice_is_current(
            ThemeChoice::FollowSite,
            OwnershipScope::Author,
            Some(&Err(WebError::server_function("read failed")))
        ));
        assert!(choice_is_current(
            custom,
            OwnershipScope::Author,
            Some(&Ok(Some(PublicThemeSelection::Custom(ThemeId::from(42)))))
        ));
        let owner = Owner::new();
        owner.with(|| {
            let page = ThemePageState::default();
            page.select_choice(custom);
            assert_eq!(page.selected.get_untracked(), Some(custom));
            assert!(page.begin_mutation());
            page.select_choice(ThemeChoice::Bundled(Theme::Reader));
            page.select_scope(OwnershipScope::Site);
            assert_eq!(page.selected.get_untracked(), Some(custom));
            assert_eq!(page.scope.get_untracked(), OwnershipScope::Author);
            page.pending.set(false);
            page.select_choice(ThemeChoice::Bundled(Theme::Reader));
            assert_eq!(page.begin_use(), None);
            page.select_scope(OwnershipScope::Site);
            assert_eq!(page.selected.get_untracked(), None);
        });
    }

    #[test]
    fn saved_version_control_selects_only_an_available_published_custom_id() {
        use super::saved_version_control_token;
        use crate::{error::WebError, themes::CatalogEntry};
        use common::{
            ids::ThemeId,
            theme::{PublicThemeSelection, Theme},
        };
        let id = ThemeId::from(42);
        let catalog = Ok(vec![CatalogEntry {
            id,
            name: "Paper".into(),
            published: true,
        }]);
        assert_eq!(saved_version_control_token(None, Some(&catalog)), "");
        assert_eq!(
            saved_version_control_token(
                Some(&Err(WebError::server_function("read failed"))),
                Some(&catalog)
            ),
            ""
        );
        assert_eq!(
            saved_version_control_token(Some(&Ok(None)), Some(&catalog)),
            ""
        );
        assert_eq!(
            saved_version_control_token(
                Some(&Ok(Some(PublicThemeSelection::BuiltIn(Theme::Reader)))),
                Some(&catalog)
            ),
            ""
        );
        let custom = Ok(Some(PublicThemeSelection::Custom(id)));
        assert_eq!(
            saved_version_control_token(Some(&custom), Some(&catalog)),
            "42"
        );
        assert_eq!(saved_version_control_token(Some(&custom), None), "");
        assert_eq!(
            saved_version_control_token(
                Some(&custom),
                Some(&Err(WebError::server_function("list failed")))
            ),
            ""
        );
        assert_eq!(
            saved_version_control_token(Some(&custom), Some(&Ok(Vec::new()))),
            ""
        );
        let unpublished = Ok(vec![CatalogEntry {
            id,
            name: "Paper".into(),
            published: false,
        }]);
        assert_eq!(
            saved_version_control_token(Some(&custom), Some(&unpublished)),
            ""
        );
    }

    #[test]
    fn current_theme_summary_reports_only_confirmed_owner_state() {
        use super::current_theme_summary;
        use crate::{error::WebError, themes::CatalogEntry};
        use common::{
            ids::ThemeId,
            theme::{PublicThemeSelection, Theme},
        };
        let entries = Ok(vec![CatalogEntry {
            id: ThemeId::from(42),
            name: "Paper".into(),
            published: true,
        }]);
        let inherited = Ok(None);
        assert_eq!(
            current_theme_summary(OwnershipScope::Author, None, None),
            "Checking current theme…"
        );
        assert_eq!(
            current_theme_summary(OwnershipScope::Author, Some(&inherited), None),
            "Using the site theme"
        );
        assert_eq!(
            current_theme_summary(OwnershipScope::Site, Some(&inherited), None),
            "studio (site default)"
        );
        assert_eq!(
            current_theme_summary(
                OwnershipScope::Author,
                Some(&Ok(Some(PublicThemeSelection::BuiltIn(Theme::Reader)))),
                None
            ),
            "reader"
        );
        let custom = Ok(Some(PublicThemeSelection::Custom(ThemeId::from(42))));
        assert_eq!(
            current_theme_summary(OwnershipScope::Site, Some(&custom), Some(&entries)),
            "Paper"
        );
        assert_eq!(
            current_theme_summary(OwnershipScope::Site, Some(&custom), None),
            "Checking current theme…"
        );
        assert_eq!(
            current_theme_summary(OwnershipScope::Site, Some(&custom), Some(&Ok(Vec::new()))),
            "Current theme could not be checked"
        );
        let failed = Err(WebError::server_function("read failed"));
        assert_eq!(
            current_theme_summary(OwnershipScope::Author, Some(&failed), Some(&entries)),
            "Current theme could not be checked"
        );
        assert_eq!(
            current_theme_summary(
                OwnershipScope::Site,
                Some(&custom),
                Some(&Err(WebError::server_function("list failed")))
            ),
            "Current theme could not be checked"
        );
    }

    #[test]
    fn reviewing_a_choice_distinguishes_unused_current_ready_and_unknown() {
        use super::review_choice;
        use crate::{error::WebError, themes::CatalogEntry};
        use common::{ids::ThemeId, theme::PublicThemeSelection};
        let id = ThemeId::from(7);
        let mut entries = Ok(vec![CatalogEntry {
            id,
            name: "Paper".into(),
            published: false,
        }]);
        assert_eq!(review_choice(None, None, None).0, "Choose a theme above");
        assert_eq!(
            review_choice(Some(id), None, None).0,
            "Checking your choice…"
        );
        assert_eq!(
            review_choice(
                Some(id),
                Some(&Err(WebError::server_function("list failed"))),
                None
            ),
            (
                "Choice could not be checked".into(),
                "Current theme could not be checked. This choice may already be in use."
            )
        );
        assert_eq!(
            review_choice(Some(id), Some(&Ok(Vec::new())), None).0,
            "Choice could not be checked"
        );
        assert_eq!(
            review_choice(Some(id), Some(&entries), Some(&Ok(None))),
            ("Paper".into(), "Saved, not live yet")
        );
        let current = Ok(Some(PublicThemeSelection::Custom(id)));
        assert!(
            review_choice(Some(id), Some(&entries), Some(&current))
                .1
                .starts_with("Currently in use")
        );
        assert!(
            review_choice(Some(id), Some(&entries), None)
                .1
                .contains("may already be in use")
        );
        assert!(
            review_choice(
                Some(id),
                Some(&entries),
                Some(&Err(WebError::server_function("read failed")))
            )
            .1
            .contains("may already be in use")
        );
        entries.as_mut().unwrap()[0].published = true;
        assert!(
            review_choice(Some(id), Some(&entries), Some(&Ok(None)))
                .1
                .starts_with("Ready to use")
        );
        assert!(
            !review_choice(Some(id), Some(&entries), Some(&current))
                .1
                .contains("not live")
        );
    }

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
            page.select_choice(ThemeChoice::Custom(common::ids::ThemeId::from(42)));
            page.status.set(Some("saved".into()));
            page.feedback_in_import.set(true);
            page.feedback.set(Feedback::Success);

            page.select_scope(OwnershipScope::Site);

            assert_eq!(page.scope.get_untracked(), OwnershipScope::Site);
            assert_eq!(page.selected.get_untracked(), None);
            assert_eq!(page.status.get_untracked(), None);
            assert!(!page.feedback_in_import.get_untracked());
            assert_eq!(page.feedback.get_untracked(), Feedback::Progress);
        });
    }

    #[test]
    fn pending_use_keeps_its_catalog_scope_and_draft_target() {
        Owner::new().with(|| {
            let page = ThemePageState::default();
            page.select_choice(ThemeChoice::Custom(common::ids::ThemeId::from(42)));
            page.pending.set(true);
            page.feedback_in_import.set(true);
            page.feedback.set(Feedback::Error);
            page.select_scope(OwnershipScope::Site);
            assert_eq!(page.scope.get_untracked(), OwnershipScope::Author);
            assert_eq!(
                page.selected.get_untracked(),
                Some(ThemeChoice::Custom(common::ids::ThemeId::from(42)))
            );
            assert!(page.feedback_in_import.get_untracked());
            assert_eq!(page.feedback.get_untracked(), Feedback::Error);
        });
    }

    #[test]
    fn feedback_styles_only_confirmed_completion_as_success() {
        use super::{MutationStage, Receipt, mutation_feedback};
        for (stage, expected) in [
            (MutationStage::Import, Feedback::Success),
            (MutationStage::Copied, Feedback::Success),
            (MutationStage::Publish, Feedback::Progress),
            (MutationStage::PublishOnly, Feedback::Success),
            (MutationStage::Apply { published: false }, Feedback::Success),
            (MutationStage::Apply { published: true }, Feedback::Success),
        ] {
            assert_eq!(mutation_feedback(stage, &Receipt::Confirmed), expected);
            for receipt in [
                Receipt::Rejected("rejected".into()),
                Receipt::Uncertain("unknown".into()),
            ] {
                assert_eq!(mutation_feedback(stage, &receipt), Feedback::Error);
            }
        }
        Owner::new().with(|| {
            let page = ThemePageState::default();
            assert_eq!(page.feedback.get_untracked(), Feedback::Progress);
        });
    }

    #[test]
    fn use_captures_one_target_and_refuses_duplicate_dispatch() {
        Owner::new().with(|| {
            let page = ThemePageState::default();
            assert_eq!(page.begin_use(), None);
            assert!(!page.pending.get_untracked());
            page.select_choice(ThemeChoice::Custom(common::ids::ThemeId::from(42)));
            assert_eq!(
                page.begin_use(),
                Some((OwnershipScope::Author, common::ids::ThemeId::from(42)))
            );
            assert_eq!(page.begin_use(), None);
            page.pending.set(false);
            page.select_scope(OwnershipScope::Site);
            page.select_choice(ThemeChoice::Custom(common::ids::ThemeId::from(7)));
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
            page.select_choice(ThemeChoice::Custom(common::ids::ThemeId::from(42)));
            page.feedback.set(Feedback::Success);
            page.feedback_in_import.set(true);
            assert!(page.begin_mutation());
            assert_eq!(page.feedback.get_untracked(), Feedback::Progress);
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
            (MutationStage::Import, "Theme saved"),
            (MutationStage::Copied, "Theme copied privately"),
            (MutationStage::Publish, "Applying your theme"),
            (MutationStage::PublishOnly, "Publication confirmed"),
            (
                MutationStage::Apply { published: true },
                "Your theme is now in use",
            ),
            (
                MutationStage::Apply { published: false },
                "Your theme is now in use",
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
            assert!(!uncertain.contains("Your theme is now in use"));
        }
        let rejected = mutation_message(MutationStage::Publish, &Receipt::Rejected("CSS".into()));
        assert!(
            rejected.contains("We did not try to switch themes")
                && rejected.contains("Fix the theme")
        );
        assert!(!rejected.contains("may have changed"));
        let applied = mutation_message(
            MutationStage::Apply { published: true },
            &Receipt::Rejected("selection".into()),
        );
        assert!(
            applied.contains("Your updated theme is ready")
                && applied.contains("cannot undo")
                && applied.contains("already in use")
        );
    }

    #[test]
    fn rereads_report_current_publication_and_selection_without_resolving_uncertain_write() {
        use super::reconciliation_message;
        use crate::{error::WebError, themes::CatalogEntry};
        use common::theme::{PublicThemeSelection, Theme};
        let id = common::ids::ThemeId::from(42);
        for (published, expected) in [
            (true, "has a ready version"),
            (false, "has no ready version"),
        ] {
            let catalog = Ok(vec![CatalogEntry {
                id,
                name: "Night".into(),
                published,
            }]);
            let message = reconciliation_message(
                OwnershipScope::Author,
                &catalog,
                &Ok(Some(PublicThemeSelection::Custom(id))),
                Some(id),
            );
            assert!(
                message.contains(expected) && message.contains("Current theme: Night."),
                "{message}"
            );
            assert!(!message.contains("confirmed"));
        }
        let missing =
            reconciliation_message(OwnershipScope::Author, &Ok(Vec::new()), &Ok(None), Some(id));
        assert!(
            missing.contains("no longer available") && missing.contains("Using the site theme")
        );
        let bundled = reconciliation_message(
            OwnershipScope::Author,
            &Ok(Vec::new()),
            &Ok(Some(PublicThemeSelection::BuiltIn(Theme::Reader))),
            None,
        );
        assert!(bundled.contains("reader"));
        let failure = Err(WebError::server_function("reread unavailable"));
        let both = reconciliation_message(
            OwnershipScope::Author,
            &failure,
            &Err(WebError::server_function("read failed")),
            Some(id),
        );
        assert!(
            both.contains("Theme readiness is unknown")
                && both.contains("current setting is unknown")
        );
        assert!(!both.contains("inherit"));
        assert!(
            reconciliation_message(OwnershipScope::Author, &failure, &Ok(None), None)
                .contains("Current theme: Using the site theme")
        );
        assert!(
            reconciliation_message(
                OwnershipScope::Author,
                &Ok(Vec::new()),
                &Err(WebError::server_function("read failed")),
                None
            )
            .contains("Current theme could not be checked")
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
