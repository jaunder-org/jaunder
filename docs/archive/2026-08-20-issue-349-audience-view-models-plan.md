# Extract audience view-model assembly implementation plan

Spec:
[`docs/superpowers/specs/2026-08-20-issue-349-audience-view-models.md`](../specs/2026-08-20-issue-349-audience-view-models.md)
Issue: [#349](https://github.com/jaunder-org/jaunder/issues/349)

## Review header

**Goal:** Move audience DTO/model assembly out of `#[server]` bodies into a
host-tested `web::audiences::model` leaf, and make subscriber-label resolution a
single SQL projection instead of Rust-side lookup/deduplication.

**Scope in:** `SubscriptionStorage` subscriber-label read projection;
`web::audiences` DTO/model extraction; dual-backend projection tests; focused
model/server tests; existing audience e2e and full `cargo xtask check` proof.  
**Scope out:** new workspace crates, moving DTOs to `common`, adding
`UserStorage` batch lookup, changing endpoint paths/request shapes/JSON
fields/UI behavior, changing audience authorization or membership write
semantics, caching/cross-request memoization, degraded-success swallowed-error
fallback for subscriber-label query failure.

**Tasks:**

1. Add and prove the SQL subscriber-label projection on `SubscriptionStorage`.
2. Add red `web::audiences::model` tests for DTO mapping.
3. Implement `web::audiences::model` and make audience server functions
   delegate.
4. Run focused audience e2e and full check gate.
5. Commit the checked deliverable.
6. Review the completed deliverable.

**Key risks/decisions:**

- SQL is the first tool for the label projection: it is a join over
  `subscriptions`, `channels`, `subscription_statuses`, and `users`.
- Remote/non-local `subscriber_ref` values are opaque even when numeric; the SQL
  must join `users` only when the subscription channel is the seeded `local`
  channel.
- Missing local user row falls back to `subscriber_ref`; query failure fails the
  server function rather than returning partial/raw labels.
- `model.rs` is deliberately vertical-local. A cross-crate presentation-model
  seam is deferred until a second vertical needs it.
- `web::audiences` public DTO names stay stable through re-exports so wasm UI,
  post composer, and server-function callers do not learn a new path.

**For agentic workers:** Execute with `jaunder-iterate`; use `jaunder-dispatch`
only if delegating a whole task. Tick each task checkbox before its commit gate.
Use `devtool run -- ...` for every command whose output or exit status matters.

## Global constraints

- Keep commits focused and omit `Co-Authored-By` trailers.
- Do not add a new crate for this issue.
- Do not move audience DTOs to `common`.
- Do not add `UserStorage::list_users_by_id` or Rust-side subscriber-label
  deduplication.
- Do not catch the subscriber-label projection's database errors to return
  partial/raw labels.
- Do not change endpoint paths, generated server-function types, request JSON
  field names, or audience UI copy/behavior except label resolution correctness
  already required by the spec.
- Use `devtool run -- ...` for every command whose result matters; inspect
  parked output files separately when needed.

## File structure

- `storage/src/subscriptions.rs` — add `SubscriberSummaryRecord`, extend
  `SubscriptionStorage`/`SubscriptionDialect`, implement the projection, and add
  SQL-shape unit checks if needed.
- `storage/src/sqlite/subscriptions.rs` — SQLite projection SQL.
- `storage/src/postgres/subscriptions.rs` — Postgres projection SQL.
- `server/tests/storage/subscriptions.rs` — dual-backend projection behavior
  tests.
- `web/src/audiences/model.rs` — new DTO/model leaf containing `Summary`,
  `SubscriberSummary`, mapping functions, and direct model tests.
- `web/src/audiences/api.rs` — remove inline DTO definitions and mapping;
  delegate `list_mine` / `list_my_subscribers` after auth/context lookup.
- `web/src/audiences/mod.rs` — declare/re-export `model` DTOs and keep stable
  `web::audiences::{Summary, SubscriberSummary}` paths.
- `web/src/audiences/component.rs` — adjust imports/internal type paths if the
  DTO source path changes.
- `web/src/audiences/server.rs` — remove obsolete N+1-era mocked `get_user`
  tests or replace them with server-boundary-only tests if still valuable.
- `server/tests/web/audiences.rs` — existing server-function integration tests;
  update only if imports require it.
- `docs/superpowers/specs/2026-08-20-issue-349-audience-view-models.md` and this
  plan — stage with the implementation.

## Task 1: Add and prove SQL subscriber-label projection

**Files:**

- Edit: `storage/src/subscriptions.rs`
- Edit: `storage/src/sqlite/subscriptions.rs`
- Edit: `storage/src/postgres/subscriptions.rs`
- Edit: `server/tests/storage/subscriptions.rs`

**Interfaces:**

- Produces storage record:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubscriberSummaryRecord {
    pub subscription_id: SubscriptionId,
    pub label: String,
}
```

- Produces trait method:

```rust
async fn list_subscriber_summaries(
    &self,
    author_user_id: UserId,
) -> sqlx::Result<Vec<SubscriberSummaryRecord>>;
```

- Produces dialect SQL constant, e.g. `LIST_SUBSCRIBER_SUMMARIES`, with bind
  order `author_user_id`.
- The SQL returns active subscriptions only, ordered by `s.subscription_id`.
- The SQL resolves labels with a local-channel-only `LEFT JOIN users`; non-local
  rows and missing local users use `s.subscriber_ref`.

- [x] **Step 1: Write dual-backend projection tests**

Add tests under `server/tests/storage/subscriptions.rs` using the existing
`#[apply(backends)]` template and helpers.

Cover in one or more tests:

1. local active subscription whose `subscriber_ref` is a real local user id
   returns the user's `username` as `label`;
2. non-local active subscription with numeric-looking `subscriber_ref` returns
   the raw ref, even when a local user with that id exists;
3. local active subscription with no matching user row returns the raw ref;
4. inactive/pending subscriber rows are excluded;
5. returned rows are ordered by `subscription_id`.

Use raw SQL helper(s) to seed a non-local channel and to seed missing-user or
inactive rows only where public storage methods cannot express the case. Keep
backend parity: every behavior test is `#[apply(backends)]`, not a
single-backend unit.

Expected before implementation: compile failure because the trait method and
record do not exist.

- [x] **Step 2: Run the focused storage test red**

```bash
devtool run -- cargo nextest run -p jaunder storage::subscriptions::list_subscriber_summaries
```

Expected: fail/compile error naming the missing storage method/record.

Resume note: skipped as a separate red run after the approved plan because the
implementation was already in progress in this resumed session.

- [x] **Step 3: Implement storage projection**

In `storage/src/subscriptions.rs`:

- add `SubscriberSummaryRecord` near `SubscriptionRecord`;
- add `list_subscriber_summaries` to `SubscriptionStorage`;
- rely on `SubscriptionDialect`'s shared `LIST_SUBSCRIBER_SUMMARIES` default;
- implement the method with `query_as::<_, (SubscriptionId, String)>`, binding
  `author_user_id` once and mapping rows to `SubscriberSummaryRecord`.

Shared SQLite/Postgres SQL shape:

```sql
SELECT s.subscription_id,
       COALESCE(u.username, s.subscriber_ref) AS label
FROM subscriptions s
JOIN subscription_statuses st ON st.status_id = s.status_id
LEFT JOIN users u
  ON s.channel_id = (SELECT channel_id FROM channels WHERE name = 'local')
 AND s.subscriber_ref = CAST(u.user_id AS TEXT)
WHERE s.author_user_id = $1
  AND st.name = 'active'
ORDER BY s.subscription_id
```

If `Username` decodes directly as the second tuple element on both backends,
prefer `(SubscriptionId, Username)` and convert to `String` at the web-model
mapping layer; otherwise keep `(SubscriptionId, String)` because the storage
projection is already presentation-oriented.

- [x] **Step 4: Run the focused storage test green**

```bash
devtool run -- cargo nextest run -p jaunder storage::subscriptions::list_subscriber_summaries
```

Expected: pass on SQLite and Postgres cases.

## Task 2: Add red `web::audiences::model` tests

**Files:**

- Create: `web/src/audiences/model.rs`
- Edit: `web/src/audiences/mod.rs`
- Possibly edit: `web/src/audiences/server.rs` to move/retire old tests after
  model tests exist.

**Interfaces:**

- Produces DTO names `Summary` and `SubscriberSummary` with the same public
  fields as today.
- Produces model functions, exact names may be adjusted during implementation
  but callers should need only:
  - `list_audiences(author_user_id, &dyn AudienceStorage) -> WebResult<Vec<Summary>>`
  - `list_subscribers(author_user_id, &dyn SubscriptionStorage) -> WebResult<Vec<SubscriberSummary>>`

- [x] **Step 1: Add `model` module and DTO shell**

Move or duplicate the DTO definitions into `web/src/audiences/model.rs` enough
for tests to compile target-independent:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Store, Patch)]
pub struct Summary { ... }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubscriberSummary { ... }
```

Keep `Summary`'s `Store`/`Patch` derives and field `#[patch]` attributes intact.
Declare `mod model;` in `web/src/audiences/mod.rs` and re-export the DTOs.

