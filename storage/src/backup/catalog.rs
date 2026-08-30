//! Private decode roles for database catalog and backup wire values.
//!
//! These names distinguish storage mechanics which share a physical `TEXT` or
//! integer representation but must not be mistaken for application-domain text.

use std::fmt;
use std::str::FromStr;

use serde::de::{Deserializer as _, IgnoredAny, MapAccess, Visitor};

/// A table name supplied by a database catalog.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct CatalogTableName(String);

impl CatalogTableName {
    pub(crate) fn into_inner(self) -> String {
        self.0
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}
/// A column name supplied by a database catalog.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct CatalogColumnName(String);

impl CatalogColumnName {
    pub(crate) fn into_inner(self) -> String {
        self.0
    }
}

/// A database-specific declared column type supplied by a catalog.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct CatalogTypeName(String);

impl CatalogTypeName {
    pub(crate) fn into_inner(self) -> String {
        self.0
    }
}

/// A catalog DDL definition retained verbatim for schema fingerprinting.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct CatalogDefinition(String);

impl CatalogDefinition {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// `PostgreSQL`'s closed `information_schema.is_nullable` token.
#[derive(Debug, macros::SqlxBridge)]
#[sqlx_bridge(text)]
pub(crate) struct CatalogNullability(String);

impl FromStr for CatalogNullability {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "YES" | "NO" => Ok(Self(value.to_owned())),
            _ => Err("catalog nullability must be YES or NO"),
        }
    }
}

impl CatalogNullability {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// A nonnegative `SQLx` migration version.
#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 0,
    error = "migration version must be a non-negative integer"
)]
pub(crate) struct MigrationVersion(i64);

impl MigrationVersion {
    pub(crate) const fn into_i64(self) -> i64 {
        self.0
    }
}

/// One JSON object rendered by the database for a backup NDJSON row.
#[derive(Debug, macros::SqlxBridge)]
#[sqlx_bridge(text)]
pub(crate) struct BackupRowJson(String);

/// Validates a single JSON object without materializing its keys or values.
struct BackupRowJsonVisitor;

impl<'de> Visitor<'de> for BackupRowJsonVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(())
    }
}

impl FromStr for BackupRowJson {
    type Err = serde_json::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut deserializer = serde_json::Deserializer::from_str(value);
        deserializer.deserialize_map(BackupRowJsonVisitor)?;
        deserializer.end()?;
        Ok(Self(value.to_owned()))
    }
}

impl BackupRowJson {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_nullability_rejects_unknown_tokens() {
        assert!("YES".parse::<CatalogNullability>().is_ok());
        assert!("NO".parse::<CatalogNullability>().is_ok());
        assert!("maybe".parse::<CatalogNullability>().is_err());
    }

    #[test]
    fn backup_row_json_requires_an_object_and_preserves_its_wire_text() {
        let row = r#"{"id": 1, "value": "preserved"}"#.parse::<BackupRowJson>().unwrap();
        assert_eq!(row.as_bytes(), br#"{"id": 1, "value": "preserved"}"#);
        assert!("[1, 2]".parse::<BackupRowJson>().is_err());
        assert!("1".parse::<BackupRowJson>().is_err());
        assert!("true".parse::<BackupRowJson>().is_err());
        assert!("not json".parse::<BackupRowJson>().is_err());
        assert!(r#"{"id": 1} trailing"#.parse::<BackupRowJson>().is_err());
    }
    #[test]
    fn migration_versions_reject_negative_values() {
        assert_eq!(MigrationVersion::try_from(0).unwrap().into_i64(), 0);
        assert!(MigrationVersion::try_from(-1).is_err());
    }
}
