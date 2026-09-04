//! The claim-lease state of a feed event.

/// Where a `feed_events` row is in the claim-lease cycle: `pending` → `claimed`
/// → `done` | `failed`. The independently retryable recovery stage is carried
/// separately by [`super::FeedEventPhase`].
///
/// A closed string enum (`#[text_enum]`, ADR-0075 as amended by #746) stored as its
/// `snake_case` token in the `status` TEXT column, so `sqlx` decodes it as itself and an
/// unrecognised token is a `ColumnDecode` error rather than a plausible-looking guess.
///
/// It lives here rather than beside the queue in `storage` for three reasons, all of them
/// about where the ceremony is reachable (#728): `storage` has no `macros` dependency, the
/// bridge is `#[cfg(feature = "sqlx")]` evaluated in the *consuming* crate and `storage`
/// declares no such feature (so the attribute would emit nothing and the decode would fail
/// to type-check with no indication why), and every other stored enum already lives in
/// `common`. `server`'s feed worker imports it too, so it crosses the crate boundary
/// regardless.
#[macros::text_enum(
    sqlx,
    error = InvalidFeedEventStatus,
    message = "feed event status must be \"pending\", \"claimed\", \"done\", or \"failed\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[strum(serialize_all = "snake_case")]
pub enum FeedEventStatus {
    Pending,
    Claimed,
    Done,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn every_token_round_trips() {
        for status in [
            FeedEventStatus::Pending,
            FeedEventStatus::Claimed,
            FeedEventStatus::Done,
            FeedEventStatus::Failed,
        ] {
            let token: &str = status.as_ref();
            assert_eq!(FeedEventStatus::from_str(token), Ok(status), "for {token}");
        }
    }

    #[test]
    fn an_unrecognised_token_is_rejected_rather_than_coerced() {
        // The behaviour this type exists to get right (#728): a value the code fails
        // to understand must not be quietly coerced into a plausible one.
        let err = FeedEventStatus::from_str("???").expect_err("must not parse");
        assert!(
            err.to_string().contains("feed event status must be"),
            "the named error carries the domain message: {err}"
        );
    }
}
