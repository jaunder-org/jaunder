//! Bounded vocabulary for transient-data retention maintenance.

/// A database domain with independently owned retention policy.
#[derive(Clone, Copy, Debug)]
pub enum Domain {
    IdempotencyKeys,
    Invites,
    EmailVerifications,
    PasswordResets,
    FeedEvents,
}

impl Domain {
    /// Returns the bounded structured label shared by maintenance telemetry.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::IdempotencyKeys => "idempotency_keys",
            Self::Invites => "invites",
            Self::EmailVerifications => "email_verifications",
            Self::PasswordResets => "password_resets",
            Self::FeedEvents => "feed_events",
        }
    }
}

/// Outcome of one domain cleanup attempt.
#[derive(Clone, Copy, Debug)]
pub enum CleanupResult {
    Success,
    Failure,
}

impl CleanupResult {
    /// Returns the bounded structured label shared by maintenance telemetry.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_the_documented_bounded_vocabulary() {
        assert_eq!(Domain::IdempotencyKeys.label(), "idempotency_keys");
        assert_eq!(Domain::Invites.label(), "invites");
        assert_eq!(Domain::EmailVerifications.label(), "email_verifications");
        assert_eq!(Domain::PasswordResets.label(), "password_resets");
        assert_eq!(Domain::FeedEvents.label(), "feed_events");
        assert_eq!(CleanupResult::Success.label(), "success");
        assert_eq!(CleanupResult::Failure.label(), "failure");
    }
}
