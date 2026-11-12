//! Closed storage roles for binding backup NDJSON cells during restore.
//!
//! Restore values are not application-domain values: their concrete SQL
//! representation is selected from the backup wire value and the live schema.
//! Keeping that dispatch closed prevents a primitive from becoming a general
//! restore bind sink.

use crate::backup::BackupError;

use super::ColumnInfo;

use common::time::UtcInstant;
use serde_json::Value;

/// The approved comparable population only; this is not catalog discovery or
/// an extensible export registry. `Some(true)` denotes a nullable selected cell.
pub(crate) fn comparable_instant_nullability(table: &str, column: &str) -> Option<bool> {
    match (table, column) {
        ("posts" | "post_revisions", "created_at" | "updated_at")
        | ("post_revisions", "captured_at")
        | ("sessions", "created_at" | "last_used_at")
        | ("invites" | "email_verifications" | "password_resets", "created_at" | "expires_at")
        | ("media" | "idempotency_keys" | "passkey_credentials", "created_at")
        | ("feed_events", "next_attempt_at" | "created_at")
        | ("feed_cache", "representation_modified_at" | "generated_at")
        | ("passkey_registration_ceremonies" | "passkey_authentication_ceremonies", "expires_at") => {
            Some(false)
        }
        ("posts" | "post_revisions", "published_at" | "deleted_at")
        | ("invites" | "email_verifications" | "password_resets", "used_at")
        | ("passkey_credentials", "last_used_at")
        | ("feed_events", "claimed_at" | "terminal_at" | "regenerated_at" | "pinged_at")
        | ("passkey_registration_ceremonies" | "passkey_authentication_ceremonies", "claimed_at") => {
            Some(true)
        }
        _ => None,
    }
}

/// Validate spelling before a native PG cast can erase it. Selected timestamps
/// enter the same closed bind sum as a checked instant, never as arbitrary text.
pub(crate) fn restore_table_bind_value(
    table: &str,
    column: &ColumnInfo,
    value: &Value,
) -> Result<RestoreBindValue, BackupError> {
    if let Some(nullable) = comparable_instant_nullability(table, &column.name) {
        if nullable && value.is_null() {
            return Ok(RestoreBindValue::Null);
        }
        if let Some(text) = value.as_str()
            && let Ok(instant) = text.parse::<UtcInstant>()
            && instant.to_string() == text
        {
            return Ok(RestoreBindValue::Instant(instant));
        }
        return Err(BackupError::InvalidBackup(format!(
            "noncanonical comparable instant in {table}.{}",
            column.name
        )));
    }
    restore_bind_value(column, value)
}

/// An INSERT's columns are selected once, so every row must have exactly that
/// same key set. Otherwise a later supplied cell could be silently discarded
/// before strict timestamp admission (including a valid publication instant).
pub(crate) fn validate_restore_row_columns(
    table: &str,
    rows: &[serde_json::Map<String, Value>],
) -> Result<(), BackupError> {
    let Some(first) = rows.first() else {
        return Ok(());
    };
    for (index, row) in rows.iter().enumerate().skip(1) {
        if row.len() != first.len() || row.keys().any(|key| !first.contains_key(key)) {
            return Err(BackupError::InvalidBackup(format!(
                "table {table} row {} has inconsistent row columns",
                index + 1
            )));
        }
    }
    Ok(())
}

pub(crate) const BINARY_WIRE_KEY: &str = "$jaunder_binary_hex";
/// A lossless binary cell from an NDJSON backup row.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct RestoreBinary(Vec<u8>);

/// Whether a live catalog column stores arbitrary binary data.
pub(crate) fn is_binary_column(column: &ColumnInfo) -> bool {
    matches!(column.type_name.as_str(), "bytea") || column.type_name.contains("blob")
}

