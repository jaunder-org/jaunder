//! Host-owned test fixtures for password domain values.

use super::feed::{FeedMinDays, FeedMinItems};
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

/// Parses `s` into a valid [`FeedMinItems`] for tests.
///
/// # Panics
///
/// Panics if `s` is not a whole number of at least 1.
#[must_use]
pub fn parse_feed_min_items(s: &str) -> FeedMinItems {
    match s.parse() {
        Ok(value) => value,
        Err(error) => panic!("valid test feeds.min_items: {error}"),
    }
}

/// Parses `s` into a valid [`FeedMinDays`] for tests.
///
/// # Panics
///
/// Panics if `s` is not a whole number of at least 1.
#[must_use]
pub fn parse_feed_min_days(s: &str) -> FeedMinDays {
    match s.parse() {
        Ok(value) => value,
        Err(error) => panic!("valid test feeds.min_days: {error}"),
    }
}