- [x] **Step 2: Write model mapping tests**

Under `#[cfg(all(test, feature = "server"))]` in `model.rs`, use
`MockAudienceStorage` and `MockSubscriptionStorage` to cover:

1. `list_audiences` maps `AudienceRecord { audience_id, name, created_at }` to
   `Summary { audience_id, name }` and preserves order;
2. `list_subscribers` maps `SubscriberSummaryRecord { subscription_id, label }`
   to `SubscriberSummary { subscription_id, label }` and preserves order;
3. projection errors propagate as `WebResult` errors; no swallowed-error
   reporting is expected in this path.

Expected before implementation: compile failure for missing model functions and
missing storage projection mock method until Task 1 is complete.

- [x] **Step 3: Run the focused model tests red**

```bash
devtool run -- cargo nextest run -p web --features server audiences::model
```

Expected: fail until the model mapping functions are implemented.

Resume note: skipped as a separate red run after the approved plan because the
implementation was already in progress in this resumed session.

## Task 3: Implement `web::audiences::model` and delegate server functions

**Files:**

- Edit: `web/src/audiences/model.rs`
- Edit: `web/src/audiences/api.rs`
- Edit: `web/src/audiences/mod.rs`
- Edit if needed: `web/src/audiences/component.rs`
- Edit: `web/src/audiences/server.rs`

