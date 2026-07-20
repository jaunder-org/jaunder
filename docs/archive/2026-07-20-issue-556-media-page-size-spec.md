# Spec — bounded page size for media listing (issue #556)

Status: proposed · Issue: jaunder-org/jaunder#556 · Follow-up to #537
(`PageSize`); family #464, #536, ADR-0063.

## Problem

`web/src/media/api.rs` `list_my_media` takes `limit: Option<u32>` and applies
`limit.unwrap_or(50)` with **no upper bound**, so a hand-crafted request can
fetch an arbitrarily large media page straight from the storage query. Its only
caller (`web/src/media/component.rs`) always passes `Some(50)` — structurally
identical to the posts page-size that #537 moved into
`common::pagination::PageSize` (`1..=50`).

## Solution

Adopt `common::pagination::PageSize` at `list_my_media`'s `limit`, exactly as
#537 did for the posts surface — decided (owner): **reuse `PageSize`
(`1..=50`)**, not a media-specific bound. The UI is unaffected (still 50); an
out-of-range media request now rejects on the wire (the `PageSize` serde bridge)
instead of passing an unbounded limit to the query. `offset` is a scroll
position with no `1..=N` invariant and **stays `Option<u32>`**.

## Acceptance criteria

- **AC1 — Typed limit.** `list_my_media`'s `limit` arg is `Option<PageSize>`;
  the body uses `limit.unwrap_or_default().value()` for the storage call. No
  `unwrap_or(50)` page-size literal remains. `offset` is unchanged
  (`Option<u32>`, `unwrap_or(0)`).
- **AC2 — Caller adopts.** The client caller in `web/src/media/component.rs`
  passes `Some(PageSize::default())` (not `Some(50)`).
- **AC3 — Storage boundary unchanged.** `MediaStorage::list_media` keeps its raw
  `u32` limit; the `PageSize` is unwrapped via `.value()` at the call site
  (edge-conversion).
- **AC4 — Behavior preserved for the UI; out-of-range rejected.** A
  `list_my_media` call with the default limit still returns up to 50 items; an
  out-of-range wire `limit` (e.g. `0` or `> 50`) is rejected by deserialization
  rather than silently honored.
- **AC5 — Tests + gate.** Any `cfg(test)` site constructing a media `limit`
  builds it via `PageSize`/`parse_page_size`; `cargo xtask validate --no-e2e`
  clean.

## Out of scope

- `offset` typing; the `list_media` storage signature; any other media behavior.
  No new newtype (reuse `PageSize`).

## Verification

`cargo xtask validate --no-e2e` (AC5). Behavior spot-check: the media library
grid still loads (50/page); an out-of-range `?limit=` is rejected.
