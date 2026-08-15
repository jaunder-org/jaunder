# Storage Integration Test Layout — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `server/tests/storage/mod.rs` with an assembly-only map over
19 focused test modules while retaining dual-backend behavior and removing only
the spec's assertion-accounted duplicates.

**Architecture:** Make the one body-level prerequisite first, extract a narrow
cross-concern fixture interface, then move one concern per green commit. Leave
the seven cleanup candidates in `mod.rs` until every retained test has moved;
five concern-scoped cleanup commits remove or split them, with the last making
`mod.rs` assembly-only. Update contributor guidance and run the full verify-only
gate last.

**Tech Stack:** Rust integration tests, `rstest`/`rstest_reuse`, SQLx SQLite and
PostgreSQL fixtures, cargo-nextest, cargo xtask.

**Spec:**
[`2026-08-14-issue-950-storage-test-layout.md`](../specs/2026-08-14-issue-950-storage-test-layout.md)

## Review header

**Scope — in:** `server/tests/storage/` test/helper relocation, the seven
required cleanup dispositions, one password-reset success test assembled from
existing coverage, and `CONTRIBUTING.md` test-filter guidance.

**Scope — out:** production/storage behavior, traits, dialects, schema,
migrations, new test behavior beyond the retained password-reset success path, a
new framework, an ADR, or a new gate.

**Tasks:**

1. Capture the baseline inventory and make `raw_exec` self-contained.
2. Extract the cross-concern fixture interface.
3. Move database opening and connection-option tests.
4. Move lookup-table correspondence tests.
5. Move subscription tests.
6. Move audience tests.
7. Move viewer-resolution tests.
8. Move composite foreign-key tests.
9. Move site-configuration and configured-mailer tests.
10. Move feed-event tests.
11. Move user and authentication tests.
12. Move session tests.
13. Move invite and invited-registration tests.
14. Move email-verification tests.
15. Move password-reset tests.
16. Move core Post mutation, rendering, revision, and audience tests.
17. Move Post listing, pagination, scheduling, and feed-window tests.
18. Move tag persistence and payload tests.
19. Move media tests.
20. Move user-configuration tests.
21. Remove the three redundant site-configuration smoke tests.
22. Remove the redundant combined user/auth smoke test.
23. Remove the redundant session-lifecycle smoke test.
24. Remove the redundant invited-registration smoke test.
25. Split the combined verification/reset workflow and finish assembly-only
    `mod.rs`.
26. Update contributor guidance and validate the complete branch.

**Key risks/decisions:**

- Macro templates resolve by bare name: every module using `#[apply(backends)]`
  imports `rstest::*`, `rstest_reuse::*`, and bare
  `storage::test_support::backends` locally.
- A move commit may add its `mod` declaration and local imports but does not
  edit moved bodies except for formatter output.
- Cleanup candidates remain in `mod.rs` through the concern moves. Tasks 21–25
  clean one owning concern per green commit; Task 25 removes the last residual
  items and imports so `mod.rs` becomes assembly-only.
- `cargo nextest list -p jaunder` registers exactly **310** baseline
  `storage::...` cases: 154 backend-parametrized functions plus two
  non-parametrized functions. Seven dual-backend removals and one dual-backend
  addition make the final exact count **298**.
- Before extraction, `raw_exec` becomes self-contained with identical backend
  dispatch. `raw_try_exec` then stays private to `fk_constraints.rs` without
  widening or cross-sibling coupling.
- `fixtures.rs` exports only helpers directly consumed by at least two concern
  modules. One-concern helpers remain local.

## Global constraints

- Final `mod.rs`: module documentation/attributes plus exactly 19 private `mod`
  declarations; no items or re-exports.
- Preserve every retained test/helper body, test input, assertion, backend
  setup, isolation rule, and PostgreSQL guard lifetime during relocation.
- Partition `storage::{...}` imports by actual file use; no globbed
  shared-helper imports.
