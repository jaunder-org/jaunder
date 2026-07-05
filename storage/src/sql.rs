//! Shared SQL-string helpers used by both dialects' assembled (non-placeholder) SQL.

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
}
