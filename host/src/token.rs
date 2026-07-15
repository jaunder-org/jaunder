//! Server-only session/opaque-token machinery: minting a [`RawToken`] and
//! hashing it into a [`TokenHash`].
//!
//! The two types live in `common::token` because both cross the server-fn wire;
//! the RNG/SHA-256 here stays in `host` so this server-only crypto never enters
//! the wasm client bundle. [`hash`] is the **sole** `RawToken -> TokenHash`
//! bridge — there is deliberately no conversion the other way.

use std::fmt;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

use common::token::{RawToken, TokenHash};

/// 32 cryptographically random bytes — the shared source for both a fresh token
/// and the digest computed over its bytes.
fn random_bytes() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

/// Mints a fresh secret token: 32 cryptographically random bytes encoded as
/// base64url without padding (43 characters).
#[must_use]
pub fn generate() -> RawToken {
    RawToken::from_generated(URL_SAFE_NO_PAD.encode(random_bytes()))
}

/// A raw token whose characters are in-alphabet but whose length is not a valid
/// base64url encoding, so it cannot be decoded and hashed.
#[derive(Debug)]
pub struct TokenHashError;

impl fmt::Display for TokenHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("raw token is not valid base64url and cannot be hashed")
    }
}

impl std::error::Error for TokenHashError {}

/// Hashes a [`RawToken`] with SHA-256 and returns the base64url digest as a
/// [`TokenHash`]. This is the **only** path from a `RawToken` to a `TokenHash`.
///
/// # Errors
///
/// Returns [`TokenHashError`] if the raw token is not decodable base64url — a
/// value that reached us from an untrusted source (a cookie or header) with a
/// valid charset but an invalid length.
pub fn hash(token: &RawToken) -> Result<TokenHash, TokenHashError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(token.as_ref())
        .map_err(|_| TokenHashError)?;
    let digest = Sha256::digest(&bytes);
    Ok(TokenHash::from_digest(URL_SAFE_NO_PAD.encode(digest)))
}

/// Mints a token and its stored hash in one step. Infallible: it hashes the raw
/// random bytes directly, so there is no decode step that could fail.
#[must_use]
pub fn generate_hashed() -> (RawToken, TokenHash) {
    let bytes = random_bytes();
    let raw = RawToken::from_generated(URL_SAFE_NO_PAD.encode(bytes));
    let token_hash = TokenHash::from_digest(URL_SAFE_NO_PAD.encode(Sha256::digest(bytes)));
    (raw, token_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_produces_distinct_parseable_tokens() {
        let a = generate();
        let b = generate();
        assert_ne!(a.as_ref(), b.as_ref());
        assert!(!a.as_ref().is_empty());
    }

    #[test]
    fn hash_matches_legacy_vector() {
        // Golden vector: the legacy SHA-256-over-base64url-decoded-bytes hashing
        // (the former `storage::auth::hash_token`) applied to "dGVzdC10b2tlbg".
        // Pins byte-identical hashing so existing sessions stay valid.
        let raw = RawToken::try_from("dGVzdC10b2tlbg".to_string()).unwrap();
        let hashed = hash(&raw).unwrap();
        assert_eq!(
            hashed.as_ref(),
            "TF3Jt3CJBfd_Xl0WMWtd-0JeaMsybc1VqGDpCncHAx4"
        );
    }

    #[test]
    fn hash_rejects_undecodable_token() {
        // In-alphabet but a base64url length that cannot decode (1 char).
        let raw = RawToken::try_from("a".to_string()).unwrap();
        assert!(hash(&raw).is_err());
    }

    #[test]
    fn token_hash_error_displays() {
        assert!(!TokenHashError.to_string().is_empty());
    }

    #[test]
    fn generate_hashed_pair_is_consistent() {
        let (raw, token_hash) = generate_hashed();
        assert_eq!(hash(&raw).unwrap(), token_hash);
    }
}