- Every `#[apply(backends)]` remains dual-backend through the shared template.
- Qualified paths may change; leaf names change only for the approved combined
  workflow split or a demonstrably inaccurate name.
- After plan approval and before Task 1, follow `jaunder-commit` to stage this
  spec and plan, gate them, and commit
  `docs: plan storage integration test split (#950)`. Subsequent task commits
  include their plan-checkbox updates.
- Before every commit: tick that task's checkbox, run
  `devtool run -- cargo xtask check`, inspect/stage formatter changes, then
  stage every intended file and commit. The pre-commit hook repeats the cached
  gate.
- No lint suppression and no `Co-Authored-By` trailer.

## File structure

- Modify: `server/tests/storage/mod.rs` — temporary residual cleanup candidates;
  final private module map only.
- Create:
  `server/tests/storage/{fixtures,database,lookups,subscriptions,audiences,resolution,fk_constraints,site_config,feed_events,users_auth,sessions,invites,email_verification,password_reset,posts,listing,tags,media,user_config}.rs`
  — one responsibility per the approved spec.
- Modify: `CONTRIBUTING.md:517-528` — nested storage paths and concern filter.

## Shared fixture interface

`fixtures.rs` produces these `pub(super)` helpers; consumers import only what
they use:

```rust
pub(super) async fn open_pool(base: &TempDir) -> SqlitePool;
pub(super) async fn local_channel_id(backend: Backend, env: &TestEnv) -> ChannelId;
pub(super) async fn channel_id_by_name(
    backend: Backend,
    env: &TestEnv,
    name: &str,
) -> ChannelId;
pub(super) fn username(s: &str) -> Username;
pub(super) fn password(s: &str) -> Password;
pub(super) async fn raw_exec(backend: Backend, env: &TestEnv, sql: &str);
pub(super) async fn anon_by_tag(
    state: &AppState,
    tag: &Tag,
    limit: &str,
) -> Vec<PostRecord>;
pub(super) async fn anon_published(
    state: &AppState,
    limit: &str,
) -> Vec<PostRecord>;
```

`anon_by_tag` is shared by `listing.rs`, `posts.rs`, and `tags.rs`;
`anon_published` by `listing.rs` and `resolution.rs`. `raw_try_exec` stays in
`fk_constraints.rs`; `raw_scalar_i64` stays in `audiences.rs`; `open_pg_pool`
and `lookup_names` stay in `lookups.rs`. All other listing, Post, tag, and media
helpers stay with their single concern.

---

### Task 1: Prepare the extraction and lock the baseline

**Files:**

- Modify: `server/tests/storage/mod.rs` — make `raw_exec` independent of
  `raw_try_exec`; no test changes.

**Interfaces:**

- Consumes: existing `open_pool`, `Backend`, and `TestEnv`.
- Produces: unchanged `async fn raw_exec(Backend, &TestEnv, &str)` behavior
  whose body dispatches directly to SQLite/PostgreSQL; `raw_try_exec` remains
  unchanged for the FK test.

- [x] **Step 1: Capture the registered-test baseline**

Run: `devtool run -- cargo nextest list -p jaunder`

Expected: PASS; the parked output contains exactly 310 lines with
`jaunder::integration storage::`. Immediately copy that parked output to
`.xtask/issue-950-storage-baseline.out`, outside the pruned `.xtask/run/`
directory. This immutable ignored file is the baseline for Task 25.

- [x] **Step 2: Make `raw_exec` self-contained**

Replace its call to `raw_try_exec` with the same backend match currently inside
`raw_try_exec`: SQLite executes against `open_pool(&env.base)`, PostgreSQL
executes against `env.base.pool().postgres()`, both map the query result to
`()`; retain the existing `unwrap_or_else` panic text. Do not change
`raw_try_exec`.

- [x] **Step 3: Prove the preparation**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder -E 'test(/^storage::/)'`

Expected: PASS, 310 storage cases across SQLite and PostgreSQL.

- [x] **Step 4: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): prepare storage test split (#950)`.

