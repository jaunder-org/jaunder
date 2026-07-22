//! Load status for a store-backed reactive list.

/// The load status of a store-backed list: `Loading` until the first resolve, then
/// `Empty` / `Loaded` per the row count, or `Error` on a failed fetch. Produced by
/// `client`'s `patched` helper and consumed by `web`'s reactive list rendering; rendered
/// as a sibling to the (unconditionally mounted) list, so the list itself is never inside a
/// branch that could tear it down on a refetch. Derive-only — a pure enum, no `leptos`.
#[derive(Clone, Debug)]
pub enum ListState {
    /// The first fetch has not resolved yet.
    Loading,
    /// Resolved with no rows.
    Empty,
    /// Resolved with at least one row.
    Loaded,
    /// The fetch failed; the payload is the error's `Display`.
    Error(String),
}
