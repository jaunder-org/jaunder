//! App-theme localStorage accessors.
//!
//! This is the app theme's only direct bridge to the raw `client::storage`
//! primitive; callers keep the theme as an opaque string and decide failure
//! reporting policy.

use common::local_storage_key::LocalStorageKey;

/// Read the persisted app theme.
pub(super) fn get() -> Result<Option<String>, client::storage::StorageError> {
    client::storage::get(LocalStorageKey::Theme.as_ref())
}

/// Persist the current app theme.
pub(super) fn set(theme: &str) -> Result<(), client::storage::StorageError> {
    client::storage::set(LocalStorageKey::Theme.as_ref(), theme)
}
