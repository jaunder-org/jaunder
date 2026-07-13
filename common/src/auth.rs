//! Pure, target-agnostic HTTP authentication helpers.
//!
//! These are the host-testable cores of `web`'s server-side request extractor:
//! decoding an `Authorization: Basic` header and comparing a Basic-auth
//! username against the authenticated session's user. They hold no leptos, wasm,
//! or wire-type tie, so they live here and are exercised by plain unit tests.

use crate::username::Username;

/// Parses an HTTP `Authorization: Basic` header value into `(username, password)`,
/// with the username parsed into a validated [`Username`] at this decode boundary.
/// Returns `None` for non-Basic schemes, malformed/undecodable credentials, or a
/// username that fails validation.
#[must_use]
pub fn parse_basic_auth(header: &str) -> Option<(Username, String)> {
    use base64::Engine as _;

    let rest = header.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(rest)
        .ok()?;
    let credentials = String::from_utf8(decoded).ok()?;
    let (username, password) = credentials.split_once(':')?;
    Some((username.parse().ok()?, password.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_auth_decodes_credentials() {
        // base64("alice:tok123") == "YWxpY2U6dG9rMTIz"
        assert_eq!(
            parse_basic_auth("Basic YWxpY2U6dG9rMTIz"),
            Some(("alice".parse().unwrap(), "tok123".to_string()))
        );
    }

    #[test]
    fn parse_basic_auth_rejects_non_basic_and_malformed() {
        use base64::Engine as _;
        assert_eq!(parse_basic_auth("Bearer abc"), None);
        assert_eq!(parse_basic_auth("Basic !!!notbase64!!!"), None);
        // decodes but has no colon
        let no_colon = base64::engine::general_purpose::STANDARD.encode("nocolon");
        assert_eq!(parse_basic_auth(&format!("Basic {no_colon}")), None);
    }

    #[test]
    fn parse_basic_auth_rejects_invalid_username() {
        use base64::Engine as _;
        // decodes to "a b:tok" — the space makes the username invalid, so the
        // whole credential is unrecognized rather than yielding a bad username.
        let bad_user = base64::engine::general_purpose::STANDARD.encode("a b:tok");
        assert_eq!(parse_basic_auth(&format!("Basic {bad_user}")), None);
    }
}
