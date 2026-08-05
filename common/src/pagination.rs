//! Pagination quantities — the page a reader sees, the offset into a listing, and the
//! row count a query fetches (#537, #588, #696; ADR-0063).
//!
//! [`PageSize`] and [`RowLimit`] are deliberately **different numbers**: a page is what
//! the reader gets, a row limit is what the query asks for, and for a paginated listing
//! the second is the first **plus one** — over-fetching a single row is how a next page
//! is detected without a second `COUNT(*)`. That `+1` and its inverse live together on
//! `PageSize` ([`PageSize::fetch_limit`] / [`PageSize::has_more`]) so the two halves
//! cannot drift apart; see #696.

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
/// The type exists to **de-transpose** the `(limit, offset)` pair on the media-listing
/// path (#588): two adjacent bare integers can be swapped silently, and one typed
/// argument makes that a compile error.
///
/// **Bounded `>= 0` by a declared `min`, not by an unsigned inner.** It carried `u32`
/// until #696; the bound moved into `min = 0` because `sqlx` has no Postgres `Encode`
/// for unsigned types, so a `u32` inner is widened away at every bind and its guarantee
/// does not survive the boundary — while a declared `min` is re-run by `FromStr`, the
/// serde bridge, and the sqlx `Decode`. `default()` is `0` (the first page).
///
/// **There is deliberately no `max`, and adding one would be a mistake.** An offset's
/// only meaningful upper bound is *the number of rows that exist*, which is not a
/// constant; any literal would be an invented number wearing the authority of a
/// validated invariant. [`PageSize`] can declare `max = 50` because that is a real
/// policy about what a page may contain — there is no equivalent policy about how far
/// into a listing a reader may skip.
///
/// The consequence, stated so it is not mistaken for an oversight: this is a `#[server]`
/// wire argument, and an offset between `u32::MAX` and `i64::MAX` is now **accepted**
/// where the old `u32` inner rejected it. It yields an empty page rather than a
/// validation error. That is a deliberate trade for removing the widening at the bind
/// (#696); it is not a hole to be closed by inventing a cap.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 0,
    default = 0,
    error = "page offset must be a whole number"
)]
pub struct PageOffset(i64);

/// How many rows a query fetches — the **storage-side** quantity, distinct from the
/// [`PageSize`] a reader sees (see the module doc).
///
/// Bounded `>= 1`: a zero-row query is pointless, and a *negative* limit is the real
/// hazard — `SQLite` reads a negative `LIMIT` as "no limit" and returns every row, where
/// Postgres errors, so an unbounded limit would be a backend-parity hole as well as a
/// resource one (ADR-0019). The bound is **declared** rather than implied by an unsigned
/// inner, because `sqlx` has no Postgres `Encode` for unsigned types: a `u32` inner would
/// be widened away at every bind, while a declared `min` is re-run by `FromStr`, the serde
/// bridge, and the sqlx `Decode` (#696, ADR-0071).
///
/// Two ways to obtain one, and no others: [`PageSize::fetch_limit`] for a paginated
/// listing (it applies the has-more `+1`), or [`RowLimit::at_most`] for a flat cap with no
/// page behind it. There is deliberately no `default` — there is no sensible default
/// number of rows to fetch.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = i64, min = 1, error = "fetch limit must be at least 1")]
pub struct RowLimit(i64);

impl RowLimit {
    /// A flat cap — "at most `n` rows", with no page behind it.
    ///
    /// Saturates a value below `1` up to `1`, so it is a **validated** door: it cannot
    /// yield an out-of-range `RowLimit`, which is the same justification
    /// [`PageSize::clamped`] carries, with only a lower bound to enforce (ADR-0063 §2).
    /// `const` so a caller can name its cap as a constant; the fallible `TryFrom<i64>` is
    /// the door for a value that might genuinely be out of range.
    #[must_use]
    pub const fn at_most(n: i64) -> Self {
        Self(if n < 1 { 1 } else { n })
    }
}

impl PageSize {
    /// Rows to fetch for one page: the page **plus one extra**, so a full page and one
    /// more row proves a next page exists without a second `COUNT(*)`.
    ///
    /// The single place this `+1` is derived. [`Self::has_more`] is its inverse and lives
    /// next to it on purpose — the two halves have to agree, and a caller that spelled
    /// either by hand could drift from the other (#696).
    #[must_use]
    pub const fn fetch_limit(self) -> RowLimit {
        // Constructs `RowLimit` directly rather than through its `TryFrom`, because a
        // `const fn` cannot `?`. Not a bypass of the bound: `PageSize` is `1..=50`, so
        // this is always `2..=51` and satisfies `RowLimit`'s `min = 1` by construction.
        RowLimit(self.0 as i64 + 1)
    }

