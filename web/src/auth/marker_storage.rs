//! Browser (`localStorage`) binding of the auth marker (#181, ADR-0044). The pure
//! codec lives in [`super::marker`] (host-tested); this wasm-only module pairs it
//! with the typed product key and generic [`client::storage`] primitive. Split out of
//! `marker.rs` (#514) so that codec file stays cfg-free and host-tested.
//!
//! The marker is **advisory**: the server authorizes every mutation, and the
//! sidebar's reconcile `Effect` (ADR-0044 D3) re-establishes the marker against the
//! real session on the next load. So a `client::storage` failure here is non-fatal
//! and is deliberately absorbed toward the safe (anonymous) direction, rather than
//! propagated to callers that could not act on it — the policy choice this advisory
//! layer is entitled to make on the primitive's truthful `Result`.

use super::marker::{SessionUser, decode_marker, encode_marker};
use common::{client_telemetry::ClientErrorContext, local_storage_key::LocalStorageKey};

fn report_storage_error(context: ClientErrorContext, error: client::storage::StorageError) {
    let source_kind = error.source_kind();
    client::telemetry::report_swallowed(
        client::telemetry::error_kind(source_kind),
        context,
        source_kind,
    );
}

/// Get + decode the marker. `None` when absent, malformed, or the store could
/// not be read. Only the unexpected storage failure is reported; an absent or
/// malformed advisory marker remains ordinary anonymous control flow.
#[must_use]
pub fn get() -> Option<SessionUser> {
    match client::storage::get(LocalStorageKey::AuthMarker.as_str()) {
        Ok(raw) => raw.and_then(|raw| decode_marker(&raw)),
        Err(error) => {
            report_storage_error(ClientErrorContext::SessionMarkerRead, error);
            None
        }
    }
}

/// Write the marker for `user`. A failed write is non-fatal — the reconcile
/// `Effect` re-writes it on the next load — but is reported before continuing.
pub fn set(user: &SessionUser) {
    if let Err(error) =
        client::storage::set(LocalStorageKey::AuthMarker.as_str(), &encode_marker(user))
    {
        report_storage_error(ClientErrorContext::SessionMarkerWrite, error);
    }
}

/// Remove the marker. A failed removal is non-fatal — the reconcile `Effect`
/// clears a stale marker against a dead session on the next load — but is
/// reported before continuing.
pub fn remove() {
    if let Err(error) = client::storage::remove(LocalStorageKey::AuthMarker.as_str()) {
        report_storage_error(ClientErrorContext::SessionMarkerRemove, error);
    }
}
