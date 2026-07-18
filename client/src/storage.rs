//! Generic browser `localStorage` key/value access — raw browser infrastructure
//! per the `client` charter (ADR-0069), no domain types.
//!
//! Every operation returns a [`Result`] so callers get a truthful, complete
//! accounting of what happened and choose their own failure policy — this
//! primitive never swallows a browser error on the caller's behalf. `get`
//! additionally distinguishes an absent key (`Ok(None)`) from a store it could not
//! read (`Err`).

use std::fmt;
use wasm_bindgen::JsValue;

/// Why a `localStorage` operation could not complete.
#[derive(Debug, Clone)]
pub enum StorageError {
    /// `window.localStorage` could not be obtained: there is no `window` (non-browser
    /// context) or no storage object (`None`), or the browser denied access and threw
    /// (`Some(err)` — e.g. storage disabled / a sandboxed origin).
    Unavailable(Option<JsValue>),
    /// The `getItem` / `setItem` / `removeItem` call itself threw — e.g. a
    /// `QuotaExceededError` on a write.
    Operation(JsValue),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(None) => write!(f, "localStorage is unavailable"),
            Self::Unavailable(Some(err)) => write!(f, "localStorage is unavailable: {err:?}"),
            Self::Operation(err) => write!(f, "localStorage operation failed: {err:?}"),
        }
    }
}

impl std::error::Error for StorageError {}

/// The window's `localStorage`, or a [`StorageError::Unavailable`] explaining why not.
fn local_storage() -> Result<web_sys::Storage, StorageError> {
    web_sys::window()
        .ok_or(StorageError::Unavailable(None))?
        .local_storage()
        .map_err(|err| StorageError::Unavailable(Some(err)))?
        .ok_or(StorageError::Unavailable(None))
}

/// Read the string stored under `key`. `Ok(None)` means the key is absent; `Err`
/// means the store could not be reached or read.
///
/// # Errors
///
/// [`StorageError::Unavailable`] if `localStorage` cannot be obtained;
/// [`StorageError::Operation`] if the `getItem` call itself throws.
pub fn get(key: &str) -> Result<Option<String>, StorageError> {
    local_storage()?
        .get_item(key)
        .map_err(StorageError::Operation)
}

/// Store `value` under `key`.
///
/// # Errors
///
/// [`StorageError::Unavailable`] if `localStorage` cannot be obtained;
/// [`StorageError::Operation`] if the `setItem` call throws (e.g. `QuotaExceededError`).
pub fn set(key: &str, value: &str) -> Result<(), StorageError> {
    local_storage()?
        .set_item(key, value)
        .map_err(StorageError::Operation)
}

/// Remove any value stored under `key`.
///
/// # Errors
///
/// [`StorageError::Unavailable`] if `localStorage` cannot be obtained;
/// [`StorageError::Operation`] if the `removeItem` call throws.
pub fn remove(key: &str) -> Result<(), StorageError> {
    local_storage()?
        .remove_item(key)
        .map_err(StorageError::Operation)
}
