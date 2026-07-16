# Plan — #476: `SubscriptionId` newtype

- Spec:
  [2026-07-16-issue-476-subscription-id.md](../specs/2026-07-16-issue-476-subscription-id.md)
- Issue: [#476](https://github.com/jaunder-org/jaunder/issues/476)

## Commit strategy (two commits, per #471/#475 precedent)

- **Commit 1 — define** `common::ids::SubscriptionId` + unit test. Unused →
  green.
- **Commit 2 — thread** through storage + web + tests. The record/trait flips
  ripple across crates → lands atomically. Mechanical; lean on
  `cargo check --all-features --all-targets` **and the full
  `cargo xtask check`** (wasm-clippy catches the wasm-only `MemberChecklist`
  hidden-input site the host check can't — per the #475 lesson).

## Task 1 — Define `SubscriptionId`

- [ ] Append `pub struct SubscriptionId(i64)` (doc-commented, derives) to
      `common/src/ids.rs`.
- [ ] Unit test exercising the generated surface (mirror `UserId`/`AudienceId`).
- **Verify:** `cargo test -p common ids::`; commit
  `refactor(common): add SubscriptionId id newtype (#476)`.

## Task 2 — Thread `SubscriptionId`

Edit-map (line numbers from `wt-base-issue-476`; verify before editing):

- [ ] **storage/subscriptions.rs** — `SubscriptionRecord.subscription_id` (:23);
      trait+impl `subscribe(...) -> Result<SubscriptionId>` (:41/impl);
      `list_subscribers` decode tuple (:231, wrap pos 0 `subscription_id`, LEAVE
      `channel_id`/`subscriber_ref`); the `subscribe` impl's separate
      `SELECT subscription_id` fetch (`query_as::<_,(i64,)>` →
      `.map(|(id,)| SubscriptionId::from(id))`, ~:178). `mockall` mock
      auto-regenerates.
- [ ] **storage/audiences.rs** — `add_member`/`remove_member` `subscription_id`
      params (trait decl :112/:121 + impls :277/:301 — no separate dispatch
      trait); `.bind(i64::from(..))` (:285/:309);
      `list_members(...) -> Vec<SubscriptionId>` (wrap the
      `SELECT subscription_id` decode).
- [ ] **web/audiences/mod.rs** — `SubscriberSummary.subscription_id` (:71);
      build site (:163, flows from `SubscriptionRecord`); `#[server]`
      `add_subscriber_to_audience` (:179) / `remove_subscriber_from_audience`
      (:196) `subscription_id` params;
      `list_audience_members(...) -> WebResult<Vec<SubscriptionId>>` (:208
      area); `MemberChecklist` `.contains(&sub.subscription_id)` (:536),
      `let subscription_id =     sub.subscription_id` (:537), hidden inputs
      `value=i64::from(subscription_id)` (:551/:573); test literal
      `subscription_id: SubscriptionId::from(7)` (:243).
- [ ] **server/tests** — sweep any `subscription_id` literal / mock expectation
      returning a subscription id (`server/tests/web/audiences.rs` `sub_id`,
      storage tests).
- **Verify:**
  1. `cargo check --all-features --all-targets` green.
  2. AC2 edit-map struck; grep (`subscription_id: i64`, `-> .*Vec<i64>`
     subscriber returns, `Result<i64` subscribe) over touched files — each
     remaining hit is SQL / channel / non-subscription.
  3. **`cargo xtask check` green** (clippy, **wasm-clippy**, coverage,
     both-backend tests).
- **Commit:** `refactor: thread SubscriptionId through storage/web (#476)`.

## Task 3 — Ship

- [ ] `cargo xtask validate --no-e2e`; cold-blind pre-merge review; rebase; PR
      (Closes #476); green CI; **HALT for merge approval** (per-PR).

## AC coverage

| AC  | Task                            |
| --- | ------------------------------- |
| AC1 | Task 1                          |
| AC2 | Task 2 (edit-map)               |
| AC3 | Task 1 serde test + Task 3      |
| AC4 | Task 2 (both backends) + Task 3 |
| AC5 | Task 3                          |
