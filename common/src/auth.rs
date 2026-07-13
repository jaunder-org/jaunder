//! Pure, target-agnostic HTTP authentication helpers.
//!
//! These are the host-testable cores of `web`'s server-side request extractor:
//! decoding an `Authorization: Basic` header and comparing a Basic-auth
//! username against the authenticated session's user. They hold no leptos, wasm,
//! or wire-type tie, so they live here and are exercised by plain unit tests.

use crate::username::Username;

/// Parses an HTTP `Authorization: Basic` header value into `(username, password)`.
/// Returns `None` for non-Basic schemes or malformed/undecodable credentials.
#[must_use]
pub fn parse_basic_auth(header: &str) -> Option<(String, String)> {
    use base64::Engine as _;

    let rest = header.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(rest)
        .ok()?;
    let credentials = String::from_utf8(decoded).ok()?;
    let (username, password) = credentials.split_once(':')?;
    Some((username.to_string(), password.to_string()))
}

/// Whether an app-password (Basic auth) request authenticated as the user it
/// claimed: the authenticated session's username must equal the supplied Basic
/// username. Cookie/Bearer requests carry no expected username and are handled
/// by the caller.
#[must_use]
pub fn basic_username_matches(authenticated: &Username, expected: &str) -> bool {
    *authenticated == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_auth_decodes_credentials() {
        // base64("alice:tok123") == "YWxpY2U6dG9rMTIz"
        assert_eq!(
            parse_basic_auth("Basic YWxpY2U6dG9rMTIz"),
            Some(("alice".to_string(), "tok123".to_string()))
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
    fn basic_username_matches_true_on_match() {
        let user: Username = "alice".parse().unwrap();
        assert!(basic_username_matches(&user, "alice"));
    }

    #[test]
    fn basic_username_matches_false_on_mismatch() {
        let user: Username = "alice".parse().unwrap();
        assert!(!basic_username_matches(&user, "mallory"));
    }
}
