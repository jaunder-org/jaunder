//! Closed storage roles for binding backup NDJSON cells during restore.
//!
//! Restore values are not application-domain values: their concrete SQL
//! representation is selected from the backup wire value and the live schema.
//! Keeping that dispatch closed prevents a primitive from becoming a general
//! restore bind sink.

use crate::backup::BackupError;

use super::ColumnInfo;

use serde_json::Value;

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
            panic!("binary column must yield a binary binding"); // cov:ignore this assertion only runs if the tested success contract fails.
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
