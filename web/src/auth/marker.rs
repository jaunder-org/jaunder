//! The client-side **auth marker** (#181, ADR-0044): a JS-readable localStorage
//! value advertising "probably the owner" for pre-paint chrome adjustment. It is
//! ADVISORY, not a credential — the real session stays the HTTP-only cookie, and
//! the server authorizes every mutation. The pre-paint `<head>` script
//! (`render::PREPAINT_SCRIPT`) reads the SAME key + `.username` field, so the
//! encode/decode shape here and that script must stay in sync.

use serde::{Deserialize, Serialize};

/// The localStorage key holding the marker. Kept in sync with the pre-paint script.
pub const MARKER_KEY: &str = "jaunder_auth";

#[derive(Serialize)]
struct Marker<'a> {
    username: &'a str,
}

/// The localStorage value for `username` (JSON `{"username":"…"}`).
#[must_use]
pub fn encode_marker(username: &str) -> String {
    serde_json::to_string(&Marker { username }).unwrap_or_default()
}

/// Parse a marker value back to its username, `None` when malformed/empty.
#[must_use]
pub fn decode_marker(raw: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Owned {
        username: String,
    }
    let m: Owned = serde_json::from_str(raw).ok()?;
    (!m.username.is_empty()).then_some(m.username)
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Read + decode the marker from localStorage (browser-only).
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn read() -> Option<String> {
    let raw = storage()?.get_item(MARKER_KEY).ok().flatten()?;
    decode_marker(&raw)
}

/// Write the marker for `username` (browser-only).
#[cfg(target_arch = "wasm32")]
pub fn set(username: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(MARKER_KEY, &encode_marker(username));
    }
}

/// Remove the marker (browser-only).
#[cfg(target_arch = "wasm32")]
pub fn clear() {
    if let Some(s) = storage() {
        let _ = s.remove_item(MARKER_KEY);
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_marker, encode_marker};

    #[test]
    fn round_trips_username() {
        let raw = encode_marker("alice");
        assert_eq!(raw, r#"{"username":"alice"}"#);
        assert_eq!(decode_marker(&raw), Some("alice".to_string()));
    }

    #[test]
    fn decode_rejects_malformed() {
        assert_eq!(decode_marker("not json"), None);
        assert_eq!(decode_marker("{}"), None);
    }

    #[test]
    fn encode_escapes_json() {
        // A quote in a username must not break the JSON the pre-paint script parses.
        assert_eq!(
            decode_marker(&encode_marker(r#"a"b"#)),
            Some(r#"a"b"#.into())
        );
    }
}
