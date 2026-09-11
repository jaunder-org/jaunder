//! Host-tested form and disclosure transitions for audience management.

use crate::forms;
use common::MutationOutcome;
use common::audience::AudienceName;
use leptos::prelude::{RwSignal, Set, Update};

/// Whether a delete result proves the row can be removed from the audience list.
///
/// An indeterminate result keeps its disclosure mounted so the recovery feedback remains
/// visible; only an acknowledged commit triggers list revalidation.
#[must_use]
pub fn delete_invalidates_list(outcome: &MutationOutcome<()>) -> bool {
    matches!(outcome, MutationOutcome::Confirmed(()))
}

/// State for the create-audience draft.
#[derive(Clone, Copy, Default)]
pub struct CreateDraftState {
    name: forms::Field<AudienceName>,
    submitted_name: RwSignal<Option<AudienceName>>,
}

impl CreateDraftState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: forms::Field::new(),
            submitted_name: RwSignal::new(None),
        }
    }

    #[must_use]
    pub fn name(self) -> forms::Field<AudienceName> {
        self.name
    }

    pub fn record_create(self, name: &AudienceName) {
        self.submitted_name.set(Some(name.clone()));
    }

    pub fn settle<T, E>(self, outcome: Option<&Result<MutationOutcome<T>, E>>) {
        if !matches!(outcome, Some(Ok(MutationOutcome::Confirmed(_)))) {
            return;
        }
        let submitted = self.submitted_name.try_update(Option::take).flatten();
        if submitted
            .as_ref()
            .is_some_and(|submitted| self.name.value() == submitted.as_ref())
        {
            self.name.reset();
        }
    }
}

/// State transitions for one audience's edit disclosure.
#[derive(Clone, Copy)]
pub struct AudienceEditorState {
    editing: RwSignal<bool>,
    name: forms::Field<AudienceName>,
    submitted_name: RwSignal<Option<AudienceName>>,
}

impl AudienceEditorState {
    #[must_use]
    pub fn new(current: &AudienceName) -> Self {
        Self {
            editing: RwSignal::new(false),
            name: forms::Field::prefilled(current),
            submitted_name: RwSignal::new(None),
        }
    }

    #[must_use]
    pub fn editing(self) -> RwSignal<bool> {
        self.editing
    }

    #[must_use]
    pub fn name(self) -> forms::Field<AudienceName> {
        self.name
    }

    pub fn open(self, current: &AudienceName) {
        self.restore(current);
        self.editing.set(true);
    }

    pub fn cancel(self, current: &AudienceName) {
        self.restore(current);
        self.editing.set(false);
    }

    pub fn record_rename(self, name: &AudienceName) {
        self.submitted_name.set(Some(name.clone()));
    }

    pub fn settle_rename<E>(
        self,
        outcome: Option<&Result<MutationOutcome<()>, E>>,
    ) -> Option<AudienceName> {
        if !matches!(outcome, Some(Ok(MutationOutcome::Confirmed(())))) {
            return None;
        }
        self.editing.set(false);
        self.submitted_name.try_update(Option::take).flatten()
    }

    fn restore(self, current: &AudienceName) {
        self.name.reset();
        self.name.set_value(current.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::ids::AudienceId;
    use common::test_support::parse_audience_name;
    use leptos::prelude::{GetUntracked, Owner};

    #[test]
    fn only_confirmed_delete_invalidates_the_list() {
        assert!(delete_invalidates_list(&MutationOutcome::Confirmed(())));
        assert!(!delete_invalidates_list(
            &MutationOutcome::CommitIndeterminate(())
        ));
    }

    #[test]
    fn create_draft_resets_only_after_confirmed_creation() {
        let owner = Owner::new();
        owner.set();
        let state = CreateDraftState::new();
        state.name().set_value("Friends");
        state.name().touch();
        state.record_create(&parse_audience_name("Friends"));

        state.settle::<AudienceId, &str>(Some(&Err("failed")));
        assert_eq!(state.name().value(), "Friends");
        assert!(state.name().is_touched());

        state.record_create(&parse_audience_name("Friends"));
        state.settle::<AudienceId, &str>(Some(&Ok(MutationOutcome::CommitIndeterminate(
            AudienceId::from(1),
        ))));
        assert_eq!(state.name().value(), "Friends");

        state.record_create(&parse_audience_name("Friends"));
        state.name().set_value("Family");
        state
            .settle::<AudienceId, &str>(Some(&Ok(MutationOutcome::Confirmed(AudienceId::from(1)))));
        assert_eq!(state.name().value(), "Family");

        state.record_create(&parse_audience_name("Family"));
        state
            .settle::<AudienceId, &str>(Some(&Ok(MutationOutcome::Confirmed(AudienceId::from(2)))));
        assert_eq!(state.name().value(), "");
        assert!(!state.name().is_touched());
    }

    #[test]
    fn editor_transitions_preserve_or_consume_the_submitted_draft() {
        let owner = Owner::new();
        owner.set();
        let friends = parse_audience_name("Friends");
        let state = AudienceEditorState::new(&friends);
        assert!(!state.editing().get_untracked());

        state.open(&friends);
        state.name().set_value("Discarded");
        state.cancel(&friends);
        assert!(!state.editing().get_untracked());
        assert_eq!(state.name().value(), "Friends");

        state.open(&friends);
        state.name().set_value("BestFriends");
        state.record_rename(&parse_audience_name("BestFriends"));
        assert_eq!(state.settle_rename::<&str>(Some(&Err("failed"))), None);
        assert!(state.editing().get_untracked());
        assert_eq!(state.name().value(), "BestFriends");

        assert_eq!(
            state.settle_rename::<&str>(Some(&Ok(MutationOutcome::CommitIndeterminate(())))),
            None
        );
        assert!(state.editing().get_untracked());

        assert_eq!(
            state.settle_rename::<&str>(Some(&Ok(MutationOutcome::Confirmed(())))),
            Some(parse_audience_name("BestFriends"))
        );
        assert!(!state.editing().get_untracked());
    }
}
