use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HybridWindow {
    pub min_items: u32,
    pub min_days: u32,
}

impl Default for HybridWindow {
    fn default() -> Self {
        Self {
            min_items: 20,
            min_days: 30,
        }
    }
}

pub trait HasPublishedAt {
    fn published_at(&self) -> DateTime<Utc>;
}

impl HybridWindow {
    #[must_use]
    pub fn cutoff_date(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        now - Duration::days(i64::from(self.min_days))
    }

    /// `posts` must be ordered by `published_at DESC`.
    /// Returns the prefix of posts where, for index `i`,
    /// `i < min_items` OR `published_at >= cutoff_date`.
    #[must_use]
    pub fn select<'a, P: HasPublishedAt>(&self, posts: &'a [P], now: DateTime<Utc>) -> &'a [P] {
        let cutoff = self.cutoff_date(now);
        let mut end = 0usize;
        for (i, p) in posts.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let i_u32 = i as u32;
            if i_u32 < self.min_items || p.published_at() >= cutoff {
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

    #[derive(Debug)]
    struct P(DateTime<Utc>);
    impl HasPublishedAt for P {
        fn published_at(&self) -> DateTime<Utc> {
            self.0
        }
    }

    fn at(days_ago: i64, now: DateTime<Utc>) -> P {
        P(now - Duration::days(days_ago))
    }

    #[test]
    fn default_window_uses_documented_defaults() {
        let w = HybridWindow::default();
        assert_eq!(w.min_items, 20);
        assert_eq!(w.min_days, 30);
    }

    #[test]
    fn empty_input_returns_empty() {
        let w = HybridWindow {
            min_items: 20,
            min_days: 30,
        };
        let now = Utc::now();
        assert!(w.select::<P>(&[], now).is_empty());
    }

    #[test]
    fn fewer_than_min_items_returns_all() {
        let w = HybridWindow {
            min_items: 20,
            min_days: 30,
        };
        let now = Utc::now();
        let posts: Vec<P> = (0..5).map(|i| at(i, now)).collect();
        assert_eq!(w.select(&posts, now).len(), 5);
    }

    #[test]
    fn quiet_blog_includes_min_items_even_if_all_older_than_min_days() {
        let w = HybridWindow {
            min_items: 20,
            min_days: 30,
        };
        let now = Utc::now();
        // 25 posts, all 100+ days ago
        let posts: Vec<P> = (0..25).map(|i| at(100 + i, now)).collect();
        // First 20 included because i < min_items; remaining 5 dropped (both predicates fail)
        assert_eq!(w.select(&posts, now).len(), 20);
    }

    #[test]
    fn busy_blog_includes_full_day_window_beyond_min_items() {
        let w = HybridWindow {
            min_items: 20,
            min_days: 30,
        };
        let now = Utc::now();
        // 50 posts all within the last 30 days
        let posts: Vec<P> = (0..50).map(|i| at(i / 2, now)).collect();
        assert_eq!(w.select(&posts, now).len(), 50);
    }

    #[test]
    fn union_stops_at_first_post_failing_both() {
        let w = HybridWindow {
            min_items: 3,
            min_days: 30,
        };
        let now = Utc::now();
        // posts at days_ago = [1, 2, 3, 100, 200] (5 posts)
        // i=0 (1d ago): i<3 → keep
        // i=1 (2d):     i<3 → keep
        // i=2 (3d):     i<3 → keep
        // i=3 (100d):   i>=3 AND published>=cutoff(30d ago)? 100 days ago < cutoff → drop
        let posts = vec![
            at(1, now),
            at(2, now),
            at(3, now),
            at(100, now),
            at(200, now),
        ];
        let kept = w.select(&posts, now);
        assert_eq!(kept.len(), 3);
    }
}
