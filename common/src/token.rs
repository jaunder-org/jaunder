//! Opaque-token value helpers shared across the token family.
//!
//! All of jaunder's opaque tokens — invite codes, the session `RawToken`, the
//! password-reset token — are base64url-no-pad strings from the same generator
//! (`storage::auth::generate_token`). Their shape validation is therefore one
//! token-general rule, kept here so every token newtype's `FromStr` delegates to a
//! single source of truth rather than re-deriving it.

use thiserror::Error;

/// Error when a string is not a syntactically valid opaque token.
#[derive(Debug, Error)]
#[error("token must be non-empty and use only base64url characters ([A-Za-z0-9_-])")]
pub struct InvalidTokenShape;

/// The single source of truth for opaque-token shape: non-empty and the
/// base64url-no-pad charset (`A-Z a-z 0-9 - _`). Deliberately **not** length-pinned,
/// so it is not coupled to any particular token size.
///
/// # Errors
///
/// Returns [`InvalidTokenShape`] when `s` is empty or contains a non-base64url
/// character.
pub fn validate_shape(s: &str) -> Result<(), InvalidTokenShape> {
    if s.is_empty()
        || !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(InvalidTokenShape);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_base64url() {
        assert!(validate_shape("abcABC012-_defDEF345ghiGHI678jklJKL901mnoPQ").is_ok());
    }

    #[test]
    fn rejects_empty_and_out_of_alphabet() {
        assert!(validate_shape("").is_err());
        assert!(validate_shape("has space").is_err());
        assert!(validate_shape("plus+code").is_err()); // base64 std, not url
        assert!(validate_shape("slash/code").is_err());
        assert!(validate_shape("at@code").is_err());
    }
}
