# Spec — #475: `AudienceId` newtype

- Issue: [#475](https://github.com/jaunder-org/jaunder/issues/475) (sub-issue of
  the umbrella [#457](https://github.com/jaunder-org/jaunder/issues/457))
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (numeric-ID trailer)
- Date: 2026-07-16

## Problem

An audience's row id crosses as a bare `i64` through `common`, `storage`, and
`web`, so it can be transposed with any other id (`user_id`, `subscription_id`,
`post_id`). This applies the umbrella's established `IdNewtype` pattern (see
#471) to the audience id.

## Decision

Introduce `AudienceId` per the shared convention and thread it through every
site that carries an audience id. Type-only refactor; behavior and wire shapes
unchanged.

### The type

Append to `common/src/ids.rs` (the shared id-newtype module #471 established):

```rust
/// An audience's row id.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype)]
pub struct AudienceId(i64);
```

Named `common::ids::AudienceId`. `IdNewtype` supplies
`From<i64>`/`Into<i64>`/`Display`/ `FromStr` + transparent-i64 serde. No `Ord`
(no sort/map-key site).

### sqlx boundary — convert at the edge (same as #471)

`.bind(i64::from(audience_id))` on writes; decode raw `i64` then wrap
`AudienceId::from(raw)` at the record-mapping chokepoints (`list_audiences`' row
tuple; the `RETURNING audience_id` scalar on
`create_audience`/`update_audience`; `audience_target_from_row`). The generic
`i64: Encode/Type` bounds are untouched. No migration; SQL column names
unchanged.

## Scope

1. **common** — `ids.rs` defines `AudienceId`; **`visibility.rs`**
   `AudienceTarget::Named(i64)` → `Named(AudienceId)` (the audience-id-carrying
   enum variant).
2. **storage `audiences.rs`** — `AudienceRecord.audience_id: AudienceId`; the
   `AudienceStorage` object trait **and** the `Backend`-generic dispatch trait:
   `create_audience(...) -> sqlx::Result<AudienceId>`, and
   `update_audience`/`delete_audience`/`add_member`/
   `remove_member`/`list_members` `audience_id: AudienceId` params; impls
   (`.bind`, the `list_audiences` decode + `RETURNING` scalars).
3. **storage `posts.rs`** — the post-targeting path: `audience_target_row`
   (extract `i64::from(id)` for the bind), `audience_target_from_row` (wrap the
   decoded `Option<i64>` → `AudienceTarget::Named(AudienceId::from(id))`), and
   the in-file `#[cfg(test)]` `AudienceTarget::Named(n)` literals.
4. **storage `site_config.rs`** — `AudienceTarget::Named(_)` match arms already
   ignore the inner value (default-audience maps `Named` → `"public"`); only the
   test literal `Named(7)` needs `AudienceId::from`.
5. **web** — `web/posts/mod.rs` `AudienceSelection.named: Vec<AudienceId>` and
   the `audience_selection_to_targets`/`targets_to_audience_selection`
   conversions + tests; `web/audiences/mod.rs` `#[server]` fns
   (`create_audience -> WebResult<AudienceId>`;
   `rename_audience`/`delete_audience`/`add_subscriber_to_audience`/`remove_...`/
   `list_audience_members` `audience_id: AudienceId`) and the Leptos components
   (`AudienceHeader`/`MemberChecklist` `audience_id: AudienceId`; hidden-input
   `value=i64::from(audience_id)` — Leptos `value=` needs `IntoAttributeValue`,
   which the id doesn't implement, so bind the primitive there);
   `web/pages/ui.rs`/`web/pages/posts.rs` (`AudienceSelection` picker sites).

**Carve-out — `AudienceSummary.audience_id` stays `i64`.** `AudienceSummary` is
a `reactive_stores` keyed-store row (`Store`/`Patch`) whose `#[store(key)]` it
is. `Patch` requires the field to be `PatchField` — a foreign trait implemented
only for primitives, with no blanket impl or derive; typing it would force
`impl PatchField for AudienceId`, coherent only in `common` (where `AudienceId`
lives) — which would pull a leptos-client dependency (`reactive_stores`) into
the backend-agnostic crate (ADR-0055/0058); in `web` it is an outright orphan
violation. This is the ADR-0063 **external-non-owned-type** flatten (like
`atom_syndication` in #470): the one reactive surface holds the primitive and
converts at its edges — built from `AudienceRecord` (`i64::from`), and
re-wrapped to `AudienceId` at `AudienceRow` where it flows into the typed
components/server fns. Confined to that single field + its store key.

**Do not over-reach:** `subscription_id` (→ #476), `post_id`/`channel_id` in the
same functions stay their own types (post_id is being typed concurrently by
#472). `list_members` still returns `Vec<i64>` of **subscription** ids (→ #476),
not audience ids.

## Acceptance criteria

- **AC1** `common::ids::AudienceId` exists, derived per convention.
- **AC2** No `audience_id` in a Rust field/param/return/`AudienceTarget::Named`
  is bare `i64` (the plan's edit-map is the completeness surface; SQL strings
  excepted).
- **AC3** Wire/serialized shapes byte-identical (`AudienceId` and
  `Vec<AudienceId>` serialize as bare integers); existing e2e/serialization
  tests pass unchanged.
- **AC4** Both backends compile and pass; no migration.
- **AC5** `cargo xtask validate --no-e2e` clean; e2e green in CI.

## Tests

Construct via `AudienceId::from(n)` (no thin helper — per the numeric-id test
convention). Update the
`audience_target_from_row`/`targets_to_audience_selection` unit tests and web
component tests. No new behavioral tests (type-only).

## Risks

- **Concurrent overlap with #472 (PostId)** in `storage/src/posts.rs` and
  `web/src/posts/mod.rs` — both edit the post-audience-targeting path (post_id +
  audience_id co-occur in `replace_post_audiences`, `AudienceSelection`). No
  hard blocker; rebase whichever lands second, touching only the
  `audience_id`/`AudienceTarget::Named` sites here.
- `AudienceTarget` is an internal domain enum (not itself a `#[server]` wire
  type — the wire form is `AudienceSelection`), so the `Named(AudienceId)`
  change is compile-only; the transparent-i64 bridge keeps
  `AudienceSelection.named` byte-identical on the wire.
