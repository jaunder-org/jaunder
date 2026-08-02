use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A stored password hash, as held in the `users.password_hash` column.
///
/// Named `StoredPasswordHash` rather than `PasswordHash` because
/// [`argon2::PasswordHash`] already means something different — the *parsed* PHC
/// structure. This is the opaque stored string that argon2 parses.
///
/// Adopts the [`StrNewtype`] `secret` surface (ADR-0063 §2): a redacting `Debug` and
/// borrowed `AsRef<str>` access only, with no `Display`, serde, `Deref`, owned-`String`,
/// or `PartialEq`. A hash is not a password, but it is secret-bearing — leaking one
/// enables offline cracking — so ADR-0011's no-secrets-in-telemetry rule applies.
///
/// It **is** a stored secret, so unlike
/// [`PgRolePassword`](crate::pg_role_password::PgRolePassword) it carries the sqlx bridge
/// (`secret, sqlx`, as [`SmtpPassword`](crate::smtp_password::SmtpPassword) does) and the
/// `password_hash` column decodes straight into it.
///
/// # Why the invariant is only non-emptiness
///
/// It would be tempting to validate the PHC format here, since #438's principle is that
/// a garbage stored value should be rejected at the query boundary. Deliberately not:
/// argon2's parser is the single definition of "a hash we can verify against", and
/// duplicating a weaker version of it here would fork that definition across two places
/// that could disagree. A malformed hash still fails — at
/// `verify_password`, which is where argon2 already decides — and surfaces as the same
/// `UserAuthError::Internal` it does today.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret, sqlx)]
pub struct StoredPasswordHash(String);

/// Error returned when a stored password hash fails its shape invariant (empty).
#[derive(Debug, Error)]
#[error("stored password hash must not be empty")]
pub struct InvalidStoredPasswordHash;

impl FromStr for StoredPasswordHash {
    type Err = InvalidStoredPasswordHash;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidStoredPasswordHash);
        }
        Ok(StoredPasswordHash(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_hash() {
        assert!("$argon2id$v=19$m=19456,t=2,p=1$abc$def"
            .parse::<StoredPasswordHash>()
            .is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<StoredPasswordHash>().is_err());
    }

    /// A malformed hash is accepted *here* on purpose — argon2 owns that verdict.
    #[test]
    fn accepts_a_malformed_hash_leaving_the_verdict_to_argon2() {
        assert!("not-a-bcrypt-hash".parse::<StoredPasswordHash>().is_ok());
    }

    #[test]
    fn debug_is_redacted() {
        let raw = "$argon2id$v=19$m=19456,t=2,p=1$abc$secretdigest";
        let h: StoredPasswordHash = raw.parse().unwrap();
        let out = format!("{h:?}");
        assert!(!out.contains("secretdigest"));
        assert_eq!(out, "StoredPasswordHash([redacted])");
    }

    #[test]
    fn as_ref_returns_original_value() {
        let raw = "$argon2id$v=19$m=19456,t=2,p=1$abc$def";
        let h: StoredPasswordHash = raw.parse().unwrap();
        assert_eq!(h.as_ref(), raw);
    }
}
