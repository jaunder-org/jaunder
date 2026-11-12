//! Persisted roles for the offline queue's identity and operation name.

use std::str::FromStr;

/// Monotonically allocated identity of one queued operation.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, macros::IdNewtype)]
pub(crate) struct CodeMigrationQueueId(i64);

/// `SQLite`'s stable row identity, scoped to one offline table conversion.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, macros::IdNewtype)]
pub(crate) struct LegacyInstantRowId(i64);

/// `PostgreSQL`'s physical tuple identity, retained only for offline diagnostics.
#[derive(Clone, Debug, macros::SqlxBridge)]
pub(crate) struct LegacyInstantTupleId(String);

impl std::fmt::Display for LegacyInstantTupleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The stored dispatch key. Unknown values remain decodable so dispatch can
/// report an explicit unsupported operation and keep the queue row intact.
#[derive(Clone, Debug, PartialEq, Eq, macros::StrNewtype)]
pub(crate) struct CodeMigrationOperation(String);

#[derive(Debug, thiserror::Error)]
#[error("code migration operation must not be empty")]
pub(crate) struct EmptyCodeMigrationOperation;

impl FromStr for CodeMigrationOperation {
    type Err = EmptyCodeMigrationOperation;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            return Err(EmptyCodeMigrationOperation);
        }
        Ok(Self(value.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::CodeMigrationOperation;

    #[test]
    fn persisted_operation_requires_a_nonempty_name() {
        assert!("".parse::<CodeMigrationOperation>().is_err());
        assert!("future_operation".parse::<CodeMigrationOperation>().is_ok());
    }
}
