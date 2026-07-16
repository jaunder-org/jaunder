# Spec — #477: `ChannelId` newtype

- Issue: [#477](https://github.com/jaunder-org/jaunder/issues/477) (sub-issue of
  the umbrella [#457](https://github.com/jaunder-org/jaunder/issues/457))
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (numeric-ID trailer)
- Date: 2026-07-16

## Problem

A channel's row id crosses as a bare `i64` through `common`/`storage`/`web`,
transposable with any other id. Applies the umbrella's established `IdNewtype`
pattern (#471/#475/#476) to the channel id. **`channel_id` and
`local_channel_id` are the same type** — the latter is the `local` channel's id
— so both become `ChannelId`.

## Decision

Introduce `ChannelId` per the shared convention; thread it through every site
carrying a channel id. Type-only; behavior and wire shapes unchanged.

### The type

Append to `common/src/ids.rs`:

```rust
/// A channel's row id.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype)]
pub struct ChannelId(i64);
```

`common::ids::ChannelId`; `IdNewtype` supplies From/Into/Display/FromStr +
transparent serde. No `Ord`.

### sqlx boundary — convert at the edge

`.bind(i64::from(channel_id))` on writes; decode raw `i64` then wrap
`ChannelId::from(raw)` at the chokepoints (`list_subscribers`'
`SubscriptionRecord` tuple `channel_id` position — note `subscription_id` is
already `SubscriptionId` from #476; the `local_channel_id` impl's
`SELECT channel_id` fetch). Generic bounds untouched; no migration; SQL column
names unchanged.

## Scope

1. **common `visibility.rs`** — `ViewerIdentity::Channel.channel_id: ChannelId`;
   `local(user_id, local_channel_id: ChannelId)`;
   `account_viewer(user_id, local_channel_id: Option<ChannelId>)`;
   `SubscriptionPolicy::initial_status(..., channel_id: ChannelId, ...)`; test
   literal `channel_id: ChannelId::from(7)`.
2. **storage `subscriptions.rs`** — `SubscriptionRecord.channel_id: ChannelId`;
   `subscribe`/`unsubscribe` `channel_id: ChannelId` params (trait + impls); the
   trait `local_channel_id(&self) -> sqlx::Result<ChannelId>` + its impl (wrap
   the `SELECT channel_id` fetch); **the process cache
   `LOCAL_CHANNEL_ID: OnceLock<i64>` → `OnceLock<ChannelId>`** and the free fn
   `local_channel_id(...) -> Option<ChannelId>` (`ChannelId` is
   `Copy`/`'static`, works in `OnceLock`); the `list_subscribers` decode wraps
   `channel_id`; `.bind(i64::from(channel_id))`. Backend dirs
   (`sqlite/postgres/subscriptions.rs`) are SQL strings only — no Rust change.
3. **storage `posts.rs`** (`resolution_where`) —
   `ViewerIdentity::Channel { channel_id }` is now `ChannelId`; convert at
   extraction: `(author_id, i64::from(*channel_id), subref)`.
   **`ResolutionBinds.channel` STAYS `i64`** (a sentinel-bearing query bind:
   `-1` for `Anonymous` is not a real channel id) — mirroring the sibling
   `ResolutionBinds.author_id: i64`, which #471 deliberately left `i64` for the
   same reason.
4. **storage `post_service.rs`** (production) — `:519`
   `ViewerIdentity::local(user_id, ChannelId::from(0))` (the `0` placeholder
   keeps its value).
5. **server/web** — `server/atompub/posts.rs` (`local_channel_id` flows into
   `ViewerIdentity::local`); `web/viewer.rs` (`local_channel_id(...)` →
   `Option<ChannelId>` flows into `account_viewer`); `web/subscriptions/mod.rs`
   (`local_channel_id()` → `ChannelId` flows into
   `subscribe`/`unsubscribe`/`ViewerIdentity::local`); `web/audiences/mod.rs` +
   `web/posts/mod.rs` mock/test literals `ChannelId::from(1)`.

**Do not over-reach:** `subscription_id` (already `SubscriptionId` #476),
`author_user_id` (`UserId`), `audience_id` (`AudienceId`), `subscriber_ref`
(`String`, polymorphic), `post_id` (`PostId`). **The `-1`/sentinel query-bind
fields in `ResolutionBinds` stay `i64`** (not a real id) — the only place a
bare-`i64` channel value legitimately remains.

## Acceptance criteria

- **AC1** `common::ids::ChannelId` exists, derived per convention.
- **AC2** No channel-id field/param/return is bare `i64` **except** the
  `ResolutionBinds` sentinel binds and SQL strings; the plan edit-map is the
  completeness surface.
- **AC3** Wire byte-identical (`ChannelId` serializes as a bare integer;
  `ViewerIdentity` is an internal domain type, not a `#[server]` wire DTO — the
  change is compile-only); tests pass.
- **AC4** Both backends compile and pass; no migration.
- **AC5** `cargo xtask validate --no-e2e` clean; e2e green in CI.

## Tests

Construct via `ChannelId::from(n)`. Update `SubscriptionRecord`/`ViewerIdentity`
construction sites and the `local_channel_id` tests. No new behavioral tests.

## Risks

- The `OnceLock` cache and the `ResolutionBinds` sentinel are the two
  non-mechanical spots — both resolved above (cache holds `ChannelId`; sentinel
  binds stay `i64` per the `author_id` precedent). Builds on merged #472
  (`posts.rs` PostId) and #476 (`subscriptions.rs` SubscriptionId) — the
  `subscriptions.rs` decode tuple now wraps two ids (`SubscriptionId` +
  `ChannelId`).
