# Plan — #579: `ByteSize` newtype for actual byte counts

Spec:
[2026-07-22-issue-579-bytesize-newtype.md](../specs/2026-07-22-issue-579-bytesize-newtype.md)
· Issue [#579](https://github.com/jaunder-org/jaunder/issues/579)

## Review header

**Goal.** Introduce `ByteSize` (`NumNewtype`, `i64`, `min = 0`) in
`common::media` and thread it through `MediaRecord.size_bytes`,
`get_user_upload_usage`, and the web DTOs
(`MediaItem`/`MediaUsageData`/`UploadResponse`); `format_bytes` becomes generic.
First add the enabling macro door.

**Scope.** In: `NumNewtype` validating `TryFrom<inner>` (+ macro test);
`ByteSize` + dedicated test + a `test_support` fixture; generic `format_bytes`;
the storage/server/web threading + the test-site sweep. Out: comparison helpers
(deferred); the per-file upload-size _checks_ (local `i64`);
`MaxFileSize`/`UserQuota` (already typed); backup byte sums.

**Tasks.**

1. `NumNewtype`: add validating `TryFrom<inner>` (scene-setting; macro test).
2. `ByteSize` in `common::media` + dedicated test + `common::test_support` byte
   fixture; make `format_bytes` generic (`impl Into<i64>`) —
   backward-compatible.
3. Atomic threading: `MediaRecord.size_bytes` / usage return / web DTOs →
   `ByteSize`; storage boundary conversions; `media_manager` construction +
   quota arithmetic; drop `.value()` at `format_bytes` sites; the test-site
   sweep.

**Key risks / decisions.**

- Task 3 is **atomic** (type-identity swap of `MediaRecord.size_bytes` breaks
  all consumers together) — one broad, compiler-guided commit; good to delegate
  via **jaunder-dispatch**.
- `TryFrom<i64>` is conflict-free precisely because `NumNewtype` omits
  `From<inner>` (no std blanket `TryFrom` collision).
- `ByteSize` (min 0, no default) needs its **own** test — it can't reuse
  `assert_positive_byte_newtype` (min 1, requires `Default`).

**For agentic workers:** **jaunder-iterate**; Task 3 via **jaunder-dispatch**.

## Global constraints

- No `Co-Authored-By`. `cargo xtask check` clean before commit (hook enforces).
  Task 3 closes with `cargo xtask validate --no-e2e`.
- Backend parity: the `MediaDialect::get_user_upload_usage` change touches both
  sqlite + postgres. Coverage: macros crate is measured (cover both `TryFrom`
  branches).

---

## Task 1 — `NumNewtype` gains a validating `TryFrom<inner>`

**Files** — `macros/src/num_newtype.rs`:

- Add a `try_from_inner_impl(name, err_name, opts)` emitting
  `impl TryFrom<#inner> for #name { type Error = #err_name; fn try_from(v) -> … }`
  that applies the **same** min/max guard as **`from_str_impl`** — i.e. the
  `if v < #m { return Err(#err_name); }` / `if v > #m { … }` flavor that returns
  the error type **directly** (NOT `serde_impl`'s flavor, which returns
  `serde::de::Error::custom(…)` — wrong type for a `TryFrom` body). Returns
  `Ok(#name(v))` in-bounds. Wire it into `expand`'s `quote!` output.
- `macros/src/lib.rs`: extend the existing
  `num_newtype::expand(parse_quote!{…})` token-assertion unit test(s) to assert
  the emitted tokens contain the `TryFrom` impl. **That is the full coverage
  need for the macro fn** — the guard closures execute during every success-path
  `expand` test (e.g. the existing min/max fixture already drives both the `<`
  and `>` arms), so no runtime derive is added here. A proc-macro crate cannot
  invoke its own derive, and a `max`-bearing runtime fixture would leave the
  max-guard region uncovered — **do not create one.** Runtime coverage of the
  _generated_ `TryFrom` body comes from `ByteSize`'s own (min-only) test in
  Task 2.

**Run:** `cargo nextest run -p macros`; `cargo xtask check`.

**Commit:** `feat(macros): NumNewtype validating TryFrom<inner> (#579)`

---

## Task 2 — `ByteSize` + fixture + generic `format_bytes`

**Files**

- `common/src/media.rs`: add
  ```rust
  #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, NumNewtype)]
  #[num_newtype(inner = i64, min = 0, error = "byte size must be a non-negative number of bytes")]
  pub struct ByteSize(i64);
  ```
  plus a dedicated `#[cfg(test)]` test (NOT the `assert_positive_byte_newtype`
  helper, which is min=1/`Default`-requiring). It must hit **every generated
  branch** for coverage: parse accept `"0"`/positive; parse reject
  negative/non-integer (domain message); **`Display`/`to_string` round-trip**;
  serde **accept** round-trip **and** serde **reject** a negative
  (`serde_json::from_str::<ByteSize>("-1").is_err()` — the deserialize min-guard
  arm, only reachable via a negative since `0` is accepted);
  `i64::from(ByteSize)`; and `ByteSize::try_from(0).is_ok()` /
  `try_from(-1).is_err()` (the new door — the min-only guard, no max branch
  since none is declared).
- `common/src/test_support.rs`: a `pub fn parse_byte_size(s: &str) -> ByteSize`
  helper routing through `FromStr` (matching the existing
  `parse_max_file_size`/`parse_user_quota` `parse_<name>(&str)` convention — NOT
  a `try_from(i64)` form), for the downstream sweep.
- `web/src/render/mod.rs`: `format_bytes(bytes: impl Into<i64>) -> String` (was
  `i64`); body `let bytes = bytes.into();`. Backward-compatible — existing
  `i64`/literal callers still compile, so this task is green on its own.

**Run:** `cargo nextest run -p common media::`, `-p web render`;
`cargo xtask check`.

**Commit:** `feat(common): ByteSize NumNewtype + generic format_bytes (#579)`

---

## Task 3 — thread `ByteSize` end-to-end (atomic) + test sweep

One cohesive commit. Organized by area:

**`storage`**

- `storage/src/media.rs`: `MediaRecord.size_bytes: ByteSize`; INSERT
  `.bind(i64::from(record.size_bytes))`; `MediaDialect::get_user_upload_usage`
  twin stays `sqlx::Result<i64>` (raw SUM), but
  `MediaStorage::get_user_upload_usage(…) -> sqlx::Result<ByteSize>` wraps the
  sum via `ByteSize::try_from(sum)` (map failure to `sqlx::Error`). Import
  `ByteSize` from `common::media`.
- `storage/src/helpers.rs::media_record_from_row`: wrap the `i64` `size_bytes`
  column via
  `ByteSize::try_from(size_bytes).map_err(|e| sqlx::Error::ColumnDecode { … })?`
  (mirrors the neighboring `MediaSource` parse), `MediaRow.size_bytes` staying
  `i64`.
- `storage/src/{sqlite,postgres}/media.rs`: `get_user_upload_usage` dialect
  impls unchanged (still return the raw `i64` SUM).

**`server`**

- `media_manager.rs`: where it builds `MediaRecord { size_bytes, … }` from the
  measured `i64`, wrap via `ByteSize::try_from(measured)` (an `i64::MAX`
  fallback is still valid). `check_quota`: `get_user_upload_usage` now returns
  `ByteSize` → use `.value()` in the `current_usage + new_size > quota.value()`
  arithmetic. Where `UploadResponse` is built (`media_manager.rs`), set
  `size_bytes: ByteSize::try_from(metadata.size_bytes)?` (or reuse the record's
  typed value). `media_upload_bytes(u64::try_from(metadata.size_bytes)…)` reads
  the server-local `UploadMetadata.size_bytes` (`i64`) — unchanged.
- `server/src/media.rs`: `UploadResponse.size_bytes: ByteSize` (import
  `ByteSize`).

**`web`**

- `web/src/media/api.rs`: `MediaItem.size_bytes: ByteSize`,
  `MediaUsageData.used_bytes: ByteSize` (add `ByteSize` to the `common::media`
  import); build from the now-typed record / usage return (drop `i64` plumbing).
- `web/src/media/component.rs`: drop `.value()` at the four `format_bytes` sites
  (`used_bytes` / `quota_bytes` / `max_file_size_bytes` / `item.size_bytes` pass
  directly); the percentage math becomes
  `used_bytes.value() as f64 / quota_bytes.value() as f64`.

**Test-site sweep** (wrap `i64` literals via
`common::test_support::parse_byte_size("…")`, compare against `ByteSize`):
`storage/src/media.rs` (`size_bytes: <lit>` ctor sites + the
`assert_eq!(fetched.size_bytes, 12345)` / `assert_eq!(usage, …)` asserts),
`server/tests/{storage/mod,web/web_media,misc/backup_fixture}.rs`,
`server/src/media_manager.rs` (`assert_eq!(first.size_bytes, …)`). Let the
compiler enumerate; fix each.

**Run / final gate**

- `cargo check` across `common`/`storage`/`server`/`web` (+ `--features server`
  for web).
- `cargo nextest run -p storage media::` (dual-backend).
- `cargo xtask validate --no-e2e` — green. Confirm no raw-`i64` byte-size DTO
  field remains
  (`rg -n "size_bytes: i64|used_bytes: i64" web/ server/src storage/src`).

**Commit:**
`refactor(common,storage,server,web): type byte counts as ByteSize (#579)`

## Self-review

- Task 1 self-contained (macros); Task 2 compiles with `ByteSize` unused +
  `format_bytes` backward-compatible; Task 3 is the atomic swap. No
  partial-migration state committed.
- Every acceptance criterion maps: `TryFrom` + coverage → Task 1; `ByteSize`
  defined → Task 2; the DTO/record/usage threading + no raw-`i64` byte DTO field
  → Task 3.
