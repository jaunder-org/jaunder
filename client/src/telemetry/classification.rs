//! Pure, host-testable contracts for audited browser failures.

use std::fmt;

use common::client_telemetry::{
    self, ClientErrorContext, ClientErrorKind, ClientSourceKind, ClientTelemetryEvent,
};

/// Why a `localStorage` operation could not complete.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum StorageError {
    /// `window.localStorage` could not be obtained.
    Unavailable,
    /// The `getItem` / `setItem` / `removeItem` call itself threw.
    Operation,
}

impl StorageError {
    /// The closed telemetry source classification for this error.
    #[must_use]
    pub const fn source_kind(self) -> ClientSourceKind {
        match self {
            Self::Unavailable => ClientSourceKind::StorageUnavailable,
            Self::Operation => ClientSourceKind::StorageOperation,
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => write!(f, "localStorage is unavailable"),
            Self::Operation => write!(f, "localStorage operation failed"),
        }
    }
}

impl std::error::Error for StorageError {}

/// A non-error outcome from attempting to show a confirmation dialog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmOutcome {
    /// The user accepted the action.
    Confirmed,
    /// The user declined the action.
    Cancelled,
    /// No browser window exists, so no dialog could be offered.
    Unavailable,
}

impl ConfirmOutcome {
    /// Whether the caller should dispatch the confirmed action.
    #[must_use]
    pub const fn should_dispatch(self) -> bool {
        matches!(self, Self::Confirmed)
    }
}

/// An exception thrown by the browser dialog API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DialogError;

impl DialogError {
    /// The closed telemetry source classification for a thrown dialog API.
    #[must_use]
    pub const fn source_kind(self) -> ClientSourceKind {
        ClientSourceKind::DialogUnavailable
    }
}

/// The expected selection outcome from a file picker.
#[derive(Debug)]
pub enum UploadOutcome<T> {
    /// The input is absent, exposes no file list, or has no selected file.
    NoFile,
    /// A selected file was successfully wrapped for upload.
    Ready(T),
}

impl<T> UploadOutcome<T> {
    /// Return ready multipart data, or `None` for the expected no-file path.
    #[must_use]
    pub fn into_ready(self) -> Option<T> {
        match self {
            Self::NoFile => None,
            Self::Ready(value) => Some(value),
        }
    }
}

/// Why a selected file could not be wrapped as multipart form data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadError {
    /// The browser refused to create `FormData`.
    FormDataCreate,
    /// The browser refused to append the selected file.
    FormDataAppend,
}

impl UploadError {
    /// The closed telemetry source classification for this error.
    #[must_use]
    pub const fn source_kind(self) -> ClientSourceKind {
        match self {
            Self::FormDataCreate => ClientSourceKind::FormDataCreate,
            Self::FormDataAppend => ClientSourceKind::FormDataAppend,
        }
    }
}

/// Derive the broad error kind from the closed browser-source classification.
#[must_use]
pub const fn error_kind(source_kind: ClientSourceKind) -> ClientErrorKind {
    match source_kind {
        ClientSourceKind::StorageUnavailable | ClientSourceKind::StorageOperation => {
            ClientErrorKind::Storage
        }
        ClientSourceKind::InvalidSeed => ClientErrorKind::Decode,
        ClientSourceKind::DialogUnavailable => ClientErrorKind::Dialog,
        ClientSourceKind::FormDataCreate | ClientSourceKind::FormDataAppend => {
            ClientErrorKind::FormData
        }
    }
}

/// Build the exact closed event represented by one audited browser failure.
#[must_use]
pub const fn event(
    context: ClientErrorContext,
    source_kind: ClientSourceKind,
) -> ClientTelemetryEvent {
    ClientTelemetryEvent {
        version: client_telemetry::WIRE_VERSION,
        kind: error_kind(source_kind),
        context,
        source_kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_errors_map_to_one_closed_event_and_missing_value_maps_to_none() {
        for (error, source_kind) in [
            (
                StorageError::Unavailable,
                ClientSourceKind::StorageUnavailable,
            ),
            (StorageError::Operation, ClientSourceKind::StorageOperation),
        ] {
            let events = [event(
                ClientErrorContext::ThemeStorageRead,
                error.source_kind(),
            )];
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].kind, ClientErrorKind::Storage);
            assert_eq!(events[0].source_kind, source_kind);
        }

        let missing: Result<Option<String>, StorageError> = Ok(None);
        let missing_event = missing
            .err()
            .map(|error| event(ClientErrorContext::ThemeStorageRead, error.source_kind()));
        assert_eq!(missing_event, None);
        assert_eq!(
            StorageError::Unavailable.to_string(),
            "localStorage is unavailable"
        );
        assert_eq!(
            StorageError::Operation.to_string(),
            "localStorage operation failed"
        );
    }

    #[test]
    fn dialog_decision_and_error_classification_are_exact() {
        assert!(ConfirmOutcome::Confirmed.should_dispatch());
        assert!(!ConfirmOutcome::Cancelled.should_dispatch());
        assert!(!ConfirmOutcome::Unavailable.should_dispatch());

        let events = [event(
            ClientErrorContext::PublishConfirm,
            DialogError.source_kind(),
        )];
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ClientErrorKind::Dialog);
        assert_eq!(events[0].source_kind, ClientSourceKind::DialogUnavailable);
    }

    #[test]
    fn form_data_errors_and_no_file_outcome_are_exact() {
        assert_eq!(UploadOutcome::<()>::NoFile.into_ready(), None);
        assert_eq!(UploadOutcome::Ready(7).into_ready(), Some(7));

        for (error, source_kind) in [
            (
                UploadError::FormDataCreate,
                ClientSourceKind::FormDataCreate,
            ),
            (
                UploadError::FormDataAppend,
                ClientSourceKind::FormDataAppend,
            ),
        ] {
            let events = [event(
                ClientErrorContext::MediaFormData,
                error.source_kind(),
            )];
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].kind, ClientErrorKind::FormData);
            assert_eq!(events[0].source_kind, source_kind);
        }
    }

    #[test]
    fn every_source_kind_has_one_broad_kind() {
        let cases = [
            (
                ClientSourceKind::StorageUnavailable,
                ClientErrorKind::Storage,
            ),
            (ClientSourceKind::StorageOperation, ClientErrorKind::Storage),
            (ClientSourceKind::InvalidSeed, ClientErrorKind::Decode),
            (ClientSourceKind::DialogUnavailable, ClientErrorKind::Dialog),
            (ClientSourceKind::FormDataCreate, ClientErrorKind::FormData),
            (ClientSourceKind::FormDataAppend, ClientErrorKind::FormData),
        ];
        for (source_kind, kind) in cases {
            assert_eq!(error_kind(source_kind), kind);
        }
    }

    #[test]
    fn event_has_no_field_for_arbitrary_exception_text() {
        let encoded = serde_json::to_value(event(
            ClientErrorContext::DeleteConfirm,
            ClientSourceKind::DialogUnavailable,
        ))
        .unwrap();
        let fields = encoded.as_object().unwrap();

        assert_eq!(fields.len(), 4);
        assert!(!fields.contains_key("message"));
        assert!(!fields.contains_key("source"));
    }
}
