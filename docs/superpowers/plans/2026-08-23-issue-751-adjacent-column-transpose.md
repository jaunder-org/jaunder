# Issue #751 Adjacent Timestamp Column Transposition Implementation Outline

> Execute with `jaunder-iterate`; delegate individual slices with
> `jaunder-dispatch` only if useful. This outline exists because #751 touches
> storage decode safety and accepted ADR/static-gate residual documentation.

## Scope

In:

- Replace the three #751 positional tuple decodes with named
  `#[derive(sqlx::FromRow)]` row structs.
- Add six role-specific timestamp wrappers for the three adjacent timestamp
  pairs.
- Keep public storage record and web DTO behavior unchanged.
- Update the existing `sqlx-newtype-decode` module doc and ADR-0085 residual
  text after the residual is removed.

Out:

- No schema or SQL semantics changes.
- No column-order static gate.
- No new ADR.
- No broad #748 `UtcInstant` migration for `PostRecord`/`MediaRecord` or
  unrelated storage records.

## Task outline

- [x] Task 1: Add local role-specific timestamp wrappers
  - Contract: define distinct wrappers for session `created_at`/`last_used_at`,
    invite `created_at`/`expires_at`, and feed-cache
    `updated_at`/`generated_at`. Each wraps `DateTime<Utc>`, has a direct
    `From<DateTime<Utc>>` door, and exposes extraction only at the final public
    record boundary, after a typed row-to-parts mapping has paired each role
    with its matching field.
  - Contract: wrappers must be usable as sqlx decode targets on both backends.
    Prefer the existing `macros::SqlxBridge` shape if it supports
    `DateTime<Utc>` directly; otherwise implement the minimal sqlx
    `Type`/`Decode` delegation needed for these wrappers.
  - Verification: `devtool run -- cargo xtask check --no-test` must compile the
    wrappers and run `sqlx-newtype-decode` without a new allowlist entry for the
    wrappers.

- [x] Task 2: Convert session and invite row aliases to named structs
  - Contract: `SessionRow` and `InviteRow` stop being tuple aliases. They become
    `#[derive(sqlx::FromRow)]` structs with field names matching the SQL
    projections used in `storage/src/sessions.rs`,
    `storage/src/sqlite/sessions.rs`, `storage/src/postgres/sessions.rs`, and
    `storage/src/invites.rs`.
  - Contract: their timestamp fields use the role-specific wrappers, not
    `DateTime<Utc>`. The mapper first builds typed intermediate parts whose
    `created_at`/`last_used_at` and `created_at`/`expires_at` fields also use
    the role-specific wrappers; unwrapping to `DateTime<Utc>` happens only when
    those typed parts are converted into the existing public records. A swap
    between the adjacent row fields and the typed parts must fail to compile.
  - Verification: update helper unit tests to use distinct timestamp values per
    field and assert each public record field receives the intended instant. Run
    the focused storage test lane covering helpers/sessions/invites if
    available; otherwise run `devtool run -- cargo xtask check --no-test` before
    commit.

- [x] Task 3: Convert feed-cache tuple alias to a named struct
  - Contract: `CacheTuple` is replaced with a `#[derive(sqlx::FromRow)]` row
    struct named for feed-cache storage. Its fields match
    `SELECT feed_url, body, etag, content_type, updated_at, generated_at`.
  - Contract: `updated_at` and `generated_at` use distinct role-specific
    wrappers. The mapper first builds typed feed-cache parts with matching
    role-specific fields; unwrapping to `DateTime<Utc>` happens only at the
    final `FeedCacheRow` construction boundary. A swap between the adjacent row
    fields and the typed parts must fail to compile.
  - Verification: update feed-cache tests to construct distinct `updated_at` and
    `generated_at` values and assert round-trip preserves their identity. Run
    the focused storage test lane covering feed-cache if available; otherwise
    run `devtool run -- cargo xtask check --no-test` before commit.

- [x] Task 4: Remove stale residual documentation
  - Contract: edit `xtask/src/steps/sqlx_newtype_decode_check.rs` module docs so
    they no longer list `SessionRow`, `InviteRow`, or `CacheTuple` as adjacent
    `DateTime<Utc>` residuals tracked by #751.
  - Contract: edit ADR-0085's `sqlx-newtype-decode` conformance paragraph so it
    no longer says two adjacent `DateTime<Utc>` columns transpose invisibly as
    an unresolved #751 residual. The update records that #751 removed the named
    residual by row structs plus role-specific timestamp wrappers, without
    changing ADR-0085's gate doctrine.
  - Verification: `devtool run -- cargo xtask check --no-test` passes, including
    `adr-format`, `adr-view-parity`, and `sqlx-newtype-decode`.

## Risk checks

- Row struct field names must match selected SQL column names exactly; if any
  query aliases a column differently, fix the SQL projection locally rather than
  relying on positional decode.
- Do not leave tuple-style destructuring for the six timestamp fields; that
  recreates the transposition seam after decode.
- Keep the role wrappers through an intermediate typed parts seam; unwrapping
  them immediately after decode recreates a same-typed manual mapping hazard.
- Do not expose the local role wrappers as new public domain vocabulary unless
  implementation reveals a reusable convention; the issue decision explicitly
  keeps this local.
- Do not add or retain a `sqlx-newtype-decode` allowlist entry for the fixed
  residual. A green stale-entry check is part of the proof.
- Use distinct timestamps in tests for each adjacent pair; equal `now` values
  cannot detect a swap.
