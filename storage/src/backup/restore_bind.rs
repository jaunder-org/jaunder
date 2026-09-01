//! Closed storage roles for binding backup NDJSON cells during restore.
//!
//! Restore values are not application-domain values: their concrete SQL
//! representation is selected from the backup wire value and the live schema.
//! Keeping that dispatch closed prevents a primitive from becoming a general
//! restore bind sink.

use serde_json::Value;

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
}

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
