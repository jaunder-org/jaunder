//! The storage crate's error carrier, and the one door for reading a required
//! row.
//!
//! `sqlx::Error` does not leave this crate for row access (#343). Two reasons,
//! and they are different:
//!
//! 1. **Absence is not a driver failure.** `sqlx::Error::RowNotFound` is what
//!    `fetch_one` returns when a query matched nothing. Lifted anonymously to
//!    the error boundary it becomes [`ErrorClass::Bug`] — ERROR-level and
//!    pageable — so an ordinary missing row can wake an operator. Absence is
//!    modelled here instead: as an `Option` (via `fetch_optional`) where the
//!    caller should decide, or as [`StorageError::MissingRow`] where the row is
//!    genuinely required and its absence names a broken invariant.
//! 2. **The rule has to be enforceable.** `fetch_one` is banned workspace-wide
//!    via `clippy.toml`'s `disallowed-methods`; [`fetch_exactly_one`] and
//!    [`fetch_exactly_one_scalar`] are the sanctioned replacements, and they
//!    are built on `fetch_optional`, so they need no lint suppression and no
//!    `RowNotFound` is ever constructed anywhere in the tree.
//!
//! [`ErrorClass::Bug`]: host::error::ErrorClass::Bug

use host::error::InternalError;
use thiserror::Error;

/// A failure from the persistence layer.
///
/// Deliberately two variants, not one: an infrastructure failure and a missing
/// required row are both bugs, but they are *different* bugs, and an operator
/// reading the page needs to know which.
#[derive(Debug, Error)]
pub enum StorageError {
    /// An infrastructure failure — pool exhaustion, I/O, protocol, a violated
    /// constraint.
    ///
    /// Never absence: `RowNotFound` cannot reach this variant, because nothing
    /// in the crate calls `fetch_one`.
    #[error(transparent)]
    Db(#[from] sqlx::Error),

    /// A row the caller requires is not there.
    ///
    /// `what` names the row for the operator, which is the whole point of the
    /// variant — `"storage operation failed"` with `"no rows returned"` buried
    /// in the source chain names neither the table nor the reason.
    #[error("expected row is missing: {what}")]
    MissingRow {
        /// A human-readable name for the absent row, e.g.
        /// `"the seeded 'local' channel row"`.
        what: &'static str,
    },
}

impl From<StorageError> for InternalError {
    /// Both arms are `ErrorClass::Bug` and both page. `MissingRow` is not a
    /// downgrade — it is a *legible* bug.
    ///
    /// `MissingRow` routes through [`InternalError::server`], not
    /// `server_message`: the latter builds its source from
    /// `anyhow::Error::msg(String)` and so discards the typed error, which
    /// ADR-0017 §3 requires the boundary to preserve.
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Db(e) => InternalError::storage(e),
            missing @ StorageError::MissingRow { .. } => InternalError::server(missing),
        }
    }
}

/// Turns "the row may be absent" into "the row is required, and here is its
/// name" — the single place absence becomes [`StorageError::MissingRow`].
///
/// Both fetch wrappers below end here rather than each writing their own `None`
/// arm. That is not only deduplication: the arm is reachable from *some*
/// callers and not others (a `RETURNING` read can never miss), so one shared
/// arm is one line the suite covers once, instead of one uncovered line per
/// wrapper shape.
fn require_row<O>(row: Option<O>, what: &'static str) -> Result<O, StorageError> {
    row.ok_or(StorageError::MissingRow { what })
}

