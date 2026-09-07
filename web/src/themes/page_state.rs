//! Host-compiled state folds for the private theme-management page.
//!
//! The component owns asynchronous resources; this leaf makes scope availability and
//! mutation revalidation explicit so an uncertain write can never leave stale catalog
//! state presented as authoritative.

use common::MutationOutcome;

use common::ids::ThemeId;
use leptos::prelude::{RwSignal, Set};

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
        self.scope.set(scope);
        self.selected.set(None);
        self.status.set(None);
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
    use super::{Revalidation, ScopeAvailability, revalidation};
    use crate::themes::OwnershipScope;
    use common::MutationOutcome;

    #[test]
    fn operator_can_choose_the_programmatically_distinct_site_scope() {
        assert!(ScopeAvailability::from_session(true, true).permits(OwnershipScope::Site));
        assert!(!ScopeAvailability::from_session(true, false).permits(OwnershipScope::Site));
    }

    #[test]
    fn indeterminate_write_requires_a_reread() {
        assert_eq!(
            revalidation::<(), String>(Ok(MutationOutcome::CommitIndeterminate(()))),
            Revalidation::Indeterminate
        );
    }
}
