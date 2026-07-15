//! Opaque-token value helpers shared across the token family.
//!
//! All of jaunder's opaque tokens — invite codes, the session `RawToken`, the
//! password-reset token — are base64url-no-pad strings from the same generator
//! (`host::token::generate`). Their shape validation is therefore one
//! token-general rule, kept here so every token newtype's `FromStr` delegates to a
//! single source of truth rather than re-deriving it.

use std::str::FromStr;

use macros::StrNewtype;
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

/// A raw, secret bearer token — a freshly minted session token, password-reset
/// token, or email-verification token (all from the same generator). Whoever
/// holds it *is* the credential, so it is secret-bearing per ADR-0063
/// (`#[str_newtype(secret, serde)]`): a redacting `Debug` keeps it out of spans
/// and logs (ADR-0011), and the only surfaces are `AsRef<str>`, `TryFrom<String>`,
/// and the validating serde bridge (it crosses the wire in the app-password
/// response). Hashing (`host::token::hash`) is the sole path to a [`TokenHash`];
/// there is deliberately no `PartialEq`/`Display`/`Deref` and no conversion the
/// other way, so a raw token can never be compared with, or mistaken for, its hash.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, serde)]
pub struct RawToken(String);

impl FromStr for RawToken {
    type Err = InvalidTokenShape;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_shape(s)?;
        Ok(RawToken(s.to_owned()))
    }
}

impl RawToken {
    /// Wraps a token freshly produced by the server-side generator
    /// (`host::token::generate`) **without** re-validating it — the generator's
    /// output is base64url by construction, so the shape check is redundant. This
    /// is the single trusted-construction door (mirroring
    /// [`crate::render::RenderedHtml::from_trusted`]); **untrusted** input (a
    /// cookie, a header, the wire) must go through [`FromStr`]/`TryFrom`, which
    /// validate.
    #[must_use]
    pub fn from_generated(token: String) -> Self {
        RawToken(token)
    }
}

/// The SHA-256 hash of a [`RawToken`] — what the `sessions` / `password_resets` /
/// `email_verifications` tables store and what lookups and revocation key on. Not
/// secret (it is a hash, compared and rendered in the session-management UI and
/// crossing the wire in `SessionInfo`), so it carries the full non-secret trailer
/// plus std `PartialEq`/`Eq`/`Hash` for `TokenHash == TokenHash`. A `TokenHash` is
/// a distinct type from [`RawToken`], so passing one where the other is expected
/// does not compile.
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
pub struct TokenHash(String);

impl FromStr for TokenHash {
    type Err = InvalidTokenShape;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_shape(s)?;
        Ok(TokenHash(s.to_owned()))
    }
}

impl TokenHash {
    /// Wraps a SHA-256 digest produced by `host::token::hash` **without**
    /// re-validating it — a base64url-encoded digest is well-formed by
    /// construction. The trusted-construction door; a hash arriving from an
    /// **untrusted** source (a revoke form field, the wire) must go through
    /// [`FromStr`]/`TryFrom`, which validate.
    #[must_use]
    pub fn from_digest(digest: String) -> Self {
        TokenHash(digest)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn raw_token_parses_valid_and_rejects_empty_and_bad_charset() {
        assert!(RawToken::from_str("abcABC012-_").is_ok());
        assert!(RawToken::from_str("").is_err());
        assert!(RawToken::from_str("has space").is_err());
        assert!(RawToken::from_str("plus+code").is_err());
    }

    #[test]
    fn raw_token_debug_redacts_body() {
        let raw = RawToken::from_str("SecretBody123").unwrap();
        let shown = format!("{raw:?}");
        assert!(shown.contains("[redacted]"));
        assert!(!shown.contains("SecretBody123"));
    }

    #[test]
    fn token_hash_parses_and_self_equality_holds() {
        let a = TokenHash::from_str("abcABC012-_").unwrap();
        let b = TokenHash::from_str("abcABC012-_").unwrap();
        assert_eq!(a, b); // std PartialEq<Self>
        assert!(TokenHash::from_str("").is_err());
    }

    #[test]
    fn token_hash_serde_roundtrips() {
        let h = TokenHash::from_str("abcABC012-_").unwrap();
        let json = serde_json::to_string(&h).unwrap();
        let back: TokenHash = serde_json::from_str(&json).unwrap();
        assert_eq!(h, back);
    }

    #[test]
    fn trusted_constructors_wrap_without_validation() {
        // The trusted doors skip validate_shape (the caller asserts provenance).
        assert_eq!(RawToken::from_generated("abc".to_string()).as_ref(), "abc");
        assert_eq!(TokenHash::from_digest("xyz".to_string()).as_ref(), "xyz");
    }

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