---

### Task 2: Extract cross-concern fixtures

**Files:**

- Create: `server/tests/storage/fixtures.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod fixtures;`, move the eight
  shared helpers, add temporary explicit imports needed by residual root tests,
  and prune moved imports.

**Interfaces:**

- Consumes: Task 1's self-contained `raw_exec`.
- Produces: the exact `pub(super)` interface under Shared fixture interface.

- [x] **Step 1: Create the fixture module**

Move the eight helper bodies without editing them. Add `use chrono::Utc;` plus
their required `common`, `sqlx`, `storage`, and `tempfile` imports. Keep helper
imports explicit in every consumer; do not re-export them from `mod.rs`.

- [x] **Step 2: Compile every residual callsite**

Import the moved helpers explicitly into `mod.rs` while residual cleanup tests
still live there. Do not move one-concern helpers.

- [x] **Step 3: Prove the fixture seam**

Run the full storage command from Task 1. Expected: PASS, 310 cases.

- [x] **Step 4: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): extract storage test fixtures (#950)`.

---

### Task 3: Move database tests

**Files:**

- Create: `server/tests/storage/database.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod database;`, cut the three
  tests, prune imports.

**Interfaces:**

- Consumes: shared `backends`; existing `recorded_postgres_url`, `sqlite_url`,
  `DbConnectOptions`, `open_database`.
- Produces:
  `storage::database::{second_open_on_migrated_database_succeeds, postgres_url_is_accepted_at_parse_time, unsupported_url_is_rejected_at_parse_time}`.

- [x] **Step 1: Move the three bodies and minimal imports**

Keep the two plain `#[test]` functions plain. Give the backend-parametrized test
all three required rstest/template imports.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::database`
Expected: PASS for both backends plus both parse-time tests.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split database storage tests (#950)`.

---

### Task 4: Move lookup tests

**Files:**

- Create: `server/tests/storage/lookups.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod lookups;`, cut three tests
  and local `open_pg_pool`/`lookup_names` helpers, prune imports.

**Interfaces:**

- Consumes: shared `open_pool`; `Backend`, `PostgresDbGuard`, `TestEnv`.
- Produces: `channels_bijection`, `target_kinds_bijection`, and
  `statuses_seed_maps_to_enum` under `storage::lookups`.

- [x] **Step 1: Move the lookup cluster**

Move all five bodies unchanged. Keep the PostgreSQL guard bound through each
query.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::lookups`
Expected: PASS, six backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split storage lookup tests (#950)`.

---

### Task 5: Move subscription tests

**Files:**

- Create: `server/tests/storage/subscriptions.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod subscriptions;`, cut five
  tests, prune imports.

**Interfaces:**

- Consumes fixtures `open_pool`, `local_channel_id`, `channel_id_by_name`, and
  `raw_exec`; shared `backends` and `seed_users`.
- Produces the five leaves from `local_channel_id_returns_seeded_local` through
  `pending_subscription_is_not_admitted` under `storage::subscriptions`.

- [x] **Step 1: Move the five test bodies**

Retain the local `StubPending` implementation and backend-specific store setup
inside its test. Use explicit fixture imports.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::subscriptions`
Expected: PASS, ten backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split subscription storage tests (#950)`.

---

### Task 6: Move audience tests

**Files:**

- Create: `server/tests/storage/audiences.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod audiences;`, cut six tests
  and `raw_scalar_i64`, prune imports.

**Interfaces:**

- Consumes fixtures `local_channel_id` and `open_pool`; shared `backends` and
  `seed_users`.
- Produces the six audience leaves from `audience_create_list_rename_delete`
  through `audience_delete_cascades_memberships`; keeps `raw_scalar_i64`
  private.

- [x] **Step 1: Move the audience cluster**

Keep ownership, ordering, and cascade assertions unchanged. Keep
`raw_scalar_i64`'s backend match local.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::audiences`
Expected: PASS, twelve backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split audience storage tests (#950)`.

---

### Task 7: Move viewer-resolution tests

**Files:**

- Create: `server/tests/storage/resolution.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod resolution;`, cut two tests,
  prune imports.

**Interfaces:**

- Consumes fixtures `anon_published`, `channel_id_by_name`, `local_channel_id`,
  and `raw_exec`; shared `backends`/`seed_users`.
- Produces `resolution_matrix` and
  `anonymous_is_not_admitted_by_an_empty_subscriber_ref`.

- [x] **Step 1: Move the resolution bodies**

Preserve the complete viewer matrix, raw channel setup, and fail-closed
assertions.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::resolution`
Expected: PASS, four backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split storage resolution tests (#950)`.

---

### Task 8: Move composite-FK tests

**Files:**

- Create: `server/tests/storage/fk_constraints.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod fk_constraints;`, cut
  `composite_fks_reject_cross_author_membership`,
  `posts_published_at_index_exists`, and `raw_try_exec`; prune imports.

**Interfaces:**

- Consumes fixtures `raw_exec` and `open_pool`; shared `backends`/`seed_users`.
- Produces `composite_fks_reject_cross_author_membership`,
  `posts_published_at_index_exists`, and private
  `raw_try_exec(Backend, &TestEnv, &str) -> Result<(), sqlx::Error>`.

- [x] **Step 1: Move the schema-integrity bodies**

Keep `raw_try_exec`'s backend match, the FK rejection matrix, and the
backend-specific schema-catalog index assertion unchanged.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::fk_constraints`
Expected: PASS, four backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split storage FK tests (#950)`.

---

### Task 9: Move site-configuration tests

**Files:**

- Create: `server/tests/storage/site_config.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod site_config;`, cut
  `build_mailer_returns_noop_when_smtp_not_configured`,
  `site_config_round_trips_through_typed_keys`, and `site_config_operations`;
  leave the three required cleanup candidates in root.

**Interfaces:**

- Consumes shared `backends`, `Backend`, and `TestEnv`.
- Produces three retained leaves under `storage::site_config`.

- [x] **Step 1: Move only retained site-config bodies**

Keep mailer construction in this module because its observable contract is
configuration-driven. Do not move or delete the three root smoke tests yet.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder -E 'test(/^storage::site_config::/)'`
Expected: PASS, six backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split site config storage tests (#950)`.

---

### Task 10: Move feed-event tests

**Files:**

- Create: `server/tests/storage/feed_events.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod feed_events;`, cut
  `feed_events_marks_run`, prune imports.

**Interfaces:**

- Consumes shared `backends`, `Backend`, `TestEnv`, and `fp`.
- Produces `storage::feed_events::feed_events_marks_run`.

- [x] **Step 1: Move the queue test unchanged**

Retain enqueue, claim, and all four mark operations.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::feed_events`
Expected: PASS, two backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split feed event storage tests (#950)`.

---

### Task 11: Move user/authentication tests

**Files:**

- Create: `server/tests/storage/users_auth.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod users_auth;`, cut ten
  retained tests, leave `create_user_duplicate_and_authenticate_work` in root.

**Interfaces:**

- Consumes fixtures `username` and `password`; shared `backends`/`SeedUser`.
- Produces the retained leaves for create/get, duplicate username, three
  authentication paths, profile update, unknown ID, two `set_email` paths, and
  `set_password_authenticate_with_old_returns_invalid_and_new_succeeds`.

- [x] **Step 1: Move the ten retained bodies**

Home `set_email_*` and `set_password_*` here because UserStorage owns those
operations. Keep the combined smoke test in root for Task 22.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::users_auth`
Expected: PASS, twenty backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split user storage tests (#950)`.

---

### Task 12: Move session tests

**Files:**

- Create: `server/tests/storage/sessions.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod sessions;`, cut six retained
  tests, leave `session_lifecycle_works` in root.

**Interfaces:**

- Consumes shared `backends`, `SeedUser`, `create_session_for`, and
  `seed_users`.
- Produces five focused SessionStorage tests plus `session_list_operations`.

- [x] **Step 1: Move the six retained bodies**

Keep token, timestamp, label, user-scoping, and list assertions unchanged.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::sessions`
Expected: PASS, twelve backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split session storage tests (#950)`.

---

### Task 13: Move invite tests

**Files:**

- Create: `server/tests/storage/invites.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod invites;`, cut seven retained
  tests, leave `invite_and_atomic_registration_work` in root.

**Interfaces:**

- Consumes fixtures `username`/`password`; shared `backends` and `SeedUser`.
- Produces `create_invite_and_list_invites_includes_it`, five
  `create_user_with_invite_*` atomic tests, and `invite_list_operations`.

- [x] **Step 1: Move the seven retained bodies**

Home cross-store registration tests here because the invited-registration
operation owns their atomic contract.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::invites`
Expected: PASS, fourteen backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split invite storage tests (#950)`.

---

### Task 14: Move email-verification tests

**Files:**

- Create: `server/tests/storage/email_verification.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod email_verification;`, cut six
  retained tests; leave the combined workflow in root.

**Interfaces:**

- Consumes fixture `raw_exec`; shared `backends`, `SeedUser`, and email/token
  parsers.
- Produces create/use, already-used, expired, unknown, supersession, and corrupt
  stored-email leaves.

- [x] **Step 1: Move the six bodies unchanged**

Keep the corrupt-row raw SQL setup and error assertion intact.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder -E 'test(/^storage::email_verification::/)'`
Expected: PASS, twelve backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split email verification storage tests (#950)`.

---

### Task 15: Move password-reset tests

**Files:**

- Create: `server/tests/storage/password_reset.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod password_reset;`, cut six
  retained tests; leave the combined workflow in root.

**Interfaces:**

- Consumes fixture `password`; shared `backends`, `SeedUser`, and token parser.
- Produces four PasswordResetStorage token tests plus the two atomic
  confirmation error tests.

- [x] **Step 1: Move the six retained bodies unchanged**

Do not create the success-path test yet; Task 25 performs the approved split in
a cleanup commit.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::password_reset`
Expected: PASS, twelve backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split password reset storage tests (#950)`.

---

### Task 16: Move core Post tests

**Files:**

- Create: `server/tests/storage/posts.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod posts;`, cut core Post tests
  and their one-concern helpers, prune imports.

**Interfaces:**

- Consumes fixtures `open_pool` and `anon_by_tag`; shared `backends`,
  `SeedRawPost`, `SeedUser`, and `UpdateRawPost`.
- Produces these leaves: `post_create_and_get_by_id_works`,
  `post_slug_conflict_returns_slug_conflict`,
  `post_update_writes_revision_and_updates_record`,
  `post_update_not_found_returns_error`,
  `post_update_by_non_owner_returns_unauthorized`,
  `update_publish_timestamp_semantics`,
  `post_audiences_are_persisted_and_replaced`, `get_post_audiences_round_trips`,
  `soft_delete_then_operations`, `post_update_invalid_slug`,
  `update_soft_deleted_post`, `get_post_by_id_nonexistent`,
  `post_revisions_created`, all `create_rendered_post_*`,
  `create_post_foreign_key_violation_maps_to_internal`, all `create_posts_*`,
  and both `perform_post_update_*` tests.
- Keeps `update_input` and `post_audience_rows` private.

- [x] **Step 1: Move exactly the named core bodies**

Do not move list/query, tag, media, user-config, or schema-integrity tests.
Preserve the PostgreSQL guard scope in audience helpers.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::posts`
Expected: PASS, forty-four backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split core Post storage tests (#950)`.

---

### Task 17: Move Post listing tests

**Files:**

- Create: `server/tests/storage/listing.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod listing;`, cut listing tests
  and local helpers, prune imports.

**Interfaces:**

- Consumes fixtures `anon_by_tag` and `anon_published`; shared `backends`,
  `SeedRawPost`, and `SeedUser`.
- Produces all five `*_hides_scheduled_until_due` tests; the hybrid-window,
  `list_published_by_user_*`, `list_published_*`, `list_drafts_*`,
  `drafts_list_includes_scheduled_excludes_live`,
  `list_posts_gone_live_between_returns_only_window_with_tags`,
  `feed_urls_needing_catchup_returns_stale_feeds`, every `list_*posts_by_*tag*`
  query/cursor/empty/scoping test, `tag_list_pagination`, `tag_not_found_error`,
  `soft_deleted_posts_excluded_from_tag_list`,
  `draft_posts_excluded_from_tag_list`, `soft_delete_excludes_post_from_lists`,
  `get_by_permalink_soft_deleted`, and
  `list_published_with_cursor_same_timestamp`.
- Keeps `anon_user_by_tag`, `anon_published_by_user`, `drafts_of`, and
  `seed_post_published_at` private.

- [x] **Step 1: Move every Post query-surface body**

Use method/observable concern, not source adjacency: tag-filtered Post queries
belong here; tag mutation and inventory do not.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::listing`
Expected: PASS, fifty-eight backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split Post listing storage tests (#950)`.

---

### Task 18: Move tag tests

**Files:**

- Create: `server/tests/storage/tags.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod tags;`, cut tag tests and
  `tags_of`, prune imports.

**Interfaces:**

- Consumes fixture `anon_by_tag`; shared `backends`, `SeedRawPost`, and
  `SeedUser`.
- Produces tag mutation/normalization/inventory/payload leaves: multiple/empty
  tag sets, case preservation, restating/reconciling sets, numeric and format
  boundaries, ordering, tags across Posts, lifecycle, creation/retrieval,
  normalization, invalid input behavior, display preservation,
  `list_tags_returns_alphabetical_with_prefix`, and `post_record_carries_tags`.
- Keeps `tags_of` private.

- [x] **Step 1: Move exactly the tag-contract bodies**

Do not pull tag-filtered Post query tests back from `listing.rs`.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::tags`
Expected: PASS, forty backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split tag storage tests (#950)`.

---

### Task 19: Move media tests

**Files:**

- Create: `server/tests/storage/media.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod media;`, cut twelve media
  tests and `make_media_record`, move the media imports, prune root imports.

**Interfaces:**

- Consumes shared `backends` and `seed_users`; no fixture helper.
- Produces all MediaStorage leaves from `create_and_get_media` through
  `find_by_hash_returns_any_match`; keeps `make_media_record` private.

- [x] **Step 1: Move the media cluster unchanged**

Preserve typed source URL decoding, invalid-row behavior, source filtering,
usage accounting, and ownership assertions.

- [x] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::media`
Expected: PASS, twenty-four backend cases.

- [x] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split media storage tests (#950)`.

---

### Task 20: Move user-configuration tests

**Files:**

- Create: `server/tests/storage/user_config.rs`.
- Modify: `server/tests/storage/mod.rs` — add `mod user_config;`, cut six tests,
  prune imports.

**Interfaces:**

- Consumes shared `backends` and `SeedUser`.
- Produces all UserConfigStorage leaves from
  `user_config_get_returns_none_when_unset` through
  `user_config_delete_nonexistent_is_ok`.

- [ ] **Step 1: Move the six bodies unchanged**

Retain typed-key, overwrite, delete, and missing-key assertions.

- [ ] **Step 2: Prove the concern**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::user_config`
Expected: PASS, twelve backend cases.

- [ ] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`refactor(server/tests): split user config storage tests (#950)`.

---

### Task 21: Remove redundant site-config smoke tests

**Files:**

- Modify: `server/tests/storage/mod.rs` — delete
  `site_config_set_then_get_roundtrips`, `get_missing_key_returns_none`, and
  `set_overwrites_existing_value`; prune their residual imports.

**Interfaces:**

- Consumes retained replacements in `site_config.rs` exactly as recorded by the
  spec cleanup table.
- Produces no new test; registered storage count falls from 310 to 304.

- [ ] **Step 1: Delete only the three accounted bodies**

Do not alter retained site-config tests.

- [ ] **Step 2: Prove replacement coverage**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::site_config`
Expected: PASS. Run `devtool run -- cargo nextest list -p jaunder`; expected
exactly 304 `storage::` cases and none of the three removed leaf names.

- [ ] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`test(server): remove redundant site config storage tests (#950)`.

---

### Task 22: Remove the redundant user/auth smoke test

**Files:**

- Modify: `server/tests/storage/mod.rs` — delete
  `create_user_duplicate_and_authenticate_work`; prune residual imports.

**Interfaces:**

- Consumes the three named `users_auth.rs` replacements from the spec table.
- Produces no new test; registered storage count falls from 304 to 302.

- [ ] **Step 1: Delete the accounted body**

Keep all retained user/auth tests byte-equivalent apart from formatting.

- [ ] **Step 2: Prove replacement coverage**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::users_auth`
Expected: PASS. List tests; expected 302 storage cases and no removed leaf.

- [ ] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`test(server): remove redundant user auth storage test (#950)`.

---

### Task 23: Remove the redundant session smoke test

**Files:**

- Modify: `server/tests/storage/mod.rs` — delete `session_lifecycle_works`;
  prune residual imports.

**Interfaces:**

- Consumes the three named `sessions.rs` replacements from the spec table.
- Produces no new test; registered storage count falls from 302 to 300.

- [ ] **Step 1: Delete the accounted body**

Do not edit retained session tests.

- [ ] **Step 2: Prove replacement coverage**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::sessions`
Expected: PASS. List tests; expected 300 storage cases and no removed leaf.

- [ ] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`test(server): remove redundant session storage test (#950)`.

---

### Task 24: Remove the redundant invite smoke test

**Files:**

- Modify: `server/tests/storage/mod.rs` — delete
  `invite_and_atomic_registration_work`; prune residual imports.

**Interfaces:**

- Consumes the two named `invites.rs` replacements from the spec table.
- Produces no new test; registered storage count falls from 300 to 298.

- [ ] **Step 1: Delete the accounted body**

Do not edit retained invite tests.

- [ ] **Step 2: Prove replacement coverage**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::invites`
Expected: PASS. List tests; expected 298 storage cases and no removed leaf.

- [ ] **Step 3: Gate and commit**

Run the global per-commit protocol. Commit:
`test(server): remove redundant invite storage test (#950)`.

---

### Task 25: Split verification/reset coverage and complete assembly

**Files:**

- Modify: `server/tests/storage/password_reset.rs` — add the focused success
  test below.
- Modify: `server/tests/storage/mod.rs` — delete
  `email_verification_and_password_reset_work`, remove every residual
  item/import, and retain exactly the 19 private `mod` declarations.

**Interfaces:**

- Consumes fixtures `password`; existing `Backend`, `SeedUser`, `backends`.
- Produces
  `storage::password_reset::confirm_password_reset_changes_credentials`;
  registered storage count remains 298 because one dual-backend test replaces
  one dual-backend test.

- [ ] **Step 1: Add the focused password-reset success contract**

```rust
#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_changes_credentials(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = SeedUser::new().seed(state).await;
    let reset_token = state
        .password_resets
        .create_password_reset(user.user_id, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();

    state
        .atomic
        .confirm_password_reset(&reset_token, &password("new_password123"))
        .await
        .unwrap();

    let authenticated = state
        .users
        .authenticate(&user.username, &password("new_password123"))
        .await
        .unwrap();
    assert_eq!(authenticated.user_id, user.user_id);
}
```

The existing email-verification success test retains the combined test's
verification assertions; the existing reset claim test retains its claimed-ID
assertion; the existing set-email test retains the intervening update contract.

- [ ] **Step 2: Delete the combined body and finish `mod.rs`**

After pruning residual imports, verify visually that only these declarations
remain: `audiences`, `database`, `email_verification`, `feed_events`,
`fixtures`, `fk_constraints`, `invites`, `listing`, `lookups`, `media`,
`password_reset`, `posts`, `resolution`, `sessions`, `site_config`,
`subscriptions`, `tags`, `user_config`, `users_auth`.

- [ ] **Step 3: Prove the split and population**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder storage::password_reset`
Expected: PASS, including both new backend cases.

Run: `devtool run -- cargo nextest list -p jaunder` and record its output path.
Expected: exactly 298 `storage::` cases; no seven removed leaf names; new leaf
exactly twice.

Use `ctx_execute` to compare `.xtask/issue-950-storage-baseline.out` and the
final parked file as exact sorted multisets. From each line retain the path
beginning at `storage::`; from each final path remove its one concern segment
(`storage::<concern>::` becomes `storage::`). From the baseline multiset remove
both backend cases for these seven leaves:

- `site_config_set_then_get_roundtrips`
- `get_missing_key_returns_none`
- `set_overwrites_existing_value`
- `create_user_duplicate_and_authenticate_work`
- `session_lifecycle_works`
- `invite_and_atomic_registration_work`
- `email_verification_and_password_reset_work`

Add the SQLite and PostgreSQL cases for
`confirm_password_reset_changes_credentials`, sort both multisets, and assert
element-for-element equality. Any residual diff fails this task; totals and
allowlist checks alone are insufficient. After an empty diff, remove
`.xtask/issue-950-storage-baseline.out`.

- [ ] **Step 4: Gate and commit**

Run the global per-commit protocol. Commit:
`test(server): split storage verification and reset coverage (#950)`.

---

### Task 26: Update contributor guidance and validate

**Files:**

- Modify: `CONTRIBUTING.md:517-528` — remove the single-file storage statement;
  add `storage::posts::post_create_and_get_by_id_works` as a nested concern
  filter example.

**Interfaces:**

- Consumes: completed 19-module layout and 298-case inventory.
- Produces: accurate contributor documentation and final verification evidence.

- [ ] **Step 1: Update the path guidance**

State that `storage` and `projector` have concern segments. Keep the subsystem
expression example unchanged; replace/add the single-concern example with:
`cargo nextest run -p jaunder storage::posts::post_create_and_get_by_id_works`.

- [ ] **Step 2: Reconcile final structure and population**

Run: `devtool run -- cargo nextest list -p jaunder` Expected: 298 storage cases
with concern-qualified paths.

Inspect `server/tests/storage/mod.rs`: expected exactly 19 private module
entries and no other items. Confirm each exported fixture helper is imported by
at least two concern modules and every `#[apply(backends)]` module has all three
required imports.

- [ ] **Step 3: Commit the contributor guidance**

Run the global per-commit protocol. Commit:
`docs: update storage test filter guidance (#950)`.

- [ ] **Step 4: Validate the committed branch**

Run: `devtool run -- cargo xtask validate` Expected: PASS for static checks,
coverage, and all four `{sqlite,postgres} × {chromium,firefox}` e2e
combinations.

- [ ] **Step 5: Record completion**

Tick Step 4, Step 5, and every remaining plan checkbox. Run the global
per-commit protocol and commit only the plan:
`docs: complete issue 950 implementation plan`.

- [ ] **Step 6: Validate the exact final HEAD**

Run: `devtool run -- cargo xtask validate` Expected: PASS again from the clean,
fully committed tree. Make no file changes after this run. Confirm the branch is
clean and every checkbox is ticked before handoff.
