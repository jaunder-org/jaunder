//! Email vertical — pure, host-tested helpers extracted from the UI (ADR-0070
//! §6): status-line formatting and verification-token parsing.

use crate::error::WebError;
use common::email::Email;
use common::token::RawToken;

/// Formats the account's current email-verification status for display.
pub fn email_status_line(email: Option<&Email>, verified: bool) -> String {
    match (email, verified) {
        (Some(e), true) => format!("{e} (verified)"),
        (Some(e), false) => format!("{e} (unverified)"),
        (None, _) => "No email set".to_string(),
    }
}

/// Parses a raw verification token, mapping a malformed value to a client-side
/// validation error (ADR-0065 pre-validation) rather than a server round-trip.
///
/// # Errors
///
/// Returns a `WebError::validation` if `raw` is not a well-formed token.
pub fn parse_verification_token(raw: &str) -> Result<RawToken, WebError> {
    raw.parse()
        .map_err(|_| WebError::validation("invalid verification token"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_email;

    #[test]
    fn status_line_verified() {
        assert_eq!(
            email_status_line(Some(&parse_email("a@b.com")), true),
            "a@b.com (verified)"
        );
    }

    #[test]
    fn status_line_unverified() {
        assert_eq!(
            email_status_line(Some(&parse_email("a@b.com")), false),
            "a@b.com (unverified)"
        );
    }

    #[test]
    fn status_line_none() {
        assert_eq!(email_status_line(None, false), "No email set");
        assert_eq!(email_status_line(None, true), "No email set");
    }

    #[test]
    fn parse_token_valid() {
        // `"abcABC012-_"` is the crate's own valid RawToken fixture (base64url,
        // no-pad, not length-pinned — common/src/token.rs).
        assert!(parse_verification_token("abcABC012-_").is_ok());
    }

    #[test]
    fn parse_token_invalid() {
        let err = parse_verification_token("not a token").unwrap_err();
        assert!(err.to_string().contains("invalid verification token"));
    }
}
