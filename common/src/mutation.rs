//! Wire-visible result of a database mutation whose commit acknowledgement may be lost.

use serde::{Deserialize, Serialize};

/// The observable result of a mutation after its write callback completed.
///
/// `CommitIndeterminate` means the database may have committed the callback's
/// mutation, but the server did not receive a commit acknowledgement. It is a
/// successful wire response so clients can revalidate their stale state rather
/// than treating a possibly completed write as rollback-confirmed failure.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum MutationOutcome<T> {
    /// The database acknowledged the commit.
    Confirmed(T),
    /// The callback completed, but commit acknowledgement was unavailable.
    CommitIndeterminate(T),
}

impl<T> MutationOutcome<T> {
    /// Transforms the callback value while preserving commit acknowledgement.
    #[must_use]
    pub fn map<U>(self, transform: impl FnOnce(T) -> U) -> MutationOutcome<U> {
        match self {
            Self::Confirmed(value) => MutationOutcome::Confirmed(transform(value)),
            Self::CommitIndeterminate(value) => {
                MutationOutcome::CommitIndeterminate(transform(value))
            }
        }
    }

    /// Borrows the callback value regardless of commit acknowledgement.
    #[must_use]
    pub fn value(&self) -> &T {
        match self {
            Self::Confirmed(value) | Self::CommitIndeterminate(value) => value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MutationOutcome;

    #[test]
    fn confirmed_wire_round_trips() {
        let outcome = MutationOutcome::Confirmed(42_u64);
        let json = serde_json::to_string(&outcome).expect("serialize confirmed outcome");
        assert_eq!(json, r#"{"Confirmed":42}"#);
        assert_eq!(
            serde_json::from_str::<MutationOutcome<u64>>(&json)
                .expect("deserialize confirmed outcome"),
            outcome
        );
    }

    #[test]
    fn indeterminate_wire_round_trips() {
        let outcome = MutationOutcome::CommitIndeterminate(42_u64);
        let json = serde_json::to_string(&outcome).expect("serialize indeterminate outcome");
        assert_eq!(json, r#"{"CommitIndeterminate":42}"#);
        assert_eq!(
            serde_json::from_str::<MutationOutcome<u64>>(&json)
                .expect("deserialize indeterminate outcome"),
            outcome
        );
    }

    #[test]
    fn map_preserves_confirmed_commit_acknowledgement() {
        let outcome = MutationOutcome::Confirmed("value").map(str::len);

        assert_eq!(outcome, MutationOutcome::Confirmed(5));
    }

    #[test]
    fn map_preserves_indeterminate_commit_acknowledgement() {
        let outcome = MutationOutcome::CommitIndeterminate("value").map(str::len);

        assert_eq!(outcome, MutationOutcome::CommitIndeterminate(5));
    }
}
