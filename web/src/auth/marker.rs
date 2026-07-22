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

/// The whole client-visible session identity (#181, #591, ADR-0044): who is logged
/// in and whether they are an operator. Persisted in the advisory marker and
/// returned by `session()`. `is_operator` is advisory chrome only —
/// `require_operator()` is the real privilege guard, so a hand-edited marker grants
/// nothing. Named `SessionUser` to stay distinct from the device-listing
/// `sessions::SessionInfo` and the visibility `ViewerIdentity`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUser {
    pub username: Username,
    /// Absent in pre-#591 markers → `false`, so an existing session decodes as a
    /// (non-operator) logged-in user rather than `None`/anonymous.
    #[serde(default)]
    pub is_operator: bool,
}

/// The localStorage value (JSON `{"username":"…","is_operator":<bool>}`). `username`
/// stays the top-level key the pre-paint script reads.
#[must_use]
pub fn encode_marker(user: &SessionUser) -> String {
    serde_json::to_string(user).unwrap_or_default()
}

/// Parse a marker value back to its [`SessionUser`], `None` when the JSON is
/// malformed or the stored username is invalid. The single malformed→`None`
/// chokepoint: `Username`'s own `Deserialize` routes through its validating
/// `FromStr` (a `Username` cannot be empty), and a missing `is_operator` defaults
/// to `false` for backward compatibility.
#[must_use]
pub fn decode_marker(raw: &str) -> Option<SessionUser> {
    serde_json::from_str(raw).ok()
}

#[cfg(test)]
mod tests {
    use common::test_support::parse_username;

    use super::{decode_marker, encode_marker, SessionUser};

    #[test]
    fn round_trips_session_info() {
        let info = SessionUser {
            username: parse_username("alice"),
            is_operator: true,
        };
        let raw = encode_marker(&info);
        // The exact JSON the pre-paint `<head>` script parses: `username` stays the
        // top-level key (the script reads only `.username`); `is_operator` is additive.
        assert_eq!(raw, r#"{"username":"alice","is_operator":true}"#);
        assert_eq!(decode_marker(&raw), Some(info));
    }

    #[test]
    fn decode_defaults_is_operator_when_absent() {
        // Backward compat: markers written before #591 lack `is_operator`. They MUST
        // decode as a non-operator session, not `None` — else an existing session
        // flashes anonymous on the first post-deploy boot (spec §1).
        assert_eq!(
            decode_marker(r#"{"username":"alice"}"#),
            Some(SessionUser {
                username: parse_username("alice"),
                is_operator: false,
            }),
        );
    }

    #[test]
    fn round_trips_all_valid_username_chars() {
        // Hyphen/underscore/digit are valid username chars; confirm they survive
        // the JSON round-trip (none need escaping — the charset excludes `"`/`\`).
        let info = SessionUser {
            username: parse_username("a_b-9"),
            is_operator: false,
        };
        assert_eq!(decode_marker(&encode_marker(&info)), Some(info));
    }

    #[test]
    fn decode_rejects_malformed_json() {
        assert_eq!(decode_marker("not json"), None);
        assert_eq!(decode_marker("{}"), None); // missing required `username`
    }

    #[test]
    fn decode_rejects_invalid_username() {
        // Well-formed JSON whose `username` is not a valid `Username` → `None`
        // (the codec is the single malformed→`None` chokepoint).
        assert_eq!(decode_marker(r#"{"username":"Has Space"}"#), None);
        assert_eq!(decode_marker(r#"{"username":""}"#), None);
    }
}
