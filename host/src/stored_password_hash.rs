use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A stored password hash, as held in the `users.password_hash` column.
///
/// Named `StoredPasswordHash` rather than `PasswordHash` because
/// [`argon2::PasswordHash`] is the parsed PHC structure; this is the opaque
/// stored string that Argon2 parses. Its secret newtype surface keeps hashes
/// out of display, serialization, and telemetry while retaining the `SQLx` bridge
/// at the host-owned persistence boundary.
///
/// The invariant is non-emptiness only. Argon2 remains the one authority for
/// deciding whether a stored string is a usable hash, during verification.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, sqlx)]
pub struct StoredPasswordHash(String);

/// Error returned when a stored password hash is empty.
#[derive(Debug, Error)]
#[error("stored password hash must not be empty")]
pub struct InvalidStoredPasswordHash;

impl FromStr for StoredPasswordHash {
    type Err = InvalidStoredPasswordHash;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidStoredPasswordHash);
        }
        Ok(Self(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_hash() {
        assert!(
            "$argon2id$v=19$m=19456,t=2,p=1$abc$def"
                .parse::<StoredPasswordHash>()
                .is_ok()
        );
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<StoredPasswordHash>().is_err());
    }

    #[test]
    fn accepts_a_malformed_hash_leaving_the_verdict_to_argon2() {
        assert!("not-a-bcrypt-hash".parse::<StoredPasswordHash>().is_ok());
    }

    #[test]
    fn debug_is_redacted() {
        let raw = "$argon2id$v=19$m=19456,t=2,p=1$abc$secretdigest";
        let hash: StoredPasswordHash = raw.parse().expect("non-empty hash");
        let debug = format!("{hash:?}");

        assert!(!debug.contains("secretdigest"));
        assert_eq!(debug, "StoredPasswordHash([redacted])");
    }

    #[test]
    fn as_ref_returns_original_value() {
        let raw = "$argon2id$v=19$m=19456,t=2,p=1$abc$def";
        let hash: StoredPasswordHash = raw.parse().expect("non-empty hash");
        assert_eq!(hash.as_ref(), raw);
    }
}
