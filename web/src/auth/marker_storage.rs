//! Browser (`localStorage`) binding of the auth marker (#181, ADR-0044). The pure
//! codec + `MARKER_KEY` live in [`super::marker`] (host-tested); this wasm-only
//! module pairs them with the generic [`client::storage`] primitive. Split out of
//! `marker.rs` (#514) so that codec file stays cfg-free and host-tested.

use super::marker::{decode_marker, encode_marker, MARKER_KEY};

/// Read + decode the marker from localStorage, `None` when absent/malformed.
#[must_use]
pub fn read() -> Option<String> {
    decode_marker(&client::storage::get(MARKER_KEY)?)
}

/// Write the marker for `username`.
pub fn set(username: &str) {
    client::storage::set(MARKER_KEY, &encode_marker(username));
}

/// Remove the marker.
pub fn clear() {
    client::storage::remove(MARKER_KEY);
}
