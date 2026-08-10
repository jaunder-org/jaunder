//! Naming a required row that is not there.
//!
//! `sqlx::Error::RowNotFound` is what `fetch_one` returns when a query matched
//! nothing. Lifted anonymously to the error boundary it becomes
//! [`ErrorClass::Bug`] — ERROR-level and pageable — with the useless operator
//! text `"no rows returned"`: neither the table nor the reason.
//!
//! So absence is *named* wherever a row can genuinely be missing. `fetch_one`
//! stays the right call for a row the statement structurally guarantees — a
//! bare aggregate, a `SELECT EXISTS`, an `INSERT … RETURNING` with no
//! `ON CONFLICT` — because there absence is not a case at all. Everywhere else
//! it is modelled at its source: either as an `Option` — via `fetch_optional`,
//! the dominant idiom, where the caller should decide — or, where the row is
//! genuinely required and its absence names a broken invariant, as
//! [`MissingRow`] via [`RequireRow::require_row`].
//!
//! [`ErrorClass::Bug`]: host::error::ErrorClass::Bug

use host::error::InternalError;
use thiserror::Error;

/// A row the caller requires is not there.
///
/// `what` names the row for the operator, which is the whole point of the type:
/// it replaces an anonymous `RowNotFound` with the row's own description.
#[derive(Debug, Error)]
#[error("expected row is missing: {what}")]
pub struct MissingRow {
    /// A human-readable name for the absent row, e.g.
    /// `"the seeded 'local' channel row"`.
    what: &'static str,
}

impl From<MissingRow> for InternalError {
    /// A missing required row still pages — it is a broken invariant, class
    /// `Bug`. It is not a downgrade; it is a *legible* bug.
    ///
    /// Routed through [`InternalError::server`], not `server_message`: the
    /// latter builds its source from `anyhow::Error::msg(String)` and so
    /// discards the typed error, which ADR-0017 §3 requires the boundary to
    /// preserve.
    fn from(missing: MissingRow) -> Self {
        InternalError::server(missing)
    }
}

/// Turns "this row may be absent" into "this row is required, and here is its
/// name".
///
/// The partner of `fetch_optional` wherever the row is required but *can* be
/// absent: `…fetch_optional(pool).await?.require_row("…")?`. The driver error
/// takes the path it always took; only absence is named.
pub trait RequireRow<T> {
    /// Names the row and requires it, mapping absence to [`MissingRow`].
    ///
    /// Requiring `what` is the forcing function: a caller cannot read a
    /// required row without writing down which row it is.
    ///
    /// # Errors
    ///
    /// Returns [`MissingRow`] naming `what` if the row is absent.
    fn require_row(self, what: &'static str) -> Result<T, MissingRow>;
}

impl<T> RequireRow<T> for Option<T> {
    fn require_row(self, what: &'static str) -> Result<T, MissingRow> {
        self.ok_or(MissingRow { what })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use host::error::{ErrorClass, ErrorKind};

    #[test]
    fn require_row_passes_a_present_row_through() {
        assert_eq!(Some(7).require_row("a row").expect("present"), 7);
    }

    #[test]
    fn missing_row_maps_to_internal_kind_bug_class_and_names_the_row() {
        let missing = Option::<i32>::None
            .require_row("the seeded 'local' channel row")
            .expect_err("absent");
        let error: InternalError = missing.into();
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
}
