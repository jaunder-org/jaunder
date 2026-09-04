//! The recovery phase currently owned by a feed event.

/// The independently retryable stage of a durable feed-event lifecycle.
///
/// The claim status remains separate: it records whether a worker currently owns
/// the row, while this value decides which retry budget, diagnostic, and
/// dead-letter category apply.
#[macros::text_enum(
    sqlx,
    error = InvalidFeedEventPhase,
    message = "feed event phase must be \"regeneration\" or \"publication\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[strum(serialize_all = "snake_case")]
pub enum FeedEventPhase {
    Regeneration,
    Publication,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn every_token_round_trips() {
        for phase in [FeedEventPhase::Regeneration, FeedEventPhase::Publication] {
            let token: &str = phase.as_ref();
            assert_eq!(FeedEventPhase::from_str(token), Ok(phase), "for {token}");
        }
    }
}