**Interfaces:**

- `api::list_mine` and `api::list_my_subscribers` remain the generated
  server-function names and return the same JSON shapes.
- `web::audiences::{Summary, SubscriberSummary}` remain public re-exports.

- [x] **Step 1: Implement model mapping functions**

In `model.rs`, add server-only functions:

- `list_audiences` calls `audiences.list_audiences(author_user_id)` and maps to
  `Summary`;
- `list_subscribers` calls
  `subscriptions.list_subscriber_summaries(author_user_id)` and maps to
  `SubscriberSummary`.

No local-channel lookup, user lookup, deduplication, or swallowed-error
reporting belongs in `model.rs`; the subscriber label is already projected by
storage SQL.

- [x] **Step 2: Delegate from `api.rs`**

Remove the inline `Summary` / `SubscriberSummary` definitions and imports that
only supported inline construction. Import DTOs from `model` and use the
re-export path in `mod.rs` as needed.

Update:

```rust
pub async fn list_mine() -> WebResult<Vec<Summary>>
```

to retrieve auth/context and call the model function.

Update:

```rust
pub async fn list_my_subscribers() -> WebResult<Vec<SubscriberSummary>>
```

to retrieve auth/context and call the model function. This server function no
longer needs `UserStorage` context.

- [x] **Step 3: Remove old mocked N+1-era server tests**

Delete or rewrite the three existing `web/src/audiences/server.rs` tests that
assert `get_user` fallback/swallowed-error behavior. Their behavior is replaced
by dual-backend storage projection tests and model mapping tests. Do not keep
tests that pin per-row user lookup or degraded-success label lookup failure.

- [x] **Step 4: Run focused web model/server tests green**

```bash
devtool run -- cargo nextest run -p web --features server audiences
```

Expected: pass.

## Task 4: Run focused behavior proof and full check gate

**Files:**

- No planned source edits after this task except formatter/gate fixes caused by
  the commands.

**Interfaces:**

- Produces evidence for spec AC5.

- [x] **Step 1: Run focused audience e2e**

```bash
devtool run -- cargo xtask e2e-local audiences.spec.ts
```

Expected: pass. This proves the existing browser-visible audiences workflow
still works through the generated server-function client.

- [x] **Step 2: Run full check gate**

```bash
devtool run -- cargo xtask check
```

Expected: pass. If the gate formats files, inspect and stage only files
belonging to this issue.

## Task 5: Commit the checked deliverable

**Files:**

- Stage only issue #349 files and formatter output from Task 4.

**Interfaces:**

- Produces: one focused commit for issue #349.

- [x] **Step 1: Inspect working tree**

```bash
devtool run -- git status --short
```

Confirm only expected issue #349 files are changed.

- [x] **Step 2: Stage checked files explicitly**

```bash
devtool run -- git add storage/src/subscriptions.rs storage/src/sqlite/subscriptions.rs storage/src/postgres/subscriptions.rs server/tests/storage/subscriptions.rs web/src/audiences/api.rs web/src/audiences/mod.rs web/src/audiences/model.rs web/src/audiences/component.rs web/src/audiences/server.rs xtask/src/steps/sqlx_newtype_decode_check.rs docs/superpowers/specs/2026-08-20-issue-349-audience-view-models.md docs/superpowers/plans/2026-08-20-issue-349-audience-view-models.md
```

If some optional files above were not touched, rerun `git add` with only
existing changed paths; do not stage unrelated files.

- [x] **Step 3: Commit**

```bash
devtool run -- git commit -m "fix(web): extract audience view model assembly"
```

Expected: commit succeeds. If the pre-commit gate changes files, inspect, stage
those issue-owned changes, and amend once.

## Task 6: Review the completed deliverable

**Files:**

- Review: whole branch diff against `origin/main`.

**Interfaces:**

- Produces: final review packet for `jaunder-ship`.

- [x] **Step 1: Capture branch diff**

```bash
devtool run -- git diff origin/main...HEAD
```

- [x] **Step 2: Run `jaunder-review`**

Run the standards/specification review against `origin/main`, with this approved
spec and this plan in scope. Resolve every finding before final ship validation.
