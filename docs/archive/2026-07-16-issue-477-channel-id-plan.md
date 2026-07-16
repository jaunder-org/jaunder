# Plan — #477: `ChannelId` newtype

- Spec:
  [2026-07-16-issue-477-channel-id.md](../specs/2026-07-16-issue-477-channel-id.md)
- Issue: [#477](https://github.com/jaunder-org/jaunder/issues/477)

## Commit strategy (two commits, per precedent)

- **Commit 1 — define** `common::ids::ChannelId` + unit test. Unused → green.
- **Commit 2 — thread** through common/storage/server/web + tests. Ripples
  across crates → atomic. Lean on `cargo check --all-features --all-targets`
  **and full `cargo xtask check`** (wasm-clippy for wasm-only pages;
  both-backend tests).

## Task 1 — Define `ChannelId`

- [ ] Append `pub struct ChannelId(i64)` (doc-commented, derives) to
      `common/src/ids.rs`.
- [ ] Unit test (mirror `SubscriptionId`).
- **Verify:** `cargo test -p common ids::`; commit
  `refactor(common): add ChannelId id newtype (#477)`.

## Task 2 — Thread `ChannelId`

Edit-map (lines from `wt-base-issue-477`; verify before editing):

- [ ] **common/visibility.rs** — `ViewerIdentity::Channel.channel_id` (:39);
      `local(user_id, local_channel_id: ChannelId)` (:48, ctor :50);
      `account_viewer(...,     local_channel_id: Option<ChannelId>)` (:63, match
      :64-65); `SubscriptionPolicy::     initial_status` `channel_id` (:99);
      test literal `channel_id: ChannelId::from(7)` (:166). (`subscriber_ref`
      stays String.)
- [ ] **storage/subscriptions.rs** — `SubscriptionRecord.channel_id` (:25);
      `subscribe`/ `unsubscribe` `channel_id` params (trait :44/:52 + impls
      :159/:184); trait `local_channel_id -> Result<ChannelId>` (:76) + impl
      (:242, wrap the `SELECT channel_id` fetch);
      **`LOCAL_CHANNEL_ID: OnceLock<i64>` → `OnceLock<ChannelId>` (:84)** and
      free fn `local_channel_id(...) -> Option<ChannelId>` (:94,
      `.get()`/`.set()` hold `ChannelId`); `list_subscribers` decode tuple wraps
      `channel_id` (:231-233, alongside the already-`SubscriptionId` pos 0);
      binds `.bind(i64::from(channel_id))` at the owned-param sites
      (:167/:174/:189), but **`is_subscriber` (:210) has `&ChannelId`**
      (destructured from `&ViewerIdentity`) → `.bind(i64::from(*channel_id))`;
      the `initial_status(author_user_id, channel_id, subscriber_ref)` call
      (:164). Tests (:269/:273).
- [ ] **storage/post_service.rs** (production) — `:519`
      `ViewerIdentity::local(user_id, 0)` →
      `ViewerIdentity::local(user_id, ChannelId::from(0))` (the `0` placeholder
      keeps its value; author-branch semantics unchanged).
- [ ] **storage/posts.rs** (`resolution_where`, ~:1731-1745) — the
      `ViewerIdentity::Channel {     channel_id, .. }` arm:
      `(author_id, i64::from(*channel_id), subscriber_ref.clone())`. **LEAVE
      `ResolutionBinds.channel: i64` and the `-1` sentinel** (sentinel bind,
      like the sibling `author_id: i64`).
- [ ] **server/web** — `server/atompub/posts.rs:215-216` (flows);
      `web/viewer.rs:55-56` (flows via `account_viewer`);
      `web/subscriptions/mod.rs:60/78/103` locals + `subscribe`/
      `unsubscribe`/`ViewerIdentity::local` calls (flow);
      `web/audiences/mod.rs:244` test literal `channel_id: ChannelId::from(1)`;
      **`web/posts/mod.rs:1016`** mock
      `expect_local_channel_id().returning(|| Ok(ChannelId::from(1)))`.
- [ ] **server/tests** — `server/tests/storage/mod.rs`: change the raw-SQL
      helper `async fn local_channel_id(...) -> i64` (~:152) to `-> ChannelId`
      (wrap the `query_scalar`), which repairs all its i64-typed consumers
      (`ViewerIdentity::local` calls, `assert_eq!(subs[0].channel_id, local)`,
      etc.) at once. Other `server/tests` files source `channel`/`local` from
      the trait method — they flow unchanged.
- **Verify:**
  1. `cargo check --all-features --all-targets` green.
  2. AC2 edit-map struck; grep (`channel_id: i64`, `local_channel_id.*i64`,
     `OnceLock<i64>`) over touched files — each remaining hit is SQL / the
     `ResolutionBinds` sentinel.
  3. **`cargo xtask check` green** (clippy, wasm-clippy, coverage, both-backend
     tests).
- **Commit:** `refactor: thread ChannelId through storage/web (#477)`.

## Task 3 — Ship

- [ ] `cargo xtask validate --no-e2e`; cold-blind pre-merge review; rebase; PR
      (Closes #477); green CI; **HALT for merge approval**.

## AC coverage

| AC  | Task                            |
| --- | ------------------------------- |
| AC1 | Task 1                          |
| AC2 | Task 2 (edit-map)               |
| AC3 | Task 1 serde test + Task 3      |
| AC4 | Task 2 (both backends) + Task 3 |
| AC5 | Task 3                          |
