use super::{FeedMinDays, FeedMinItems};
use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HybridWindow {
    pub min_items: FeedMinItems,
    pub min_days: FeedMinDays,
}

pub trait HasPublishedAt {
    fn published_at(&self) -> DateTime<Utc>;
}

impl HybridWindow {
    /// Returns `None` when the cutoff predates all representable timestamps.
    #[must_use]
    pub fn cutoff_date(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        now.checked_sub_signed(Duration::days(i64::from(self.min_days.value())))
    }

    /// `posts` must be ordered by `published_at DESC`.
    /// Returns the prefix of posts where, for index `i`, `i < min_items` or
    /// the cutoff is unrepresentable or `published_at >= cutoff_date`.
    #[must_use]
    pub fn select<'a, P: HasPublishedAt>(&self, posts: &'a [P], now: DateTime<Utc>) -> &'a [P] {
        let cutoff = self.cutoff_date(now);
        let min_items = usize::try_from(self.min_items.value()).unwrap_or(usize::MAX);
        let mut end = 0usize;
        for (i, p) in posts.iter().enumerate() {
            if i < min_items || cutoff.is_none_or(|cutoff| p.published_at() >= cutoff) {
                end = i + 1;
            } else {
                break;
            }
        }
        &posts[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{parse_feed_min_days, parse_feed_min_items};

    #[derive(Debug)]
    struct P(DateTime<Utc>);
    impl HasPublishedAt for P {
        fn published_at(&self) -> DateTime<Utc> {
            self.0
        }
    }

    #[derive(Debug)]
    struct IdentifiedP {
        id: u8,
        published_at: DateTime<Utc>,
    }
    impl HasPublishedAt for IdentifiedP {
        fn published_at(&self) -> DateTime<Utc> {
            self.published_at
        }
    }

    fn identified_at(id: u8, days_ago: i64, now: DateTime<Utc>) -> IdentifiedP {
        IdentifiedP {
            id,
            published_at: now - Duration::days(days_ago),
        }
    }

    fn at(days_ago: i64, now: DateTime<Utc>) -> P {
        P(now - Duration::days(days_ago))
    }

    #[test]
    fn default_window_uses_documented_defaults() {
        let w = HybridWindow::default();
        assert_eq!(w.min_items.value(), 20);
        assert_eq!(w.min_days.value(), 30);
    }

    #[test]
    fn empty_input_returns_empty() {
        let w = HybridWindow::default();
        let now = Utc::now();
        assert!(w.select::<P>(&[], now).is_empty());
    }

    #[test]
    fn fewer_than_min_items_returns_all() {
        let w = HybridWindow::default();
        let now = Utc::now();
        let posts: Vec<P> = (0..5).map(|i| at(i, now)).collect();
        assert_eq!(w.select(&posts, now).len(), 5);
    }

    #[test]
    fn quiet_blog_includes_min_items_even_if_all_older_than_min_days() {
        let w = HybridWindow::default();
        let now = Utc::now();
        // 25 posts, all 100+ days ago
        let posts: Vec<P> = (0..25).map(|i| at(100 + i, now)).collect();
        // First 20 included because i < min_items; remaining 5 dropped (both predicates fail)
        assert_eq!(w.select(&posts, now).len(), 20);
    }

    #[test]
    fn busy_blog_includes_full_day_window_beyond_min_items() {
        let w = HybridWindow::default();
        let now = Utc::now();
        // 50 posts all within the last 30 days
        let posts: Vec<P> = (0..50).map(|i| at(i / 2, now)).collect();
        assert_eq!(w.select(&posts, now).len(), 50);
    }

    #[test]
    fn union_keeps_minimum_items_and_inclusive_cutoff() {
        let window = HybridWindow {
            min_items: parse_feed_min_items("1"),
            min_days: parse_feed_min_days("30"),
        };
        let now = Utc::now();
        let posts = vec![
            identified_at(1, 1, now),
            identified_at(2, 30, now),
            identified_at(3, 31, now),
            identified_at(4, 100, now),
        ];

        // The item exactly on the cutoff joins the count-floor item; the next
        // older item fails both predicates, ending the ordered prefix.
        let selected = window.select(&posts, now);
        let actual: Vec<_> = selected
            .iter()
            .map(|post| (post.id, post.published_at))
            .collect();
        let expected = vec![(1, now - Duration::days(1)), (2, now - Duration::days(30))];
        assert_eq!(actual, expected);
    }

    #[test]
    fn unrepresentably_old_cutoff_selects_all_history() {
        let window = HybridWindow {
            min_items: parse_feed_min_items("1"),
            min_days: parse_feed_min_days(&u32::MAX.to_string()),
        };
        let now = Utc::now();
        let posts = vec![
            identified_at(1, 1, now),
            identified_at(2, 31, now),
            identified_at(3, 365, now),
        ];

        // A valid but unrepresentable cutoff is older than every eligible post.
        let selected = window.select(&posts, now);
        let actual: Vec<_> = selected
            .iter()
            .map(|post| (post.id, post.published_at))
            .collect();
        let expected = vec![
            (1, now - Duration::days(1)),
            (2, now - Duration::days(31)),
            (3, now - Duration::days(365)),
        ];
        assert_eq!(actual, expected);
    }
}
