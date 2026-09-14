//! Host-compiled folds for the private passkey-management page.

/// Availability of the passkey feature as resolved by the deployment and browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Loading,
    Available,
    Unsupported,
    Failed,
}

/// Browser ceremony result expressed without browser types, so host tests can
/// exercise the same UI decision table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CeremonyResult {
    Success,
    Cancelled,
    Unsupported,
    Failed,
}

/// Browser ceremony result reduced to user-safe presentation state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CeremonyState {
    Cancelled,
    Unsupported,
    Failed,
    Succeeded,
}

/// Credential-list presentation state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialListState {
    Loading,
    Empty,
    Ready,
    Failed,
}

/// Feedback after a registration attempt, reduced without browser or server types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistrationState {
    Added,
    MayHaveAdded,
    CouldNotSave,
    Invalid,
    Ceremony(CeremonyState),
    Indeterminate,
}

/// Feedback after deleting a credential, reduced without server types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeletionState {
    Deleted,
    MayHaveDeleted,
}

/// The registration feedback text and whether the credential resource must reload.
#[must_use]
pub fn registration_feedback(state: RegistrationState) -> (&'static str, bool) {
    match state {
        RegistrationState::Added => ("Passkey added.", true),
        RegistrationState::MayHaveAdded => (
            "The passkey may have been added. The credential list was refreshed; review it before continuing.",
            true,
        ),
        RegistrationState::CouldNotSave => ("The passkey could not be saved.", false),
        RegistrationState::Invalid => ("The passkey registration was invalid. Try again.", false),
        RegistrationState::Ceremony(CeremonyState::Cancelled) => {
            ("Passkey request cancelled.", false)
        }
        RegistrationState::Ceremony(CeremonyState::Unsupported) => {
            ("Passkeys are not supported by this browser.", false)
        }
        RegistrationState::Ceremony(CeremonyState::Failed) => {
            ("The passkey request failed. Try again.", false)
        }
        RegistrationState::Ceremony(CeremonyState::Succeeded) => {
            unreachable!("completed ceremony outcome")
        }
        RegistrationState::Indeterminate => (
            "The registration could not be confirmed. Try again after checking your passkeys.",
            false,
        ),
    }
}

/// The deletion feedback text and whether the credential resource must reload.
#[must_use]
pub const fn deletion_feedback(state: DeletionState) -> (&'static str, bool) {
    match state {
        DeletionState::Deleted => ("Passkey removed.", true),
        DeletionState::MayHaveDeleted => (
            "The passkey may have been removed. The credential list was refreshed; review it before continuing.",
            true,
        ),
    }
}

/// Maps independent server/browser checks into a page state without masking errors.
#[must_use]
pub fn availability<E>(
    server_available: Option<&Result<bool, E>>,
    browser_supported: bool,
) -> Availability {
    match server_available {
        None => Availability::Loading,
        Some(Ok(true)) if browser_supported => Availability::Available,
        Some(Ok(_)) => Availability::Unsupported,
        Some(Err(_)) => Availability::Failed,
    }
}

/// Classifies a completed browser ceremony without exposing browser exception details.
#[must_use]
pub const fn ceremony_state(outcome: CeremonyResult) -> CeremonyState {
    match outcome {
        CeremonyResult::Success => CeremonyState::Succeeded,
        CeremonyResult::Cancelled => CeremonyState::Cancelled,
        CeremonyResult::Unsupported => CeremonyState::Unsupported,
        CeremonyResult::Failed => CeremonyState::Failed,
    }
}

