//! Shared SQL-string helpers used by both dialects' assembled (non-placeholder) SQL.

/// A row cardinality decoded from SQL.
///
/// SQL count expressions are signed on both supported backends; the declaration
/// rejects the corrupt negative values that cannot represent a cardinality.
#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 0,
    error = "row count must be a non-negative integer"
)]
pub(crate) struct RowCount(i64);

impl RowCount {
    /// Converts this checked count to the unsigned representation exposed by
    /// count-facing storage APIs.
    #[must_use]
    pub(crate) fn into_u64(self) -> u64 {
        match u64::try_from(self.0) {
            Ok(count) => count,
            Err(_) => unreachable!("RowCount rejects negative values"),
        }
    }
}

/// A boolean fact decoded from an SQL `EXISTS` expression.
#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::SqlxBridge)]
pub(crate) struct Exists(bool);

impl Exists {
    /// Returns the existence fact as a Rust boolean.
    #[must_use]
    pub(crate) const fn into_bool(self) -> bool {
        self.0
    }
}

/// SQL-standard identifier quoting: wrap in double quotes, doubling any interior `"`.
pub(crate) fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// SQL-standard literal quoting: wrap in single quotes, doubling any interior `'`.
pub(crate) fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_identifier_wraps_and_escapes_double_quotes() {
        assert_eq!(quote_identifier("users"), "\"users\"");
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn quote_literal_wraps_and_escapes_single_quotes() {
        assert_eq!(quote_literal("password"), "'password'");
        assert_eq!(quote_literal("can't"), "'can''t'");
    }

    #[test]
    fn row_count_rejects_negative_values_and_converts_valid_boundaries_losslessly() {
        assert_eq!(RowCount::try_from(0).unwrap().into_u64(), 0);
        assert_eq!(
            RowCount::try_from(i64::MAX).unwrap().into_u64(),
            i64::MAX.unsigned_abs()
        );
        assert!(RowCount::try_from(-1).is_err());
    }
}
