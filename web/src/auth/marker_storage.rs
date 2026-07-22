//! Browser (`localStorage`) binding of the auth marker (#181, ADR-0044). The pure
//! codec + `MARKER_KEY` live in [`super::marker`] (host-tested); this wasm-only
//! module pairs them with the generic [`client::storage`] primitive. Split out of
//! `marker.rs` (#514) so that codec file stays cfg-free and host-tested.
//!
//! The marker is **advisory**: the server authorizes every mutation, and the
//! sidebar's reconcile `Effect` (ADR-0044 D3) re-establishes the marker against the
//! real session on the next load. So a `client::storage` failure here is non-fatal
//! and is deliberately absorbed toward the safe (anonymous) direction, rather than
//! propagated to callers that could not act on it — the policy choice this advisory
//! layer is entitled to make on the primitive's truthful `Result`.

use super::marker::{decode_marker, encode_marker, SessionUser, MARKER_KEY};

/// Get + decode the marker. `None` when absent, malformed, **or** the store could
/// not be read — an unreadable marker is treated as "no marker" (anonymous chrome),
/// which the reconcile `Effect` corrects if the session says otherwise.
#[must_use]
pub fn get() -> Option<SessionUser> {
    client::storage::get(MARKER_KEY)
        .ok()
        .flatten()
        .and_then(|raw| decode_marker(&raw))
}

/// Write the marker for `user`. A failed write is non-fatal — the reconcile
/// `Effect` re-writes it on the next load.
pub fn set(user: &SessionUser) {
    let _ = client::storage::set(MARKER_KEY, &encode_marker(user));
}

/// Remove the marker. A failed removal is non-fatal — the reconcile `Effect` clears
/// a stale marker against a dead session on the next load.
pub fn remove() {
    let _ = client::storage::remove(MARKER_KEY);
}
