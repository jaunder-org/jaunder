//! Generic browser `localStorage` key/value access — raw browser infrastructure
//! per the `client` charter (ADR-0069), no domain types.
//!
//! Every operation returns a [`Result`] so callers get a truthful, complete
//! accounting of what happened and choose their own failure policy — this
//! primitive never swallows a browser error on the caller's behalf. `get`
//! additionally distinguishes an absent key (`Ok(None)`) from a store it could not
//! read (`Err`).

pub use crate::telemetry::StorageError;

/// The window's `localStorage`, or a [`StorageError::Unavailable`].
fn local_storage() -> Result<web_sys::Storage, StorageError> {
    web_sys::window()
        .ok_or(StorageError::Unavailable)?
        .local_storage()
        .map_err(|_| StorageError::Unavailable)?
        .ok_or(StorageError::Unavailable)
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
        .map_err(|_| StorageError::Operation)
}

/// Store `value` under `key`.
///
/// # Errors
///
/// [`StorageError::Unavailable`] if `localStorage` cannot be obtained;
/// [`StorageError::Operation`] if the `setItem` call throws.
pub fn set(key: &str, value: &str) -> Result<(), StorageError> {
    local_storage()?
        .set_item(key, value)
        .map_err(|_| StorageError::Operation)
}

/// Remove any value stored under `key`.
///
/// # Errors
///
/// [`StorageError::Unavailable`] if `localStorage` cannot be obtained;
/// [`StorageError::Operation`] if the `removeItem` call itself throws.
pub fn remove(key: &str) -> Result<(), StorageError> {
    local_storage()?
        .remove_item(key)
        .map_err(|_| StorageError::Operation)
}

#[cfg(test)]
mod tests {
    use super::{get, remove, set};
    use wasm_bindgen_test::wasm_bindgen_test;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const TEST_KEY: &str = "jaunder-wasm-test-storage-lifecycle";

    #[wasm_bindgen_test]
    fn local_storage_lifecycle() {
        let stale_cleanup = remove(TEST_KEY);
        let initial = get(TEST_KEY);
        let stored = set(TEST_KEY, "stored-value");
        let observed = get(TEST_KEY);
        let removed = remove(TEST_KEY);
        let after_remove = get(TEST_KEY);
        let final_cleanup = remove(TEST_KEY);

        assert!(stale_cleanup.is_ok(), "stale cleanup: {stale_cleanup:?}");
        assert!(matches!(initial, Ok(None)), "initial state: {initial:?}");
        assert!(stored.is_ok(), "store: {stored:?}");
        assert!(matches!(&observed, Ok(Some(value)) if value == "stored-value"));
        assert!(removed.is_ok(), "remove: {removed:?}");
        assert!(
            matches!(after_remove, Ok(None)),
            "state after remove: {after_remove:?}"
        );
        assert!(final_cleanup.is_ok(), "final cleanup: {final_cleanup:?}");
    }
}
