# Spec — #474: `RevisionId` newtype

- Issue: [#474](https://github.com/jaunder-org/jaunder/issues/474) (sub-issue of
  the umbrella [#457](https://github.com/jaunder-org/jaunder/issues/457))
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (numeric-ID trailer)
- Date: 2026-07-16

## Problem

A post revision's row id (`post_revisions.revision_id`) is modelled as a bare
`i64` on `PostRevisionRecord`. Applies the umbrella's `IdNewtype` pattern
(#471/…/#473) to the last remaining ID class — the revision id.

## Scope — the smallest track

The single Rust site: **`PostRevisionRecord.revision_id: i64` → `RevisionId`**
(`storage/src/posts.rs:111`). Plus defining `common::ids::RevisionId`.

**`PostRevisionRecord` is currently unconstructed.** The `post_revisions` table
IS written on every post update (`{sqlite,postgres}/posts.rs`
`INSERT INTO post_revisions …`), but no query yet reads a row back into
`PostRevisionRecord` (there is no `list_revisions`/`get_revision`). So this
change is a **data-model type improvement / future-proofing**: when a
revision-read query is added later, its decode wraps `RevisionId::from(raw)` at
construction (the standard chokepoint), consistent with every other record.
There is **no** current decode site, test, or wire surface to update.
`post_id`/`user_id` on the struct are already `PostId`/`UserId` (#472/#471); the
string fields are already newtypes.

## Decision

```rust
// common/src/ids.rs
/// A post revision's row id.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype)]
pub struct RevisionId(i64);
```

`PostRevisionRecord.revision_id: RevisionId`. Type-only; behavior and wire
shapes unchanged.

## Acceptance criteria

- **AC1** `common::ids::RevisionId` exists, derived per convention
  (From/Into/Display/FromStr
  - transparent serde), covered by a unit test.
- **AC2** `PostRevisionRecord.revision_id` is `RevisionId`; no bare-`i64`
  revision id remains in a Rust field/param/return (grep `revision_id` → only
  the SQL `post_revisions` strings).
- **AC3** No wire/behavior change (`PostRevisionRecord` is
  `#[derive(Clone, Debug)]` only — not `Serialize`; no query returns it).
- **AC4** Both backends compile (no revision read path exists); no migration.
- **AC5** `cargo xtask validate --no-e2e` clean; e2e green in CI.

## Non-goals

- Adding a revision-read query / surfacing revision history — out of scope; #474
  only types the existing model field.