/// Fetches a single-column row that must exist, naming it if it does not.
///
/// The sanctioned replacement for `fetch_one` on a `query_scalar`. Built on
/// `fetch_optional`, so the banned method appears nowhere and `RowNotFound` is
/// never constructed; the absent case becomes [`StorageError::MissingRow`]
/// carrying `what`.
///
/// Requiring `what` is the forcing function: a caller cannot read a required
/// row without writing down which row it is. Use this even where the query is
/// row-guaranteed (`COUNT`, `INSERT … RETURNING`) — the arm is then unreachable,
/// costs nothing, and catches the day someone adds an `ON CONFLICT DO NOTHING`
/// that silently makes the row optional.
pub(crate) async fn fetch_exactly_one_scalar<'q, DB, O, A, E>(
    query: sqlx::query::QueryScalar<'q, DB, O, A>,
    executor: E,
    what: &'static str,
) -> Result<O, StorageError>
where
    DB: sqlx::Database,
    O: Send + Unpin,
    (O,): Send + Unpin + for<'r> sqlx::FromRow<'r, DB::Row>,
    A: 'q + sqlx::IntoArguments<'q, DB> + Send,
    E: sqlx::Executor<'q, Database = DB>,
{
    require_row(query.fetch_optional(executor).await?, what)
}

/// The multi-column twin of [`fetch_exactly_one_scalar`], for a `query_as`.
///
/// This shape was written first, removed for want of a caller, and brought back
/// by #343's `posts` slice: `update_post` reads a whole [`PostRow`] back through
/// `RETURNING`, which no scalar wrapper can express.
///
/// [`PostRow`]: crate::helpers::PostRow
pub(crate) async fn fetch_exactly_one<'q, DB, O, A, E>(
    query: sqlx::query::QueryAs<'q, DB, O, A>,
    executor: E,
    what: &'static str,
) -> Result<O, StorageError>
where
    DB: sqlx::Database,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, DB::Row>,
    A: 'q + sqlx::IntoArguments<'q, DB> + Send,
    E: sqlx::Executor<'q, Database = DB>,
{
    require_row(query.fetch_optional(executor).await?, what)
}

#[cfg(test)]
mod tests {
    use super::*;
    use host::error::{ErrorClass, ErrorKind};

    #[test]
    fn db_maps_to_storage_kind_and_bug_class() {
        let error: InternalError = StorageError::Db(sqlx::Error::PoolClosed).into();
        assert_eq!(error.kind(), ErrorKind::Storage);
        assert_eq!(error.class(), ErrorClass::Bug);
        assert_eq!(error.public_message(), "storage operation failed");
    }

    #[test]
    fn missing_row_maps_to_internal_kind_bug_class_and_names_the_row() {
        let error: InternalError = StorageError::MissingRow {
            what: "the seeded 'local' channel row",
        }
        .into();
        assert_eq!(error.kind(), ErrorKind::Internal);
        assert_eq!(error.class(), ErrorClass::Bug);
        // Masked on the wire...
        assert_eq!(error.public_message(), "server operation failed");
        // ...but the operator is told exactly which row is gone. Bound rather
        // than called twice: a lazily-evaluated `assert!` message argument is
        // never executed on the passing path, and so reads as uncovered.
        let operator = error.operator_message();
        assert!(
            operator.contains("the seeded 'local' channel row"),
            "operator message must name the row, got: {operator}"
        );
    }

    /// The `Db` arm must keep the driver error as a *source*, not flatten it
    /// into a string — `operator_message` walks the chain, so a stringified
    /// lift would lose the nested cause (ADR-0017 §3).
    ///
    /// Note this is the only arm where the choice is observable.
    /// `MissingRow` carries no nested source of its own, so
    /// `InternalError::server(missing)` and a hypothetical
    /// `server_message(missing.to_string())` render identically today. `server`
    /// is used anyway, so the typed error is in the chain for anyone
    /// downcasting it and so the arm does not have to change if `MissingRow`
    /// ever gains a cause.
    #[test]
    fn db_keeps_the_driver_error_as_a_source_not_a_string() {
        let error: InternalError = StorageError::Db(sqlx::Error::PoolClosed).into();
        assert_eq!(
            error.operator_message(),
            sqlx::Error::PoolClosed.to_string(),
            "the sqlx error must render through the preserved chain"
        );
    }
}