    /// Rows to fetch for exactly one page, with **no** has-more probe.
    ///
    /// For a listing that returns a page and does not report whether another exists —
    /// the media listing and the tag dropdown, which have no "load more" affordance.
    /// Contrast [`Self::fetch_limit`], which fetches the extra probing row; using that
    /// one here would return a row the caller then has to know to discard.
    #[must_use]
    pub const fn exact_limit(self) -> RowLimit {
        // `PageSize` is `1..=50`, so this satisfies `RowLimit`'s `min = 1`; see the note
        // on `fetch_limit` for why this constructs directly rather than via `TryFrom`.
        RowLimit(self.0 as i64)
    }

    /// Whether an over-fetched row set proves another page exists — the inverse of
    /// [`Self::fetch_limit`].
    #[must_use]
    pub const fn has_more(self, fetched: usize) -> bool {
        fetched > self.page_len()
    }

    /// The page's own length: what an over-fetched row set truncates back to.
    #[must_use]
    pub const fn page_len(self) -> usize {
        self.0 as usize
    }
}

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
        assert!(
            "0".parse::<PageSize>()
                .err()
                .is_some_and(|e| e.to_string().starts_with("page size"))
        );
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
        // value()/From<Self>, trim, and no upper bound — including past `u32::MAX`,
        // which the pre-#696 `u32` inner rejected. See the type's doc: that widening is
        // deliberate, and `page_offset_rejects_negative` pins the floor that matters.
        assert_eq!("0".parse::<PageOffset>().map(i64::from).ok(), Some(0));
        assert_eq!(
            "  4294967295  "
                .parse::<PageOffset>()
                .map(PageOffset::value)
                .ok(),
            Some(i64::from(u32::MAX))
        );
        assert_eq!(
            "4294967296"
                .parse::<PageOffset>()
                .map(PageOffset::value)
                .ok(),
            Some(4_294_967_296)
        );
        // FromStr rejects non-integers / negatives (the only error path)...
        for bad in ["abc", "-1", "1.5"] {
            assert!(bad.parse::<PageOffset>().is_err(), "{bad} should reject");
        }
        // ...with the domain message.
        assert!(
            "abc"
                .parse::<PageOffset>()
                .err()
                .is_some_and(|e| e.to_string().starts_with("page offset"))
        );
        // Default is 0 and Display round-trips.
        let d = PageOffset::default();
        assert_eq!(d.value(), 0);
        assert_eq!(d.to_string().parse::<PageOffset>().ok(), Some(d));
        // serde: bare integer, round-trip, wire-rejection of a non-integer.
        assert_eq!(serde_json::to_string(&d).ok(), Some("0".to_owned()));
        assert_eq!(
            serde_json::from_str::<PageOffset>("42").map(i64::from).ok(),
            Some(42)
        );
        assert!(serde_json::from_str::<PageOffset>("\"x\"").is_err());
        // The generated TryFrom<i64> — exercise the Ok region; the Err region is
        // `page_offset_rejects_negative`.
        assert_eq!(PageOffset::try_from(7_i64).map(i64::from), Ok(7));
        // The shared test-support fixture builds a valid PageOffset (its single door).
        assert_eq!(crate::test_support::parse_page_offset("5").value(), 5);
    }

    #[test]
    fn page_offset_rejects_negative() {
        use super::PageOffset;
        // The floor survives the move from `inner = u32` to `inner = i64` (#696).
        //
        // This is the test for the trap in that change: switching the inner type
        // WITHOUT declaring `min = 0` makes the sqlx bind gate green while silently
        // deleting the only guarantee the type carried — `u32` was the whole of it.
        // With `min = 0` declared, the bound is re-run by `FromStr`, the serde bridge,
        // and the sqlx `Decode` instead of being implied by a primitive that is widened
        // away at the boundary.
        assert!(
            "-1".parse::<PageOffset>().is_err(),
            "FromStr must reject -1"
        );
        assert!(
            serde_json::from_str::<PageOffset>("-1").is_err(),
            "the wire must reject -1"
        );
        assert!(
            PageOffset::try_from(-1_i64).is_err(),
            "TryFrom must reject -1"
        );
        // 0 is the first page and remains valid.
        assert_eq!(PageOffset::try_from(0_i64).map(i64::from), Ok(0));
    }

    #[test]
    fn row_limit_surface() {
        use super::RowLimit;
        // value()/From<Self> for i64, and trim.
        assert_eq!("10".parse::<RowLimit>().map(i64::from).ok(), Some(10));
        assert_eq!(
            "  100  ".parse::<RowLimit>().map(RowLimit::value).ok(),
            Some(100)
        );
        // The floor is 1: zero rows is a pointless query and a negative LIMIT is the
        // hazard this bound exists for (SQLite reads it as "no limit").
        for bad in ["0", "-1", "abc", "1.5"] {
            assert!(bad.parse::<RowLimit>().is_err(), "{bad} should reject");
        }
        assert!(
            "0".parse::<RowLimit>()
                .err()
                .is_some_and(|e| e.to_string().starts_with("fetch limit"))
        );
        // No `default` is declared — there is no sensible default row count — so no
        // `Default` impl exists to assert on. Display round-trips via the fixture.
        let r = crate::test_support::parse_row_limit("7");
        assert_eq!(r.value(), 7);
        assert_eq!(r.to_string().parse::<RowLimit>().ok(), Some(r));
        // serde: bare integer, round-trip, wire-rejection below the floor.
        assert_eq!(serde_json::to_string(&r).ok(), Some("7".to_owned()));
        assert_eq!(
            serde_json::from_str::<RowLimit>("42").map(i64::from).ok(),
            Some(42)
        );
        assert!(serde_json::from_str::<RowLimit>("0").is_err());
        assert!(serde_json::from_str::<RowLimit>("-1").is_err());
    }

    #[test]
    fn at_most_saturates_below_one() {
        use super::RowLimit;
        // A validated door: it cannot yield a value below the declared min, so a
        // caller's mistake becomes 1 rather than an invalid RowLimit.
        assert_eq!(RowLimit::at_most(0).value(), 1);
        assert_eq!(RowLimit::at_most(-5).value(), 1);
        assert_eq!(RowLimit::at_most(i64::MIN).value(), 1);
        // In range, it is the identity.
        assert_eq!(RowLimit::at_most(1).value(), 1);
        assert_eq!(RowLimit::at_most(100).value(), 100);
    }

    #[test]
    fn fetch_limit_is_page_plus_one() {
        // Across PageSize's whole range, the fetch limit is the page plus the one
        // extra row that proves a next page exists.
        for size in [
            PageSize::clamped(PageSize::MIN),
            PageSize::clamped(25),
            PageSize::clamped(PageSize::MAX),
        ] {
            // `i64::from` rather than an `as` cast: `PageSize`'s inner is `u32`, so the
            // widening is infallible and lossless.
            assert_eq!(
                size.fetch_limit().value(),
                i64::from(size.value()) + 1,
                "fetch_limit must be page_len + 1 for {size}"
            );
            // The probing variant is exactly one row more than the non-probing one —
            // stated as a relation between the two so neither can move alone.
            assert_eq!(
                size.exact_limit().value(),
                i64::from(size.value()),
                "exact_limit must be the page itself for {size}"
            );
            assert_eq!(
                size.fetch_limit().value(),
                size.exact_limit().value() + 1,
                "fetch_limit must be exact_limit + 1 for {size}"
            );
        }
    }

    #[test]
    fn has_more_is_the_inverse_of_fetch_limit() {
        // The two halves of the has-more convention must agree. If either drifts —
        // the `+1` in fetch_limit or the `>` in has_more — this fails.
        for size in [
            PageSize::clamped(PageSize::MIN),
            PageSize::clamped(25),
            PageSize::clamped(PageSize::MAX),
        ] {
            let page = size.page_len();
            let fetched = usize::try_from(size.fetch_limit().value()).unwrap_or(usize::MAX);

            // A full page and nothing more: no next page.
            assert!(!size.has_more(page), "a full page is not more for {size}");
            // Short of a full page: no next page.
            assert!(
                !size.has_more(page.saturating_sub(1)),
                "a partial page is not more for {size}"
            );
            // The over-fetched row arrived: there is a next page.
            assert!(
                size.has_more(fetched),
                "an over-fetched row set is more for {size}"
            );
            // `fetch_limit` is the SMALLEST count that proves a next page: one row fewer
            // must not. This is what couples the two halves — without it the assertions
            // above hold for any `+n`, so a `+2` drift would pass unnoticed (verified by
            // mutation).
            assert!(
                !size.has_more(fetched - 1),
                "one row below fetch_limit must not be more for {size}"
            );
        }
    }
}
