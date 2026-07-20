//! Validated numeric-value newtypes for the feed-window settings stored in `site_config`
//! (`feeds.min_items` / `feeds.min_days`). Both are `u32` with a **min-1** invariant — a
//! "minimum" of zero items, or a zero-day history window, is degenerate — enforced by the
//! `NumNewtype` derive's generated `FromStr`. Distinct types (not one shared `u32`) so the
//! two can't be transposed at a `HybridWindow`/`FeedsConfig` construction site.

use macros::NumNewtype;

/// The minimum number of items to include in any feed, regardless of age (default 20).
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = u32,
    min = 1,
    default = 20,
    error = "feeds.min_items must be a whole number of at least 1"
)]
pub struct FeedMinItems(u32);

/// The minimum age window, in days, of items to include in any feed, regardless of count
/// (default 30).
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = u32,
    min = 1,
    default = 30,
    error = "feeds.min_days must be a whole number of at least 1"
)]
pub struct FeedMinDays(u32);

#[cfg(test)]
mod tests {
    use super::*;

    // Each type is a distinct monomorphization, so both exercise the full generated surface
    // (FromStr accept/reject, get, Display, Default, and the serde bridge) — `FeedsConfig`
    // does not serde these, so the unit tests are the only reachability for that code.

    #[test]
    fn min_items_full_surface() {
        assert_eq!("5".parse::<FeedMinItems>().unwrap().value(), 5);
        assert_eq!("  20  ".parse::<FeedMinItems>().unwrap().value(), 20);
        for bad in ["0", "", "-1", "abc", "1.5"] {
            assert!(bad.parse::<FeedMinItems>().is_err(), "{bad} should reject");
        }
        assert!("0"
            .parse::<FeedMinItems>()
            .unwrap_err()
            .to_string()
            .starts_with("feeds.min_items"));
        assert_eq!(FeedMinItems::default().value(), 20);
        let d = FeedMinItems::default();
        assert_eq!(u32::from(d), 20); // From<Self> for the inner
        assert_eq!(d.to_string(), "20");
        assert_eq!(d.to_string().parse::<FeedMinItems>().unwrap(), d);
        // serde: bare integer, wire-rejects out-of-range.
        assert_eq!(serde_json::to_string(&d).unwrap(), "20");
        assert_eq!(
            serde_json::from_str::<FeedMinItems>("42").unwrap().value(),
            42
        );
        assert!(serde_json::from_str::<FeedMinItems>("0").is_err());
    }

    #[test]
    fn min_days_full_surface() {
        assert_eq!("7".parse::<FeedMinDays>().unwrap().value(), 7);
        assert!("0".parse::<FeedMinDays>().is_err());
        assert!("0"
            .parse::<FeedMinDays>()
            .unwrap_err()
            .to_string()
            .starts_with("feeds.min_days"));
        assert_eq!(FeedMinDays::default().value(), 30);
        let d = FeedMinDays::default();
        assert_eq!(u32::from(d), 30); // From<Self> for the inner
        assert_eq!(d.to_string(), "30");
        assert_eq!(d.to_string().parse::<FeedMinDays>().unwrap(), d);
        assert_eq!(serde_json::to_string(&d).unwrap(), "30");
        assert_eq!(
            serde_json::from_str::<FeedMinDays>("15").unwrap().value(),
            15
        );
        assert!(serde_json::from_str::<FeedMinDays>("0").is_err());
    }
}
