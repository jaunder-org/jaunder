//! `AtomPub` responses for transaction acknowledgement outcomes.

use axum::http::StatusCode;
use common::mutation::MutationOutcome;

/// Preserves a confirmed value or acknowledges an indeterminate commit to the client.
pub(super) fn confirmed_or_accepted<T>(outcome: MutationOutcome<T>) -> Result<T, StatusCode> {
    match outcome {
        MutationOutcome::Confirmed(value) => Ok(value),
        MutationOutcome::CommitIndeterminate(_) => Err(StatusCode::ACCEPTED),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmed_outcome_preserves_value() {
        assert!(matches!(
            confirmed_or_accepted(MutationOutcome::Confirmed(42)),
            Ok(42)
        ));
    }

    #[test]
    fn indeterminate_outcome_is_accepted() {
        let status = confirmed_or_accepted(MutationOutcome::CommitIndeterminate(())).unwrap_err();

        assert_eq!(status, StatusCode::ACCEPTED);
    }
}
