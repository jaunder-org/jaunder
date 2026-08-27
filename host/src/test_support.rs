//! Host-owned test fixtures for password domain values.

use super::password::Password;

/// Parses `s` into a valid [`Password`] for tests.
///
/// # Panics
///
/// Panics if `s` does not meet the shared password shape invariant.
#[must_use]
pub fn parse_password(s: &str) -> Password {
    match s.parse() {
        Ok(password) => password,
        Err(error) => panic!("valid test password: {error}"),
    }
}
