//! Closed wire contract for swallowed browser-error diagnostics.
//!
//! The event carries only reviewed enum tokens. It cannot contain source text,
//! URLs, identifiers, form values, or request bodies, which keeps the untrusted
//! diagnostics intake bounded before any server-side policy is applied.

use serde::{Deserialize, Deserializer, Serialize};

/// Wire version accepted by the client-telemetry intake.
pub const CLIENT_TELEMETRY_VERSION: u8 = 1;

/// Broad class of an unexpected browser failure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientErrorKind {
    Network,
    Storage,
    Decode,
    Dialog,
    FormData,
    Internal,
}

/// Static operation at which an unexpected browser failure was swallowed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientErrorContext {
    ThemeStorageRead,
    ThemeStorageWrite,
    SessionMarkerRead,
    SessionMarkerWrite,
    SessionMarkerRemove,
    ProjectorSeedDecode,
    PublishConfirm,
    DeleteConfirm,
    MediaFormData,
}

/// Bounded classification of the underlying browser failure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientSourceKind {
    StorageUnavailable,
    StorageOperation,
    InvalidSeed,
    DialogUnavailable,
    FormDataCreate,
    FormDataAppend,
}

/// One versioned, closed client diagnostic event.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientTelemetryEvent {
    #[serde(deserialize_with = "deserialize_version")]
    pub version: u8,
    pub kind: ClientErrorKind,
    pub context: ClientErrorContext,
    pub source_kind: ClientSourceKind,
}

fn deserialize_version<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let version = u8::deserialize(deserializer)?;
    if version == CLIENT_TELEMETRY_VERSION {
        Ok(version)
    } else {
        Err(serde::de::Error::custom(
            "unsupported client telemetry version",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [(ClientErrorKind, &str); 6] = [
        (ClientErrorKind::Network, "network"),
        (ClientErrorKind::Storage, "storage"),
        (ClientErrorKind::Decode, "decode"),
        (ClientErrorKind::Dialog, "dialog"),
        (ClientErrorKind::FormData, "form_data"),
        (ClientErrorKind::Internal, "internal"),
    ];
    const CONTEXTS: [(ClientErrorContext, &str); 9] = [
        (ClientErrorContext::ThemeStorageRead, "theme_storage_read"),
        (ClientErrorContext::ThemeStorageWrite, "theme_storage_write"),
        (ClientErrorContext::SessionMarkerRead, "session_marker_read"),
        (
            ClientErrorContext::SessionMarkerWrite,
            "session_marker_write",
        ),
        (
            ClientErrorContext::SessionMarkerRemove,
            "session_marker_remove",
        ),
        (
            ClientErrorContext::ProjectorSeedDecode,
            "projector_seed_decode",
        ),
        (ClientErrorContext::PublishConfirm, "publish_confirm"),
        (ClientErrorContext::DeleteConfirm, "delete_confirm"),
        (ClientErrorContext::MediaFormData, "media_form_data"),
    ];
    const SOURCES: [(ClientSourceKind, &str); 6] = [
        (ClientSourceKind::StorageUnavailable, "storage_unavailable"),
        (ClientSourceKind::StorageOperation, "storage_operation"),
        (ClientSourceKind::InvalidSeed, "invalid_seed"),
        (ClientSourceKind::DialogUnavailable, "dialog_unavailable"),
        (ClientSourceKind::FormDataCreate, "form_data_create"),
        (ClientSourceKind::FormDataAppend, "form_data_append"),
    ];

    fn event() -> ClientTelemetryEvent {
        ClientTelemetryEvent {
            version: CLIENT_TELEMETRY_VERSION,
            kind: ClientErrorKind::Internal,
            context: ClientErrorContext::MediaFormData,
            source_kind: ClientSourceKind::FormDataAppend,
        }
    }

    #[test]
    fn every_error_kind_has_a_stable_snake_case_token() {
        for (variant, token) in KINDS {
            assert_eq!(
                serde_json::to_string(&variant).expect("serialize kind"),
                format!("\"{token}\"")
            );
            assert_eq!(
                serde_json::from_str::<ClientErrorKind>(&format!("\"{token}\""))
                    .expect("deserialize kind"),
                variant
            );
        }
    }

    #[test]
    fn every_error_context_has_a_stable_snake_case_token() {
        for (variant, token) in CONTEXTS {
            assert_eq!(
                serde_json::to_string(&variant).expect("serialize context"),
                format!("\"{token}\"")
            );
            assert_eq!(
                serde_json::from_str::<ClientErrorContext>(&format!("\"{token}\""))
                    .expect("deserialize context"),
                variant
            );
        }
    }

    #[test]
    fn every_source_kind_has_a_stable_snake_case_token() {
        for (variant, token) in SOURCES {
            assert_eq!(
                serde_json::to_string(&variant).expect("serialize source kind"),
                format!("\"{token}\"")
            );
            assert_eq!(
                serde_json::from_str::<ClientSourceKind>(&format!("\"{token}\""))
                    .expect("deserialize source kind"),
                variant
            );
        }
    }

    #[test]
    fn event_shape_and_version_are_exact_and_closed() {
        let encoded = serde_json::to_string(&event()).expect("serialize event");

        assert_eq!(
            encoded,
            r#"{"version":1,"kind":"internal","context":"media_form_data","source_kind":"form_data_append"}"#
        );
        assert_eq!(
            serde_json::from_str::<ClientTelemetryEvent>(&encoded).expect("deserialize event"),
            event()
        );
    }

    #[test]
    fn event_rejects_unknown_fields() {
        let encoded = r#"{"version":1,"kind":"internal","context":"media_form_data","source_kind":"form_data_append","detail":"dynamic"}"#;

        assert!(serde_json::from_str::<ClientTelemetryEvent>(encoded).is_err());
    }

    #[test]
    fn event_rejects_an_unknown_version() {
        let encoded = r#"{"version":2,"kind":"internal","context":"media_form_data","source_kind":"form_data_append"}"#;

        assert!(serde_json::from_str::<ClientTelemetryEvent>(encoded).is_err());
    }

    #[test]
    fn event_rejects_unknown_enum_tokens() {
        let unknown_kind = r#"{"version":1,"kind":"other","context":"media_form_data","source_kind":"form_data_append"}"#;
        let unknown_context =
            r#"{"version":1,"kind":"internal","context":"other","source_kind":"form_data_append"}"#;
        let unknown_source =
            r#"{"version":1,"kind":"internal","context":"media_form_data","source_kind":"other"}"#;

        assert!(serde_json::from_str::<ClientTelemetryEvent>(unknown_kind).is_err());
        assert!(serde_json::from_str::<ClientTelemetryEvent>(unknown_context).is_err());
        assert!(serde_json::from_str::<ClientTelemetryEvent>(unknown_source).is_err());
    }

    #[test]
    fn maximum_valid_event_encoding_is_below_1024_bytes() {
        let mut maximum = 0;
        for (kind, _) in KINDS {
            for (context, _) in CONTEXTS {
                for (source_kind, _) in SOURCES {
                    let encoded = serde_json::to_vec(&ClientTelemetryEvent {
                        version: CLIENT_TELEMETRY_VERSION,
                        kind,
                        context,
                        source_kind,
                    })
                    .expect("serialize valid event");
                    maximum = maximum.max(encoded.len());
                }
            }
        }

        assert!(
            maximum < 1_024,
            "maximum valid encoding was {maximum} bytes"
        );
    }
}
