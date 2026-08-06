//! The client-side **auth marker** (#181, ADR-0044): a JS-readable localStorage
//! value advertising "probably the owner" for pre-paint chrome adjustment. It is
//! ADVISORY, not a credential — the real session stays the HTTP-only cookie, and
//! the server authorizes every mutation.
//!
//! The pure codec + `MARKER_KEY` live in `common::session_user` (moved there so
//! `test-support` can build markers without linking `web`, #791) and are
//! re-exported here unchanged; the wasm-only `localStorage` binding lives in
//! [`super::marker_storage`] (#514).

pub use common::session_user::{MARKER_KEY, SessionUser, decode_marker, encode_marker};
