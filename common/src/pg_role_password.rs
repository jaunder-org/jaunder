use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A validated, non-empty `PostgreSQL` role password.
///
/// Adopts the [`StrNewtype`] `secret` surface (ADR-0063 §2): a redacting `Debug` and
/// borrowed `AsRef<str>` access for the `CREATE ROLE … PASSWORD` statement, with no
/// `Display`, serde, `Deref`, owned-`String`, or `PartialEq` — so the credential cannot
/// be rendered, serialised, logged, or value-compared (ADR-0011). The `macros` crate is
/// the authoritative list of what `secret` emits.
///
/// Unlike [`SmtpPassword`](crate::smtp_password::SmtpPassword) this is **not** a stored
/// secret: it arrives as a clap argument and is consumed once during bootstrap, never
/// written to or decoded from a column, so it takes no `sqlx` bridge.
///
/// It exists because `create_postgres_database_and_role` took three adjacent `&str` with
/// this credential in the middle (#693) — every permutation compiled, and the middle one
/// was a secret.
#[derive(Clone, StrNewtype)]
#[str_newtype(secret)]
pub struct PgRolePassword(String);

/// Error returned when a `PostgreSQL` role password fails its shape invariant (empty).
#[derive(Debug, Error)]
#[error("PostgreSQL role password must not be empty")]
pub struct InvalidPgRolePassword;

impl FromStr for PgRolePassword {
    type Err = InvalidPgRolePassword;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidPgRolePassword);
        }
        Ok(PgRolePassword(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_non_empty() {
        assert!("s3cr3t".parse::<PgRolePassword>().is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<PgRolePassword>().is_err());
    }

    #[test]
    fn debug_is_redacted() {
        let raw = "hunter2-role-pw";
        let p: PgRolePassword = raw.parse().unwrap();
        let out = format!("{p:?}");
        assert!(!out.contains(raw));
        assert_eq!(out, "PgRolePassword([redacted])");
    }

    #[test]
    fn as_ref_returns_original_value() {
        let raw = "correct horse role";
        let p: PgRolePassword = raw.parse().unwrap();
        assert_eq!(p.as_ref(), raw);
    }
}