/// Maps the credential resource to its visible state.
#[must_use]
pub const fn credential_list_state<T, E>(
    result: Option<&Result<Vec<T>, E>>,
) -> CredentialListState {
    match result {
        None => CredentialListState::Loading,
        Some(Ok(credentials)) if credentials.is_empty() => CredentialListState::Empty,
        Some(Ok(_)) => CredentialListState::Ready,
        Some(Err(_)) => CredentialListState::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Availability, CeremonyResult, CeremonyState, CredentialListState, DeletionState,
        RegistrationState, availability, ceremony_state, credential_list_state, deletion_feedback,
        registration_feedback,
    };

    #[test]
    fn availability_waits_for_the_server_check_and_keeps_its_failure_distinct() {
        assert_eq!(availability::<()>(None, true), Availability::Loading);
        assert_eq!(
            availability(Some(&Ok::<bool, ()>(true)), true),
            Availability::Available
        );
        assert_eq!(
            availability(Some(&Ok::<bool, ()>(false)), true),
            Availability::Unsupported
        );
        assert_eq!(
            availability(Some(&Ok::<bool, ()>(true)), false),
            Availability::Unsupported
        );
        assert_eq!(availability(Some(&Err(())), true), Availability::Failed);
    }

    #[test]
    fn ceremony_outcomes_are_bounded_to_safe_presentation_states() {
        assert_eq!(
            ceremony_state(CeremonyResult::Success),
            CeremonyState::Succeeded
        );
        assert_eq!(
            ceremony_state(CeremonyResult::Cancelled),
            CeremonyState::Cancelled
        );
        assert_eq!(
            ceremony_state(CeremonyResult::Unsupported),
            CeremonyState::Unsupported
        );
        assert_eq!(
            ceremony_state(CeremonyResult::Failed),
            CeremonyState::Failed
        );
    }

    #[test]
    fn credential_list_fold_covers_loading_empty_ready_and_failed_states() {
        assert_eq!(
            credential_list_state::<(), ()>(None),
            CredentialListState::Loading
        );
        assert_eq!(
            credential_list_state(Some(&Ok::<Vec<()>, ()>(Vec::new()))),
            CredentialListState::Empty
        );
        assert_eq!(
            credential_list_state(Some(&Ok::<Vec<()>, ()>(vec![()]))),
            CredentialListState::Ready
        );
        assert_eq!(
            credential_list_state(Some(&Err::<Vec<()>, ()>(()))),
            CredentialListState::Failed
        );
    }

    #[test]
    fn registration_feedback_preserves_confirmed_indeterminate_and_ceremony_decisions() {
        assert_eq!(
            registration_feedback(RegistrationState::Added),
            ("Passkey added.", true)
        );
        assert_eq!(
            registration_feedback(RegistrationState::MayHaveAdded),
            (
                "The passkey may have been added. The credential list was refreshed; review it before continuing.",
                true
            )
        );
        assert_eq!(
            registration_feedback(RegistrationState::Ceremony(CeremonyState::Cancelled)),
            ("Passkey request cancelled.", false)
        );
        assert_eq!(
            registration_feedback(RegistrationState::Indeterminate),
            (
                "The registration could not be confirmed. Try again after checking your passkeys.",
                false
            )
        );
        assert_eq!(
            registration_feedback(RegistrationState::CouldNotSave),
            ("The passkey could not be saved.", false)
        );
        assert_eq!(
            registration_feedback(RegistrationState::Invalid),
            ("The passkey registration was invalid. Try again.", false)
        );
        assert_eq!(
            registration_feedback(RegistrationState::Ceremony(CeremonyState::Unsupported)),
            ("Passkeys are not supported by this browser.", false)
        );
        assert_eq!(
            registration_feedback(RegistrationState::Ceremony(CeremonyState::Failed)),
            ("The passkey request failed. Try again.", false)
        );
    }

    #[test]
    fn deletion_feedback_refreshes_for_confirmed_or_indeterminate_outcomes() {
        assert_eq!(
            deletion_feedback(DeletionState::Deleted),
            ("Passkey removed.", true)
        );
        assert_eq!(
            deletion_feedback(DeletionState::MayHaveDeleted),
            (
                "The passkey may have been removed. The credential list was refreshed; review it before continuing.",
                true
            )
        );
    }
}
