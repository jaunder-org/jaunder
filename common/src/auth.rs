//! Pure, target-agnostic HTTP authentication helpers.
//!
//! These are the host-testable cores of `web`'s server-side request extractor:
//! decoding an `Authorization: Basic` header and comparing a Basic-auth
//! username against the authenticated session's user. They hold no leptos, wasm,
//! or wire-type tie, so they live here and are exercised by plain unit tests.

use thiserror::Error;

use crate::{token::RawToken, username::Username};

/// A validated Basic-auth credential.
#[derive(Debug, Clone)]
pub struct BasicAuthCredential {
    pub username: Username,
    pub token: RawToken,
}

/// Error returned when an `Authorization` header is not a valid Basic credential.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BasicAuthParseError {
    #[error("authorization scheme is not Basic")]
    NotBasic,
    #[error("Basic authorization credentials are malformed")]
    Malformed,
}

/// Parses an HTTP `Authorization: Basic` header value into validated domain values.
///
/// # Errors
///
/// Returns [`BasicAuthParseError::NotBasic`] when the header uses another scheme,
/// and [`BasicAuthParseError::Malformed`] when Basic credentials fail base64,
/// UTF-8, separator, username, or token validation.
pub fn parse_basic_auth(header: &str) -> Result<BasicAuthCredential, BasicAuthParseError> {
    use base64::Engine as _;

    let rest = header
        .strip_prefix("Basic ")
        .ok_or(BasicAuthParseError::NotBasic)?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(rest)
        .map_err(|_| BasicAuthParseError::Malformed)?;
    let credentials = String::from_utf8(decoded).map_err(|_| BasicAuthParseError::Malformed)?;
    let (username, password) = credentials
        .split_once(':')
        .ok_or(BasicAuthParseError::Malformed)?;
    Ok(BasicAuthCredential {
        username: username
            .parse()
            .map_err(|_| BasicAuthParseError::Malformed)?,
        token: password
            .parse()
            .map_err(|_| BasicAuthParseError::Malformed)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_auth_decodes_credentials() {
        let credential =
            parse_basic_auth("Basic YWxpY2U6dG9rMTIz").expect("valid Basic credentials");
        assert_eq!(credential.username, "alice");
        assert_eq!(credential.token.as_ref(), "tok123");
    }

    #[test]
    fn parse_basic_auth_reports_non_basic_and_malformed() {
        use base64::Engine as _;
        assert!(matches!(
            parse_basic_auth("Bearer abc"),
            Err(BasicAuthParseError::NotBasic)
        ));
        assert!(matches!(
            parse_basic_auth("Basic !!!notbase64!!!"),
            Err(BasicAuthParseError::Malformed)
        ));
        // decodes but has no colon
        let no_colon = base64::engine::general_purpose::STANDARD.encode("nocolon");
        assert!(matches!(
            parse_basic_auth(&format!("Basic {no_colon}")),
            Err(BasicAuthParseError::Malformed)
        ));
    }

    #[test]
    fn parse_basic_auth_rejects_invalid_username() {
        use base64::Engine as _;
        // decodes to "a b:tok" — the space makes the username invalid.
        let bad_user = base64::engine::general_purpose::STANDARD.encode("a b:tok");
        assert!(matches!(
            parse_basic_auth(&format!("Basic {bad_user}")),
            Err(BasicAuthParseError::Malformed)
        ));
    }

    #[test]
    fn parse_basic_auth_rejects_invalid_token() {
        use base64::Engine as _;

        // decodes to "alice:not a token" — the space makes the token invalid.
        let bad_token = base64::engine::general_purpose::STANDARD.encode("alice:not a token");
        assert!(matches!(
            parse_basic_auth(&format!("Basic {bad_token}")),
            Err(BasicAuthParseError::Malformed)
        ));
    }
}
