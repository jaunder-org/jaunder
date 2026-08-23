# Spec — Issue #751: adjacent timestamp column transposition

## Outcome

The three same-typed timestamp decode seams named in #751 stop depending on
positional `DateTime<Utc>` agreement. A reviewer can see the storage column
identity in the Rust type and in the row struct field name, and a wrong
created/updated/generated/expires mapping becomes a compile-time error rather
than a silent runtime swap.

## Load-bearing decisions

- Use both selected directions: named row structs and role-specific timestamp
  newtypes.
- Do not build a column-order static gate for this issue. That avoids a
  SQL-content correspondence checker and leaves ADR-0085's enumerate-not-search
  rule unchanged.
- The role-specific timestamp newtypes are local to the three #751 seams unless
  a future issue generalizes a repo-wide instant-role convention.
- The affected tuple aliases stop being tuple decodes. Replace them with
  `#[derive(sqlx::FromRow)]` structs whose fields are named after the selected
  columns.
- The six timestamp roles are distinct Rust types at the row/mapping seam:
  - session `created_at` vs `last_used_at`,
  - invite `created_at` vs `expires_at`,
  - feed-cache `updated_at` vs `generated_at`.
- Each role type wraps a UTC instant and exposes an explicit conversion back to
  `DateTime<Utc>` for existing public storage records that still carry raw
  chrono timestamps.
- Do not fold #748 into this work. `PostRecord`/`MediaRecord` timestamp
  migration to `UtcInstant` remains separate.
- Record the row-struct plus role-type decision in the issue, not an ADR. It is
  local to this residual unless later work turns role-specific instants into a
  general convention.
- Update the existing `sqlx-newtype-decode`/ADR-0085 residual documentation
  after the three named residual sites are fixed; this is cleanup of stale
  accepted text, not a new ADR.

## Acceptance

- `storage/src/helpers.rs` no longer defines `SessionRow` or `InviteRow` as
  positional tuple aliases containing adjacent bare `DateTime<Utc>` leaves.
- `storage/src/feed_cache.rs` no longer defines `CacheTuple` as a positional
  tuple alias containing adjacent bare `DateTime<Utc>` leaves.
- The three affected decodes use `#[derive(sqlx::FromRow)]` row structs with
  named fields matching the SQL projection.
- The six listed timestamp roles are represented by distinct Rust types at the
  row/mapping seam, so swapping either member of each adjacent pair fails to
  compile.
- Existing externally visible storage/web behavior is unchanged: session records
  still expose `created_at`/`last_used_at`, invite records still expose
  `created_at`/`expires_at`, and feed-cache rows still expose
  `updated_at`/`generated_at` with the same timestamp values.
- The `sqlx-newtype-decode` module doc and ADR-0085 no longer state that the
  three #751 sites are an unresolved adjacent-`DateTime<Utc>` residual.
- `cargo xtask check --no-test` passes, including `sqlx-newtype-decode`.

## Boundaries

- No new static gate in this issue.
- No new ADR unless implementation discovers a reusable cross-repo convention
  rather than the three local row-seam roles; updating existing ADR-0085
  residual text is in scope.
- No broad timestamp migration for records outside the three #751 sites.
- No schema or SQL semantics change; this is a Rust decode/mapping safety
  change.
