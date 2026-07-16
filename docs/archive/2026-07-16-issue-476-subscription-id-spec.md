# Spec — #476: `SubscriptionId` newtype

- Issue: [#476](https://github.com/jaunder-org/jaunder/issues/476) (sub-issue of
  the umbrella [#457](https://github.com/jaunder-org/jaunder/issues/457))
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (numeric-ID trailer)
- Date: 2026-07-16

## Problem

A subscription's row id crosses as a bare `i64` through
`common`(none)/`storage`/`web`, transposable with any other id. Applies the
umbrella's established `IdNewtype` pattern (#471, #475) to the subscription id.

## Decision

Introduce `SubscriptionId` per the shared convention; thread it through every
site carrying a subscription id. Type-only; behavior and wire shapes unchanged.
**No reactive-store carve-out** — `SubscriberSummary` is a plain serde DTO (not
`Store`/`Patch`), so its `subscription_id` becomes the newtype directly
(contrast #475's `AudienceSummary`).

### The type

Append to `common/src/ids.rs`:

```rust
/// A subscription's row id.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype)]
pub struct SubscriptionId(i64);
```

`common::ids::SubscriptionId`; `IdNewtype` supplies From/Into/Display/FromStr +
transparent serde. No `Ord`.

### sqlx boundary — convert at the edge (same as #471/#475)

`.bind(i64::from(subscription_id))` on writes; decode raw `i64` then wrap
`SubscriptionId::from(raw)` at the chokepoints (`list_subscribers`'
`SubscriptionRecord` tuple, pos 0; the `subscribe` `RETURNING subscription_id`
scalar; `list_members`' `Vec<i64>` → `Vec<SubscriptionId>` decode). Generic
`i64: Encode/Type` bounds untouched; no migration; SQL column names unchanged.

## Scope

1. **storage `subscriptions.rs`** —
   `SubscriptionRecord.subscription_id: SubscriptionId`; trait+impl
   `subscribe(...) -> sqlx::Result<SubscriptionId>` (the grep-invisible return);
   the `list_subscribers` decode tuple
   `(subscription_id, channel_id, subscriber_ref, …)` wraps pos 0 (**leave
   `channel_id: i64` → #477, `subscriber_ref: String`**). The
   `mockall::automock` mock regenerates from the trait.
2. **storage `audiences.rs`** — `add_member`/`remove_member`
   `subscription_id: SubscriptionId` params (object + dispatch trait + impls);
   `list_members(...) -> Vec<SubscriptionId>` (wrap the `SELECT subscription_id`
   decode); `.bind(i64::from(subscription_id))`.
3. **web `audiences/mod.rs`** —
   `SubscriberSummary.subscription_id: SubscriptionId` (plain serde, no
   carve-out); `#[server]` `add_subscriber_to_audience`/
   `remove_subscriber_from_audience` `subscription_id: SubscriptionId`;
   `list_audience_members(...) -> WebResult<Vec<SubscriptionId>>`;
   `MemberChecklist` (`member_ids: Vec<SubscriptionId>`,
   `.contains(&sub.subscription_id)`, hidden-input
   `value=i64::from(subscription_id)` — Leptos `value=` needs
   `IntoAttributeValue`); test literal
   `subscription_id: SubscriptionId::from(7)`.
4. **backend dirs** (`sqlite/postgres/subscriptions.rs`) — SQL-string constants
   only (`SELECT subscription_id …`); the decode/wrap lives in the generic
   `subscriptions.rs` impl, so **no Rust change** here.

**Do not over-reach:** `channel_id`/`local_channel_id` (→ #477),
`subscriber_ref` (`String`, polymorphic user-ref per ADR-0020), `audience_id`
(already `AudienceId`, #475), `author_user_id` (already `UserId`, #471),
`post_id` stay as-is. The `posts.rs` `subscription_id` is a SQL JOIN string only
(unchanged).

## Acceptance criteria

- **AC1** `common::ids::SubscriptionId` exists, derived per convention.
- **AC2** No subscription-id field/param/return is bare `i64` (incl.
  `subscribe`'s return and the `list_members`/`list_audience_members` `Vec`
  returns); plan edit-map is the completeness surface; SQL strings excepted.
- **AC3** Wire byte-identical (`SubscriptionId`/`Vec<SubscriptionId>` serialize
  as bare integers); existing tests pass unchanged.
- **AC4** Both backends compile and pass; no migration.
- **AC5** `cargo xtask validate --no-e2e` clean; e2e green in CI.

## Tests

Construct via `SubscriptionId::from(n)`. Update the `mockall` expectations and
any `SubscriptionRecord`/`SubscriberSummary` construction sites. No new
behavioral tests.

## Risks

- Low. No reactive-store carve-out; no `common` changes beyond `ids.rs`.
  Disjoint from the in-flight #472 (PostId) — the only `posts.rs`
  `subscription_id` is a SQL JOIN string. Builds on merged #475 (audiences
  already speak `AudienceId`/`UserId`).
