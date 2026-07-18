//! Generic browser `localStorage` key/value access — raw browser infrastructure
//! per the `client` charter (ADR-0069), no domain types. Best-effort: every
//! operation silently no-ops when `window`/`localStorage` is unavailable or the
//! browser rejects the access (private-mode quota, storage disabled), matching the
//! swallow-the-error behavior the migrated `web` call sites have always had.

/// The window's `localStorage`, or `None` when unavailable.
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Read the string stored under `key`, or `None` if absent/unavailable.
#[must_use]
pub fn get(key: &str) -> Option<String> {
    local_storage()?.get_item(key).ok().flatten()
}

/// Store `value` under `key` (best-effort; ignores failure).
pub fn set(key: &str, value: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(key, value);
    }
}

/// Remove any value stored under `key` (best-effort; ignores failure).
pub fn remove(key: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.remove_item(key);
    }
}
