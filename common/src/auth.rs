//! Pure, target-agnostic HTTP authentication helpers.
//!
//! These are the host-testable cores of `web`'s server-side request extractor:
//! decoding an `Authorization: Basic` header and comparing a Basic-auth
//! username against the authenticated session's user. They hold no leptos, wasm,
//! or wire-type tie, so they live here and are exercised by plain unit tests.

use crate::{token::RawToken, username::Username};

/// Parses an HTTP `Authorization: Basic` header value into `(username, token)`,
/// with both fields parsed into validated domain values at this decode boundary.
/// Returns `None` for non-Basic schemes, malformed/undecodable credentials, or a
/// username/token that fails validation.
#[must_use]
pub fn parse_basic_auth(header: &str) -> Option<(Username, RawToken)> {
    use base64::Engine as _;

    let rest = header.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(rest)
        .ok()?;
    let credentials = String::from_utf8(decoded).ok()?;
    let (username, password) = credentials.split_once(':')?;
    Some((username.parse().ok()?, password.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_auth_decodes_credentials() {
        let (username, token) =
            parse_basic_auth("Basic YWxpY2U6dG9rMTIz").expect("valid Basic credentials");
        assert_eq!(username, "alice");
        assert_eq!(token.as_ref(), "tok123");
    }

    #[test]
    fn parse_basic_auth_rejects_non_basic_and_malformed() {
        use base64::Engine as _;
        assert!(parse_basic_auth("Bearer abc").is_none());
        assert!(parse_basic_auth("Basic !!!notbase64!!!").is_none());
        // decodes but has no colon
        let no_colon = base64::engine::general_purpose::STANDARD.encode("nocolon");
        assert!(parse_basic_auth(&format!("Basic {no_colon}")).is_none());
    }

    #[test]
    fn parse_basic_auth_rejects_invalid_username() {
        use base64::Engine as _;
        // decodes to "a b:tok" — the space makes the username invalid, so the
        // whole credential is unrecognized rather than yielding a bad username.
        let bad_user = base64::engine::general_purpose::STANDARD.encode("a b:tok");
        assert!(parse_basic_auth(&format!("Basic {bad_user}")).is_none());
    }

    #[test]
    fn parse_basic_auth_rejects_invalid_token() {
        use base64::Engine as _;

        // decodes to "alice:not a token" — the space makes the token invalid.
        let bad_token = base64::engine::general_purpose::STANDARD.encode("alice:not a token");
        assert!(parse_basic_auth(&format!("Basic {bad_token}")).is_none());
    }
}
