//! The two `PostgreSQL` object names the bootstrap path threads around: a role name and
//! a database name.
//!
//! They are **deliberately distinct types**, not one shared `PgIdentifier`. Sharing one
//! would leave `create_postgres_database_and_role` with two adjacent same-typed
//! parameters, which is the transposition hazard #693 exists to close — swapping a role
//! name with a database name compiles, and silently provisions the wrong pair.

use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// A validated, non-empty `PostgreSQL` role (user) name.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct PgRoleName(String);

/// Error returned when a role name fails its shape invariant (empty).
#[derive(Debug, Error)]
#[error("PostgreSQL role name must not be empty")]
pub struct InvalidPgRoleName;

impl FromStr for PgRoleName {
    type Err = InvalidPgRoleName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidPgRoleName);
        }
        Ok(PgRoleName(s.to_owned()))
    }
}

/// A validated, non-empty `PostgreSQL` database name.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct PgDatabaseName(String);

/// Error returned when a database name fails its shape invariant (empty).
#[derive(Debug, Error)]
#[error("PostgreSQL database name must not be empty")]
pub struct InvalidPgDatabaseName;

impl FromStr for PgDatabaseName {
    type Err = InvalidPgDatabaseName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(InvalidPgDatabaseName);
        }
        Ok(PgDatabaseName(s.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_name_accepts_non_empty() {
        assert!("jaunder".parse::<PgRoleName>().is_ok());
    }

    #[test]
    fn role_name_rejects_empty() {
        assert!("".parse::<PgRoleName>().is_err());
    }

    #[test]
    fn role_name_display_round_trips() {
        let r: PgRoleName = "jaunder".parse().unwrap();
        assert_eq!(r.to_string(), "jaunder");
    }

    #[test]
    fn database_name_accepts_non_empty() {
        assert!("jaunder_db".parse::<PgDatabaseName>().is_ok());
    }

    #[test]
    fn database_name_rejects_empty() {
        assert!("".parse::<PgDatabaseName>().is_err());
    }

    #[test]
    fn database_name_display_round_trips() {
        let d: PgDatabaseName = "jaunder_db".parse().unwrap();
        assert_eq!(d.to_string(), "jaunder_db");
    }
}
