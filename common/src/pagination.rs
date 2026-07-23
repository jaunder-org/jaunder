//! Pagination page size — the `1..=50` range newtype (#537, ADR-0063).

use macros::NumNewtype;

/// A pagination page size, bounded to `1..=50` (the bound lives here, once).
///
/// `default()` is `50`, the web listing default. `AtomPub`'s default of `25` is its own
/// policy, expressed as [`PageSize::clamped`]`(25)`. The `clamp` affordance means an
/// out-of-range request coerces into range rather than rejecting — used by the public
/// `AtomPub` `?limit=` param; the web `#[server]` args instead reject out-of-range on the
/// wire via the serde bridge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = u32,
    min = 1,
    max = 50,
    default = 50,
    clamp,
    error = "page size must be between 1 and 50"
)]
pub struct PageSize(u32);

/// A pagination offset — the 0-based row offset into a listing.
///
/// Unlike [`PageSize`], there is **no range bound**: the full `u32` domain is valid, so this
/// carries no `min`/`max`/`clamp`. The type exists to **de-transpose** the `(limit, offset)`
/// pair on the media-listing path (#588) — two adjacent bare `u32`s can be swapped silently;
/// one typed argument makes that a compile error — not to validate a range. The `NumNewtype`
/// trailer still rejects a non-integer or negative value on parse/deserialize (the only error
/// path); `default()` is `0` (the first page).
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = u32, default = 0, error = "page offset must be a whole number")]
pub struct PageOffset(u32);

#[cfg(test)]
mod tests {
    use super::PageSize;

    #[test]
    fn page_size_surface() {
        // value()/From<Self> for u32, and trim
        assert_eq!("10".parse::<PageSize>().map(u32::from).ok(), Some(10));
        assert_eq!(
            "  50  ".parse::<PageSize>().map(PageSize::value).ok(),
            Some(50)
        );
        // FromStr rejects out-of-range and non-integers...
        for bad in ["0", "51", "abc", "-1", "1.5"] {
            assert!(bad.parse::<PageSize>().is_err(), "{bad} should reject");
        }
        // ...with the domain message
        assert!("0"
            .parse::<PageSize>()
            .err()
            .is_some_and(|e| e.to_string().starts_with("page size")));
        // Default is the web default (50), and Display round-trips
        let d = PageSize::default();
        assert_eq!(d.value(), 50);
        assert_eq!(d.to_string().parse::<PageSize>().ok(), Some(d));
        // serde: bare integer, round-trip, wire-rejection of out-of-range
        assert_eq!(serde_json::to_string(&d).ok(), Some("50".to_owned()));
        assert_eq!(
            serde_json::from_str::<PageSize>("25").map(u32::from).ok(),
            Some(25)
        );
        assert!(serde_json::from_str::<PageSize>("0").is_err());
        assert!(serde_json::from_str::<PageSize>("51").is_err());
        // clamp affordance: bounds + coercion
        assert_eq!(PageSize::MIN, 1);
        assert_eq!(PageSize::MAX, 50);
        assert_eq!(PageSize::clamped(0).value(), 1);
        assert_eq!(PageSize::clamped(999).value(), 50);
        assert_eq!(PageSize::clamped(25).value(), 25);
        // The shared test-support fixture builds a valid PageSize (its single door).
        assert_eq!(crate::test_support::parse_page_size("30").value(), 30);
    }

    #[test]
    fn page_offset_surface() {
        use super::PageOffset;
        // value()/From<Self>, trim, and the full u32 domain is valid (no upper bound).
        assert_eq!("0".parse::<PageOffset>().map(u32::from).ok(), Some(0));
        assert_eq!(
            "  4294967295  "
                .parse::<PageOffset>()
                .map(PageOffset::value)
                .ok(),
            Some(u32::MAX)
        );
        // FromStr rejects non-integers / negatives (the only error path)...
        for bad in ["abc", "-1", "1.5"] {
            assert!(bad.parse::<PageOffset>().is_err(), "{bad} should reject");
        }
        // ...with the domain message.
        assert!("abc"
            .parse::<PageOffset>()
            .err()
            .is_some_and(|e| e.to_string().starts_with("page offset")));
        // Default is 0 and Display round-trips.
        let d = PageOffset::default();
        assert_eq!(d.value(), 0);
        assert_eq!(d.to_string().parse::<PageOffset>().ok(), Some(d));
        // serde: bare integer, round-trip, wire-rejection of a non-integer.
        assert_eq!(serde_json::to_string(&d).ok(), Some("0".to_owned()));
        assert_eq!(
            serde_json::from_str::<PageOffset>("42").map(u32::from).ok(),
            Some(42)
        );
        assert!(serde_json::from_str::<PageOffset>("\"x\"").is_err());
        // The generated TryFrom<u32> (always-Ok for the unbounded type) — exercise the region.
        assert_eq!(PageOffset::try_from(7u32).map(u32::from), Ok(7));
        // The shared test-support fixture builds a valid PageOffset (its single door).
        assert_eq!(crate::test_support::parse_page_offset("5").value(), 5);
    }
}
