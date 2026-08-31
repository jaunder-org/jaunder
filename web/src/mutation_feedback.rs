//! Shared presentation policy for mutation settlement.

use common::MutationOutcome;

use crate::error::WebError;

/// A confirmed value, or error-like copy for a failed or indeterminate mutation.
pub(crate) enum MutationFeedback<T> {
    Confirmed(T),
    Error(String),
}

/// Classifies mutation settlement without letting indeterminate commits look successful.
pub(crate) fn classify<T>(
    result: Result<MutationOutcome<T>, WebError>,
    indeterminate_message: &'static str,
) -> MutationFeedback<T> {
    match result {
        Ok(MutationOutcome::Confirmed(value)) => MutationFeedback::Confirmed(value),
        Ok(MutationOutcome::CommitIndeterminate(_)) => {
            MutationFeedback::Error(indeterminate_message.to_owned())
        }
        Err(error) => MutationFeedback::Error(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use common::MutationOutcome;

    use super::{MutationFeedback, classify};
    use crate::error::WebError;

    #[test]
    fn settlement_classification_reserves_success_for_confirmed_mutations() {
        assert!(matches!(
            classify::<()>(Ok(MutationOutcome::Confirmed(())), "unknown"),
            MutationFeedback::Confirmed(())
        ));
        assert!(matches!(
            classify::<()>(Ok(MutationOutcome::CommitIndeterminate(())), "unknown"),
            MutationFeedback::Error(message) if message == "unknown"
        ));
        assert!(matches!(
            classify::<()>(Err(WebError::validation("invalid")), "unknown"),
            MutationFeedback::Error(message) if message == "invalid"
        ));
    }
}