/// Produces the deterministic JSON wire object used for a binary cell.
fn decode_binary(value: &Value) -> Result<RestoreBinary, BackupError> {
    let Value::Object(object) = value else {
        return Err(BackupError::InvalidBackup(
            "binary column must use the explicit binary wire object".to_owned(),
        ));
    };
    let Some(Value::String(hex)) = object.get(BINARY_WIRE_KEY) else {
        return Err(BackupError::InvalidBackup(
            "binary column has an invalid binary wire object".to_owned(),
        ));
    };
    if object.len() != 1 {
        return Err(BackupError::InvalidBackup(
            "binary wire object must contain only its binary payload".to_owned(),
        ));
    }
    if hex.len() % 2 != 0 {
        return Err(BackupError::InvalidBackup(
            "binary wire payload must contain an even number of hexadecimal digits".to_owned(),
        ));
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks_exact(2) {
        let high = hex_nibble(chunk[0]).ok_or_else(|| {
            BackupError::InvalidBackup("binary wire payload is not hexadecimal".to_owned())
        })?;
        let low = hex_nibble(chunk[1]).ok_or_else(|| {
            BackupError::InvalidBackup("binary wire payload is not hexadecimal".to_owned())
        })?;
        bytes.push((high << 4) | low);
    }
    Ok(RestoreBinary(bytes))
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Parses an NDJSON cell using the live target column type.
pub(crate) fn restore_bind_value(
    column: &ColumnInfo,
    value: &Value,
) -> Result<RestoreBindValue, BackupError> {
    if is_binary_column(column) {
        return if value.is_null() {
            Ok(RestoreBindValue::Null)
        } else {
            decode_binary(value).map(RestoreBindValue::Binary)
        };
    }
    Ok(RestoreBindValue::from_json(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_column() -> ColumnInfo {
        ColumnInfo {
            name: "payload".to_owned(),
            type_name: "blob".to_owned(),
        }
    }

    #[test]
    fn binary_wire_decodes_arbitrary_bytes() {
        let value = serde_json::json!({"$jaunder_binary_hex": "0000ff80"});

        let RestoreBindValue::Binary(binary) =
            restore_bind_value(&binary_column(), &value).expect("decode binary wire value")
        else {
            unreachable!("binary column decodes to a binary binding");
        };
        assert_eq!(binary.0, [0, 0, 0xff, 0x80]);
    }

    #[test]
    fn binary_wire_rejects_malformed_values() {
        for value in [
            serde_json::json!("00ff"),
            serde_json::json!({}),
            serde_json::json!({"$jaunder_binary_hex": "0"}),
            serde_json::json!({"$jaunder_binary_hex": "0z"}),
            serde_json::json!({"$jaunder_binary_hex": "zz"}),
            serde_json::json!({"$jaunder_binary_hex": "00", "extra": true}),
        ] {
            assert!(matches!(
                restore_bind_value(&binary_column(), &value),
                Err(BackupError::InvalidBackup(_))
            ));
        }
    }

    #[test]
    fn binary_null_remains_null() {
        assert!(matches!(
            restore_bind_value(&binary_column(), &serde_json::Value::Null),
            Ok(RestoreBindValue::Null)
        ));
    }

    #[test]
    fn text_resembling_binary_wire_remains_text() {
        let column = ColumnInfo {
            name: "payload".to_owned(),
            type_name: "text".to_owned(),
        };
        let value = serde_json::json!("{\"$jaunder_binary_hex\":\"00ff\"}");

        assert!(matches!(
            restore_bind_value(&column, &value),
            Ok(RestoreBindValue::Text(_))
        ));
    }
}

/// Every storage representation admitted by dynamic backup restore.
#[derive(Debug)]
pub(crate) enum RestoreBindValue {
    Null,
    Instant(UtcInstant),
    Boolean(RestoreBoolean),
    Integer(RestoreInteger),
    Real {
        value: Option<RestoreReal>,
        text: RestoreText,
    },
    Text(RestoreText),
    Json(RestoreJson),
    Binary(RestoreBinary),
}

/// A lossless textual cell from an NDJSON backup row.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct RestoreText(String);

impl RestoreText {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A boolean cell from an NDJSON backup row.
#[derive(Clone, Copy, Debug, macros::SqlxBridge)]
pub(crate) struct RestoreBoolean(bool);

impl RestoreBoolean {
    pub(crate) fn into_text(self) -> RestoreText {
        RestoreText::new(self.0.to_string())
    }
}

/// An integral cell from an NDJSON backup row.
#[derive(Clone, Copy, Debug, macros::SqlxBridge)]
pub(crate) struct RestoreInteger(i64);

impl RestoreInteger {
    pub(crate) fn into_text(self) -> RestoreText {
        RestoreText::new(self.0.to_string())
    }
}

/// A non-integral numeric cell from an NDJSON backup row.
#[derive(Clone, Copy, Debug, macros::SqlxBridge)]
pub(crate) struct RestoreReal(f64);

/// An object or array cell rendered as JSON text for an NDJSON backup row.
#[derive(Debug, macros::SqlxBridge)]
pub(crate) struct RestoreJson(String);

impl RestoreBindValue {
    pub(crate) fn from_json(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Boolean(RestoreBoolean(*value)),
            Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    Self::Integer(RestoreInteger(value))
                } else if value
                    .as_u64()
                    .and_then(|value| i64::try_from(value).ok())
                    .is_some()
                {
                    unreachable!("as_i64 already claims every u64 that fits in i64")
                } else {
                    Self::Real {
                        value: value.as_f64().map(RestoreReal),
                        text: RestoreText::new(value.to_string()),
                    }
                }
            }
            Value::String(value) => Self::Text(RestoreText::new(value.clone())),
            Value::Array(_) | Value::Object(_) => Self::Json(RestoreJson(value.to_string())),
        }
    }
}
