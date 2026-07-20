# Plan — issue #536: media byte-limit numeric newtypes

Spec:
[`2026-07-20-issue-536-media-byte-limits.md`](../specs/2026-07-20-issue-536-media-byte-limits.md).
This plan is "how"; the spec is "what/why".

## Review header

**Goal.** Adopt the shipped `NumNewtype` macro for the two media config limits:
`MaxFileSize` / `UserQuota` (`i64`, min-1), threaded through the config read
path + `MediaUsageData` DTO, retiring `get_int`.

**Scope — in:** the two newtypes + tests/helpers; typed storage getters;
`get_int` deletion; threading through `media_manager`, the DTO, `media_usage`,
and the `component.rs` display. **out:** `used_bytes` (stays `i64`); any media
admin form (none exists); `PageSize` (#537).

**Tasks (one line each):**

1. `MaxFileSize` / `UserQuota` newtypes + `parse_*` helpers + unit tests
   (`common`).
2. Atomic: typed storage getters + delete `get_int` + thread
   `media_manager`/DTO/`media_usage`/`component` + retire `DEFAULT_*` consts +
   fix touched tests.
3. Full-gate pass + close-out.

**Key risks / decisions.**

- **Atomic compile at Task 2.** Deleting `get_int` and changing `get_limits` /
  `MediaUsageData` field types breaks `server`/`web` until every consumer
  updates, so Task 2 is one commit across `storage` + `server` + `web`. Task 1
  compiles standalone (new `pub` types; the `NumNewtype` derive entry is already
  covered by `RetentionCount`/feeds on `main`).
- `used_bytes` stays `i64` (can be 0). Edge `.value()` only at the size/quota
  comparisons (`media_manager`) and the `component.rs` display.
- No macro change, no ADR change, no gate.

**For agentic workers:** drive with `jaunder-iterate`; delegate a task to
`jaunder-dispatch` if useful. Tick checkboxes live.

## Global constraints

- No `Co-Authored-By` trailer. Each task ends green (`cargo xtask check` before
  commit; `jaunder-commit`). Crate names: `common`, `storage`, server =
  `jaunder`.
- Storage getter tests keep the dual-backend `#[apply(backends)]` template.
- `cfg(test)` newtype construction via `common::test_support::parse_*` helpers.

---

## Task 1 — `MaxFileSize` / `UserQuota` (`common`)

**Files**

- `common/src/media.rs` — the two types (spec Part 1), each
  `#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::NumNewtype)]` +
  `#[num_newtype(inner = i64, min = 1, default = <52_428_800 | 1_073_741_824>, error = "…")]`.
  Add `use macros::NumNewtype;` (the file currently imports only `StrNewtype`).
  In-module `#[cfg(test)] mod tests`: parse accept, reject `0`/`-1`/non-integer,
  `Default::value()` = 50 MiB / 1 GiB, `Display` round-trip, serde bare-integer
  round-trip + wire-reject `0`, and `i64::from(default)` (covers the generated
  `From<Self> for i64`). Cover **both** types (each is a distinct
  monomorphization). The type's own tests construct inline
  (`"5".parse::<MaxFileSize>()`), like `feed/settings.rs`.

The `common::test_support::parse_max_file_size` / `parse_user_quota` helpers are
**deferred to Task 2** (where the storage tests use them) — added here they'd be
uncovered.

**Run**

- `cargo nextest run -p common media` → **PASS**.
- `cargo xtask check` clean → commit.

## Task 2 — typed getters, retire `get_int`, thread the types (atomic)

One commit; the whole workspace compiles at its end.

**Files — `common`**

- `common/src/test_support.rs` — add `parse_max_file_size(&str) -> MaxFileSize`
  / `parse_user_quota(&str) -> UserQuota` (siblings of `parse_feed_min_items`) +
  the `use crate::media::{MaxFileSize, UserQuota};` import (used by the storage
  getter tests below).

**Files — `storage`**

- `storage/src/site_config.rs` —
  - Add `get_media_max_file_size(&self) -> sqlx::Result<MaxFileSize>` and
    `get_media_user_quota(&self) -> sqlx::Result<UserQuota>`:
    `.get(KEY).await?.as_deref().and_then(|v| v.parse().ok()).unwrap_or_default()`.
  - **Delete `get_int`** (site_config.rs:38) — no remaining callers after this
    task.
  - Import `common::media::{MaxFileSize, UserQuota}`.
  - Dual-backend `#[apply(backends)]` tests for both getters:
    default-when-unset, override value, and falls-back-when-invalid-or-zero
    (mirrors `feeds_min_items_falls_back_when_invalid_or_zero`).
- `storage/src/media.rs` — remove `DEFAULT_MAX_FILE_SIZE_BYTES` /
  `DEFAULT_USER_QUOTA_BYTES` consts (now the macro `default`s); keep the
  `// 50 MiB` / `// 1 GiB` note on the macro attributes. Keep the `MEDIA_*_KEY`
  consts. Audit `media.rs` tests that referenced the removed consts.

**Files — `server`**

- `server/src/media_manager.rs` —
  - `get_limits(&self) -> anyhow::Result<(MaxFileSize, UserQuota)>` via the
    typed getters (drop the `DEFAULT_*` args).
  - `stream_to_temp(…, max_file_size: MaxFileSize)`:
    `bytes_written > max_file_size.value()`.
  - `check_quota(…, user_quota: UserQuota)`:
    `current_usage + size_bytes > user_quota.value()`.
  - in-memory path: `size_bytes > max_file_size.value()`.
  - `size_bytes`/`current_usage`/`bytes_written` stay `i64`.
  - Update the `#[cfg(test)]` blocks that
    `cfg.set(MEDIA_MAX_FILE_SIZE_BYTES_KEY, "5")` etc. — the stored strings are
    unchanged (values ≥ 1); assertions unaffected.

**Files — `web`**

- `web/src/media/api.rs` —
  - `MediaUsageData { used_bytes: i64, quota_bytes: UserQuota, max_file_size_bytes: MaxFileSize }`.
  - `media_usage()` sets `quota_bytes`/`max_file_size_bytes` from the typed
    getters (replace the local `.get(KEY)…unwrap_or(DEFAULT)` parses);
    `used_bytes` unchanged.
  - Import the types.
- `web/src/media/component.rs` — the display sites (≈ lines 236–250):
  `u.quota_bytes.value()` (the `> 0` guard and the `as f64` ratio) and
  `format_bytes(u.max_file_size_bytes.value())` /
  `format_bytes(u.quota_bytes.value())`; `format_bytes(u.used_bytes)` unchanged.

**Run**

- `cargo nextest run -p common` → PASS.
- `cargo nextest run -p storage site_config` (SQLite + Postgres) → PASS.
- `cargo nextest run -p jaunder media` → PASS.
- `cargo build --all-features --all-targets` → clean (catches any missed
  consumer, incl. the wasm `component`).
- `cargo xtask check` clean → commit.

## Task 3 — full gate + close-out

- `cargo xtask validate --no-e2e` → clean (foreground `timeout: 600000`; the
  diff can touch the media/web/e2e surface but the wire form is unchanged, so
  e2e is belt-and-suspenders on CI).
- Confirm acceptance: two newtypes via the macro, error/validation covered;
  `get_int` gone; DTO + read path typed; wire unchanged.
- `git status --porcelain` clean.

## Self-review

- Every `get_int`/`DEFAULT_*`/`max_file_size`/`user_quota` site from the spec's
  grep is covered by Task 2 (storage getters, `media_manager`, `api.rs`,
  `component.rs`).
- Coverage: the two newtypes' generated surface (Task 1 unit tests incl.
  `From`); the typed getters + fallback (Task 2 dual-backend tests).
  `component.rs` display is wasm (e2e/`#[component]`-exempt).
- Atomic-compile boundary explicit (Task 2). No placeholders; concrete files +
  exact attributes + run commands.
