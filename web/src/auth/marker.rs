//! The client-side **auth marker** (#181, ADR-0044): a JS-readable localStorage
//! value advertising "probably the owner" for pre-paint chrome adjustment. It is
//! ADVISORY, not a credential — the real session stays the HTTP-only cookie, and
//! the server authorizes every mutation. The pre-paint `<head>` script
//! (`render::PREPAINT_SCRIPT`) reads the SAME key + `.username` field, so the
//! encode/decode shape here and that script must stay in sync.
//!
//! Pure codec only: the wasm-only `localStorage` binding lives in
//! [`super::marker_storage`] (#514).

use common::username::Username;
use serde::{Deserialize, Serialize};

/// The localStorage key holding the marker. Kept in sync with the pre-paint script.
pub const MARKER_KEY: &str = "jaunder_auth";

#[derive(Serialize)]
struct Marker<'a> {
    username: &'a str,
}

/// The localStorage value for `username` (JSON `{"username":"…"}`).
#[must_use]
pub fn encode_marker(username: &Username) -> String {
    serde_json::to_string(&Marker {
        username: username.as_ref(),
    })
    .unwrap_or_default()
}

/// Parse a marker value back to its [`Username`], `None` when the JSON is
/// malformed or the stored username is invalid. The single malformed→`None`
/// chokepoint: routing through `Username`'s validating `FromStr` subsumes the
/// old emptiness check (a `Username` cannot be empty).
#[must_use]
pub fn decode_marker(raw: &str) -> Option<Username> {
    #[derive(Deserialize)]
    struct Owned {
        username: String,
    }
    let m: Owned = serde_json::from_str(raw).ok()?;
    m.username.parse().ok()
}

#[cfg(test)]
mod tests {
    use common::test_support::parse_username;

    use super::{decode_marker, encode_marker};

    #[test]
    fn round_trips_username() {
        let u = parse_username("alice");
        let raw = encode_marker(&u);
        // The exact JSON the pre-paint `<head>` script parses — must not drift.
        assert_eq!(raw, r#"{"username":"alice"}"#);
        assert_eq!(decode_marker(&raw), Some(u));
    }

    #[test]
    fn round_trips_all_valid_username_chars() {
        // Hyphen/underscore/digit are valid username chars; confirm they survive
        // the JSON round-trip (none need escaping — the charset excludes `"`/`\`).
        let u = parse_username("a_b-9");
        assert_eq!(decode_marker(&encode_marker(&u)), Some(u));
    }

    #[test]
    fn decode_rejects_malformed_json() {
        assert_eq!(decode_marker("not json"), None);
        assert_eq!(decode_marker("{}"), None);
    }

    #[test]
    fn decode_rejects_invalid_username() {
        // Well-formed JSON whose `username` is not a valid `Username` → `None`
        // (the codec is the single malformed→`None` chokepoint).
        assert_eq!(decode_marker(r#"{"username":"Has Space"}"#), None);
        assert_eq!(decode_marker(r#"{"username":""}"#), None);
    }
}
