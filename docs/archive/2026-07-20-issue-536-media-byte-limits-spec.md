# Spec — issue #536: validated numeric newtypes for media byte-limits

- Issue: [#536](https://github.com/jaunder-org/jaunder/issues/536)
- Date: 2026-07-20
- Milestone: Domain-value type safety (newtypes)
- Builds on: #464/#535 (the `NumNewtype` macro), ADR-0063 §2 (numeric-value
  trailer)

## Goal

`media.max_file_size_bytes` (default 50 MiB) and `media.user_quota_bytes`
(default 1 GiB) are bare `i64` config limits read through the untyped
`SiteConfigStorage::get_int(key, default) -> i64` (which silently accepts a
stored negative). Their invariant is **positive `i64` bytes** — a zero/negative
limit or quota is nonsensical. Adopt the now-shipped `NumNewtype` macro (this is
one of its two documented sibling cases) to make that invariant a type, thread
it through the media config read path + the `MediaUsageData` DTO, and retire
`get_int`.

## Decisions (interview outcomes)

- **Two types, `MaxFileSize` / `UserQuota`** (not one reused `ByteSize`). The
  macro's `default` is per-type, so two distinct defaults (50 MiB / 1 GiB) map
  cleanly to `unwrap_or_default()` getters — mirroring #535's
  `FeedMinItems`/`FeedMinDays`. A single reused type can carry only one default,
  and the macro emits no `From<i64>`/const constructor, making the two fallbacks
  awkward to build. Two types also give transposition safety (per-file limit vs
  per-user quota).
- **`used_bytes` stays `i64`.** The DTO's usage field is a runtime quantity that
  can legitimately be `0`, so it fails a min-1 invariant and is not one of the
  two config limits this issue targets. (A min-0 "non-negative bytes" type would
  be scope creep.)
- **min = 1** (positive). `i64` inner, so the generated `v < 1` check is
  meaningful (rejects 0 and negatives; no absurd-comparison lint).
- No enforcement gate (config values, no security surface). No ADR change (the
  numeric-value convention is already documented).

## Part 1 — the newtypes (`common::media`)

Add to `common/src/media.rs` (already home to `ContentHash`/`Filename`):

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = i64, min = 1, default = 52_428_800, // 50 MiB
    error = "media max file size must be a positive number of bytes")]
pub struct MaxFileSize(i64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = i64, min = 1, default = 1_073_741_824, // 1 GiB
    error = "media user quota must be a positive number of bytes")]
pub struct UserQuota(i64);
```

Unit tests (in-module, mirroring `feed/settings.rs`): parse accept, reject
`0`/negative/non-integer, `Default::value()` = 50 MiB / 1 GiB, `Display`
round-trip, serde bare-integer round-trip + wire-rejection of `0`, and the
`From<Self> for i64` conversion (so the generated `From` is covered).

Add `common::test_support::parse_max_file_size(&str)` / `parse_user_quota(&str)`
helpers (siblings of `parse_feed_min_items`).

## Part 2 — storage getters (retire `get_int`)

`storage/src/site_config.rs`:

- Add `get_media_max_file_size(&self) -> sqlx::Result<MaxFileSize>` and
  `get_media_user_quota(&self) -> sqlx::Result<UserQuota>`, each
  `.get(KEY).await?.as_deref().and_then(|v| v.parse().ok()).unwrap_or_default()`
  (mirrors the feed getters; a stored `0`/negative now falls back to the
  default).
- **Delete `get_int`** — its only two callers (`media_manager::get_limits`) are
  retired here. Confirmed no other callers repo-wide.
- The `DEFAULT_MAX_FILE_SIZE_BYTES` / `DEFAULT_USER_QUOTA_BYTES` consts in
  `storage/src/media.rs` move into the macro `default = …` attributes (keep the
  `// 50 MiB` / `// 1 GiB` note there); audit and remove the consts if now
  unused (the `MEDIA_*_KEY` consts stay).

## Part 3 — thread the types

**`server/src/media_manager.rs`:**

- `get_limits(&self) -> anyhow::Result<(MaxFileSize, UserQuota)>` via the typed
  getters.
- `stream_to_temp(…, max_file_size: MaxFileSize)` — internally
  `bytes_written > max_file_size.value()`.
- `check_quota(…, user_quota: UserQuota)` —
  `current_usage + size_bytes > user_quota.value()`.
- in-memory path: `size_bytes > max_file_size.value()`.
- `size_bytes` / `current_usage` / `bytes_written` stay `i64` (runtime
  quantities; `.value()` only at the comparison edges).

**`web/src/media/api.rs`:**

- `MediaUsageData { used_bytes: i64, quota_bytes: UserQuota, max_file_size_bytes: MaxFileSize }`.
- `media_usage()` builds `quota_bytes`/`max_file_size_bytes` from the new typed
  getters (replacing the local `.get(KEY)…unwrap_or(DEFAULT)` parse);
  `used_bytes` from `get_user_upload_usage` unchanged.
- Transparent-i64 serde keeps the wire form bare integers — **no
  serialized-shape change, no e2e break**.

**`web/src/media/component.rs`** (wasm UI): the display reads
`max_file_size_bytes` / `quota_bytes` — convert with `.value()` (or `i64::from`)
where they feed `format_bytes`/arithmetic.

## Acceptance (from the issue)

- `MaxFileSize`/`UserQuota` built via the `NumNewtype` macro; validation/error
  paths covered by unit tests.
- Media config read path + `MediaUsageData` typed end-to-end; wire shape
  unchanged (bare integers via the serde bridge); no e2e breakage.
- `get_int` retired.
- `cargo xtask validate --no-e2e` clean (storage getter tests dual-backend via
  `#[apply(backends)]`).

## Risks / notes

- **Atomic compile:** changing `MediaUsageData` field types + `get_limits`
  return breaks `web`/`server` until consumers update — one commit for the
  threading (Part 3), as in #535.
- **`common` wasm-safety:** the types live in `common::media` (already
  wasm-safe, hosting `ContentHash`/`Filename`); `web`'s CSR build deserializes
  the DTO, so the newtype `Deserialize` (min-1 check) runs client-side — fine
  (server only ever sends validated config).
- Post-merge, **#537** (`PageSize`, the range case) remains the last sibling.
