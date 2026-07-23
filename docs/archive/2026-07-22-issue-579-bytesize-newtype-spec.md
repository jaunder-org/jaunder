# Spec — #579: `ByteSize` newtype for actual byte counts

- Issue: [#579](https://github.com/jaunder-org/jaunder/issues/579)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
- Related: [#536](https://github.com/jaunder-org/jaunder/issues/536)
  (`MaxFileSize`/`UserQuota` — the limits side),
  [#464](https://github.com/jaunder-org/jaunder/issues/464) (`NumNewtype` macro)
- Date: 2026-07-22

## Problem

The media byte-count domain is half-typed: the _limits_ are newtypes
(`MaxFileSize`, `UserQuota`), but the _actual_ sizes they bound are raw `i64`
end-to-end — `MediaRecord.size_bytes`,
`MediaStorage::get_user_upload_usage(…) -> i64` (+ the `MediaDialect` twin), the
web DTOs (`MediaItem.size_bytes`, `MediaUsageData.used_bytes`, the latter
sitting beside typed `MaxFileSize`/`UserQuota`), and `format_bytes(bytes: i64)`.
The non-negativity invariant is unstated, and every size-vs-limit comparison
mixes a raw `i64` with a newtype's inner value.

## Decision

Introduce `ByteSize` (a `NumNewtype`, `inner = i64`, `min = 0`) in
`common::media` and thread it through the record, the usage query, and the web
DTOs. `format_bytes` becomes generic over the byte-ish newtypes.

### Scene-setting: `NumNewtype` gains a validating `TryFrom<inner>` (macros)

`NumNewtype` today can only be built from a _string_ (`FromStr`), a deserialized
value (serde), a declared `default`, or `clamped` (which needs both bounds). It
deliberately omits `From<inner>` — an _unchecked_ constructor "would bypass the
bound" (`macros/src/num_newtype.rs:108`). But `MaxFileSize`/`UserQuota` are only
ever built from config strings, whereas **`ByteSize` is the first `NumNewtype`
that must be built from a runtime `i64`** — the trusted-but-fallible
`size_bytes` DB column and the `COALESCE(SUM(size_bytes), 0)` usage aggregate.

So the **first change** adds a **validating `TryFrom<inner>`** to the macro —
the checked analogue of the omitted `From<inner>`: it routes the value through
the _same_ min/max bound check as `FromStr`/serde, returning the generated
`Invalid<Name>` on an out-of-bounds input. This is the one safe
integer-construction door; it is general (every `NumNewtype` gains it) and
`ByteSize` is the first adopter. Covered by a macros unit test (the crate is
coverage-measured) — a positive-branch fixture plus an out-of-bounds rejection.

The macro's _deliberate omission_ of `From<inner>` is exactly what makes this
legal: were `From<i64> for Name` present, std's blanket
`impl<T, U: Into<T>> TryFrom<U> for T` would auto-provide an **infallible**
`TryFrom<i64>`, colliding with the generated checked one. Because
`i64: Into<Name>` does not hold, the blanket doesn't apply and the emitted
`TryFrom<i64>` is conflict-free for every `NumNewtype`. (`From<Name> for i64` is
the opposite direction — no overlap.)

### The newtype — `common::media::ByteSize`

```rust
/// A non-negative count of bytes — a measured/stored size (a media file's byte length,
/// a user's total upload usage), the actual-value counterpart to the MaxFileSize /
/// UserQuota *limits*. `min = 0` (an empty object is 0 bytes); no default (it is
/// measured, never a config fallback).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, NumNewtype)]
#[num_newtype(inner = i64, min = 0, error = "byte size must be a non-negative number of bytes")]
pub struct ByteSize(i64);
```

`Ord`/`PartialOrd` are added (byte sizes are naturally ordered — useful if
comparison helpers land; harmless otherwise). Generated surface: `value()`,
`From<ByteSize> for i64`, `FromStr`, `Display`, serde, and the new
`TryFrom<i64>`.

### `storage` — record, dialect, boundaries

- `MediaRecord.size_bytes: ByteSize` (was `i64`).
- `helpers::media_record_from_row` (the FromRow → record boundary;
  `MediaRow.size_bytes` stays raw `i64`): wrap via
  `ByteSize::try_from(size_bytes)` mapping the impossible-in- practice failure
  to `sqlx::Error::ColumnDecode`. This is the **in-function** wrap structurally
  identical to the neighboring `source: MediaSource` parse in the same fn —
  _not_ the `sha256`/`filename` path (those decode at the `MediaRow` tuple layer
  via the `StrNewtype` sqlx bridge, #438, which `NumNewtype` has no analogue
  of). A negative size in the DB (data corruption) is thus surfaced as a decode
  error, not silently accepted.
- `MediaStore::insert` binds `i64::from(record.size_bytes)` (was
  `.bind(record.size_bytes)`).
- `MediaStorage::get_user_upload_usage(…) -> sqlx::Result<ByteSize>` and the
  `MediaDialect` twin: the dialect SQL still returns `i64` (`COALESCE(SUM,0)`);
  the `MediaStore` impl wraps it via `ByteSize::try_from(sum)` (≥ 0 by
  construction, so the conversion never fails, but is routed through the checked
  door rather than an unchecked wrap) mapping any failure to `sqlx::Error`.

### `web` — DTOs + generic `format_bytes`

- `MediaItem.size_bytes: ByteSize`, `MediaUsageData.used_bytes: ByteSize`
  (import `ByteSize` from `common::media` alongside `MaxFileSize`/`UserQuota`).
  Built directly from the now-typed `MediaRecord.size_bytes` / usage return
  (drop any `i64` plumbing).
- **`UploadResponse.size_bytes: ByteSize`** (`server/src/media.rs`) — a
  serialized web-facing DTO carrying a byte size, the same "half-typed" defect
  this issue names. It's built from the server-local `UploadMetadata.size_bytes`
  (`i64`) in `media_manager.rs`; wrap that measured `i64` into `ByteSize` at
  construction. (Added after the cold review flagged it; see the HALT note.)
- `web/src/render/mod.rs`: `format_bytes(bytes: impl Into<i64>) -> String` (was
  `bytes: i64`). Generic because it formats **both** actual sizes (`ByteSize`)
  and the limit newtypes (`MaxFileSize`, `UserQuota`) — all three have
  `From<Self> for i64`. This kills the `.value()` at every call site
  (`web/src/media/component.rs`: `format_bytes(u.used_bytes)`,
  `format_bytes(u.quota_bytes)`, `format_bytes(u.max_file_size_bytes)`,
  `format_bytes(item.size_bytes)`), reading as domain statements.
- The percentage math in `component.rs`
  (`used_bytes as f64 / quota_bytes.value() as f64`) becomes
  `used_bytes.value() as f64 / …`.

### `server` — construction + quota arithmetic (compiler-forced)

`media_manager.rs` constructs `MediaRecord { size_bytes, … }` from a measured
upload size (an `i64` from `bytes.len()`/`bytes_written`) and calls
`get_user_upload_usage`. Forced by the type change:

- Where it builds the record: wrap the measured `i64` into `ByteSize` (via
  `try_from`, clamping-free; an `i64::MAX` fallback size is still a valid
  `ByteSize`).
- `check_quota`: `get_user_upload_usage` now returns `ByteSize`, so the
  `current_usage + new_size > quota.value()` arithmetic uses
  `current_usage.value()`. The per-file `size > max.value()` checks are
  unchanged (those sizes stay local `i64`).

## Design decisions to confirm at the HALT

1. **Comparison helpers (issue's "Consider whether") — deferred.** The
   quota/size checks (`media_manager.rs`) are few and read clearly with
   `.value()`. Adding e.g. `MaxFileSize::admits(ByteSize)` would couple the
   limit types to `ByteSize` for marginal gain; I'd rather keep this change
   focused. Say the word to add them.
2. **`format_bytes` generic** (`impl Into<i64>`) vs strictly `ByteSize` — I
   chose generic because the same fn formats the limit newtypes too;
   strictly-`ByteSize` would force `ByteSize::try_from(quota.value())` at
   limit-format sites. Flag if you'd prefer the strict signature.

## Out of scope

- `MaxFileSize`/`UserQuota` themselves (already typed, #536).
- The per-file upload-size _checks_ in `media_manager` stay `i64` locals (a
  transient measurement, not a stored/returned domain value) — only the stored
  record field and the usage return are typed.
- Backup byte sums (`server/src/backup.rs`, a `u64` filesystem walk) — unrelated
  domain.

## Tests

- `macros`: a unit test drives the new `TryFrom<inner>` positive branch (an
  in-bounds integer wraps) and the out-of-bounds rejection (returns
  `Invalid<Name>`), on a local fixture `NumNewtype` (the crate is
  coverage-measured, so both branches must be hit at runtime).
- `common::media`: **`ByteSize` gets its own test — it CANNOT reuse the existing
  `assert_positive_byte_newtype` helper**, which hardcodes `min = 1` semantics
  (asserts `"0".parse().is_err()`, serde-rejects `"0"`, and requires
  `T: Default`). `ByteSize` has `min = 0` (accepts `0`) and no `default`. A
  dedicated test: parse accept `"0"`/positive, reject negative/non-integer with
  the domain message, serde round-trip, `From<ByteSize> for i64`, and
  `TryFrom<i64>` accept-0 / reject-negative. (Generalizing the shared helper to
  a min-parameterized form is possible but out of scope — a dedicated test is
  simpler.)
- `storage`: the dual-backend media tests already round-trip `size_bytes`
  through the DB; they now exercise the `ByteSize` decode boundary. A
  negative-size decode-error test is optional (mirrors the `ContentHash`
  corruption test) — add if coverage needs it.
- `web`: `format_bytes` unit tests updated to pass `ByteSize`/limit newtypes
  (behavior unchanged — same rendered strings).
- **Test-site sweep (real work, compiler-forced but enumerated so it isn't
  lost).** Many sites construct `MediaRecord { size_bytes: <i64 literal>, … }`,
  mutate `.size_bytes`, or assert `size_bytes`/`used_bytes`/`usage` against
  `i64` literals — across `storage/src/media.rs`,
  `server/tests/{storage/mod,web/web_media,misc/backup_fixture}.rs`, and
  `server/src/media_manager.rs`. Each wraps the literal in `ByteSize` (via a
  `common::test_support` byte-size fixture, per the newtype-test-helper
  convention) and compares against a `ByteSize`. The plan's implementation task
  must sweep these; the gate proves completeness.

## Acceptance

- `ByteSize` in `common::media` (`NumNewtype`, `min = 0`), used for
  `MediaRecord.size_bytes`, `get_user_upload_usage`'s return,
  `MediaItem.size_bytes`, `MediaUsageData.used_bytes`, and
  `UploadResponse.size_bytes`; `format_bytes` takes the typed value(s). No
  raw-`i64` byte-size DTO field remains.
- `NumNewtype` gains a validating `TryFrom<inner>`; its validation/error path is
  covered by a macros unit test.
- `cargo xtask validate --no-e2e` clean.
