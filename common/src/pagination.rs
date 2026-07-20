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
}
