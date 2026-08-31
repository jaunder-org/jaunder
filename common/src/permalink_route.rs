use std::str::FromStr;

use crate::slug::Slug;
use crate::time::PermalinkDate;
use crate::username::Username;

/// A fully parsed public Post permalink route (#697, ADR-0063 §4).
///
/// The router-specific `~` marker is intentionally absent: adapters normalize it
/// before parsing, so all callers hold only the typed Post identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermalinkRoute {
    /// The canonical author identifier.
    pub username: Username,
    /// The calendar date in the permalink.
    pub date: PermalinkDate,
    /// The Post's canonical slug.
    pub slug: Slug,
}

impl PermalinkRoute {
    /// Parses five decoded permalink segments all-or-nothing.
    ///
    /// This deliberately accepts the standard integer grammar unchanged: years
    /// use [`i32::from_str`] and months/days use [`u32::from_str`].
    #[must_use]
    pub fn parse(username: &str, year: &str, month: &str, day: &str, slug: &str) -> Option<Self> {
        let username = username.parse().ok()?;
        let year = i32::from_str(year).ok()?;
        let month = u32::from_str(month).ok()?;
        let day = u32::from_str(day).ok()?;
        let date = PermalinkDate::from_ymd(year, month, day)?;
        let slug = slug.parse().ok()?;
        Some(Self {
            username,
            date,
            slug,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PermalinkRoute;
    use crate::{slug::Slug, time::PermalinkDate, username::Username};

    #[test]
    fn parses_zero_padded_permalink_segments() {
        let route = PermalinkRoute::parse("alice", "2026", "01", "02", "hello")
            .expect("valid route segments parse");

        assert_eq!(
            route.username,
            "alice".parse::<Username>().expect("valid username")
        );
        assert_eq!(
            route.date,
            PermalinkDate::from_ymd(2026, 1, 2).expect("valid calendar date")
        );
        assert_eq!(route.slug, "hello".parse::<Slug>().expect("valid slug"));
    }

    #[test]
    fn accepts_signed_years_with_i32_semantics() {
        let positive = PermalinkRoute::parse("alice", "+2026", "01", "02", "hello")
            .expect("a signed year permitted by i32 parses");
        let negative = PermalinkRoute::parse("alice", "-0001", "01", "02", "hello")
            .expect("a negative year permitted by i32 parses");

        assert_eq!(
            positive.date,
            PermalinkDate::from_ymd(2026, 1, 2).expect("valid calendar date")
        );
        assert_eq!(
            negative.date,
            PermalinkDate::from_ymd(-1, 1, 2).expect("valid calendar date")
        );
    }

    #[test]
    fn rejects_overflow_and_non_numeric_date_segments() {
        assert_eq!(
            PermalinkRoute::parse("alice", "2147483648", "01", "02", "hello"),
            None
        );
        assert_eq!(
            PermalinkRoute::parse("alice", "2026", "0x1", "02", "hello"),
            None
        );
        assert_eq!(
            PermalinkRoute::parse("alice", "2026", "01", "4294967296", "hello"),
            None
        );
    }

    #[test]
    fn rejects_invalid_username_and_slug() {
        assert_eq!(
            PermalinkRoute::parse("~alice", "2026", "01", "02", "hello"),
            None
        );
        assert_eq!(
            PermalinkRoute::parse("alice", "2026", "01", "02", "Not A Slug!"),
            None
        );
    }

    #[test]
    fn rejects_impossible_calendar_dates() {
        assert_eq!(
            PermalinkRoute::parse("alice", "2026", "02", "29", "hello"),
            None
        );
    }
}
