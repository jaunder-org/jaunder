# Content Visibility — Layer A Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Layer A of ADR-0020 — the `local` channel, subscriptions, named audiences, per-post audience targeting, and read-time visibility enforcement across the web UI, feeds, and AtomPub — building the `ViewerIdentity` and admission seams forward-compatibly so Layers B/C are purely additive.

**Architecture:** A Channel → Subscription → Audience → resolution chain. Enumerated columns are FK lookup tables (not CHECKs, per ADR-0019's SQLite discipline), each guarded by a bijection test against a Rust enum. Same-owner integrity is enforced by composite foreign keys in the database, not application code. A single resolution rule — "viewer is the author OR some `post_audiences` row admits the viewer" — is expressed once as an `EXISTS` sub-clause and folded into every read path. The viewer is a `ViewerIdentity` (`Anonymous | Channel{channel_id, subscriber_ref}`), wider than Layer A needs, so non-local channels need no signature change. Subscription creation routes through a `SubscriptionPolicy::initial_status` seam that returns `Active` today and becomes M13's approval gate later.

**Tech Stack:** Rust, sqlx (SQLite + Postgres dual-backend), async-trait, Leptos `#[server]` functions + axum extractors, Playwright (Nix VM) for E2E. Dev/CI driver is `cargo xtask`. The verify ladder: `cargo xtask check --no-test` (static + clippy only) is the **fast inner-loop** tool used *while* working a task; full **`cargo xtask check`** (adds instrumented tests incl. Postgres + coverage) is the **pre-commit gate** run before every commit; **`cargo xtask validate`** (adds e2e on sqlite + postgres) is the CI-faithful whole-plan gate.

## Global Constraints

- **Backend parity (ADR-0019):** every table ships as paired `storage/migrations/sqlite/NNNN_*.sql` + `storage/migrations/postgres/NNNN_*.sql`; every storage trait has SQLite **and** Postgres impls, exercised against both backends by the same test (one body, expanded per backend — see the next bullet). Next free migration number is **0018**.
- **Enumerated columns are lookup tables, never CHECK** — a lookup grows with a one-line seed `INSERT` in both backends; a CHECK would force SQLite's 12-step table rebuild. Each lookup is guarded by a bijection test.
- **No application-enforced invariants** for same-owner: composite FKs do it in the DB. Composite FKs require `PRAGMA foreign_keys = ON` on every SQLite connection (Task 1).
- **Fail closed:** losing targeting data makes a post *more* private; resolution admits only `status = 'active'` subscriptions.
- **DI / ADR-0016:** consumers receive `Arc<dyn FooStorage>`, never the whole `AppState`; `AppState` holds only storage. New stores are added to `AppState`, both `make_app_state` constructors, and `provide_app_state_contexts`.
- **No in-file `#[cfg(test)]` tests in per-backend dialect files** (project memory `dialect_files_no_infile_tests`): backend-specific tests live in the backend-parametrized storage integration tests (`server/tests/storage.rs`), not in `sqlite/*.rs` or `postgres/*.rs`. Pure-logic unit tests with no DB (enum round-trips, config parsing) may stay as in-file `#[cfg(test)]` in `common`/the trait module (as `site_config.rs` already does).
- **Storage integration tests — backend parametrization (use this exact shape, not an invented one):** storage tests are **integration tests in `server/tests/storage.rs`**, each expanded to a `::sqlite` and a `::postgres` case by the `rstest_reuse` templates in `server/tests/helpers/mod.rs`. There is **no separate "parity" suite or harness** — since Part 1 of the rstest parameterization (2026-06-19), backend coverage is intrinsic to every storage test (the old `sqlite_X`/`postgres_X` wrapper twins and `*_parity_suite` sequences are gone). A both-backends test is:
  ```rust
  #[apply(backends)]            // both; or #[apply(sqlite_only)] / #[apply(postgres_only)]
  #[tokio::test]
  async fn my_test(#[case] backend: Backend) {
      let env = backend.setup().await;   // TestEnv { state: Arc<AppState>, base: TempDir }
      let state = &env.state;            // exercise state.posts / state.subscriptions / state.audiences …
  }
  ```
  Exercise behaviour through the `AppState` store handles (`state.subscriptions`, `state.audiences`, `state.posts`), never an invented `AnyPool`/`test_pg_pool()`. Tests that genuinely need **raw SQL on both backends** (the lookup bijection reads) use a backend-dispatched pool helper added next to the existing SQLite-only `open_pool(&env.base)` in `server/tests/storage.rs` — add a sibling `open_pg_pool()` (connect to `template_postgres_url()`) and a `lookup_names(backend, &env, table)` dispatcher; **confirm during implementation whether Postgres `setup()` gives each `#[case::postgres]` an isolated DB** (it uses `template_postgres_url()`) before relying on writes through a second pool.
- **TDD, DRY, YAGNI, frequent commits.** Iterate with `cargo xtask check --no-test` (fast: static + clippy), but run the **full `cargo xtask check`** (adds instrumented tests + Postgres + coverage) to validate **before every commit**; gate the whole plan on `cargo xtask validate` (adds e2e). Invoke xtask bare through context-mode (pass/fail = exit code; no `2>&1`/`| tee`/`; echo`).
- **SQLite timestamps:** `TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))`. **Postgres timestamps:** `TIMESTAMPTZ NOT NULL DEFAULT NOW()`. **PK:** SQLite `INTEGER PRIMARY KEY AUTOINCREMENT`, Postgres `BIGSERIAL PRIMARY KEY`. **FK column type:** SQLite `INTEGER`, Postgres `BIGINT`.

## Source specs

- ADR: `docs/decisions/0020-content-visibility-and-subscription-model.md`
- Layer A design: `docs/superpowers/specs/2026-06-18-content-visibility-design.md`
- Layer C design (forward-compat constraints only): `docs/superpowers/specs/2026-06-19-content-visibility-layer-c-design.md`

## File structure (what gets created / modified)

**Migrations (create):**
- `storage/migrations/{sqlite,postgres}/0018_create_visibility_lookups.sql` — `channels`, `subscription_statuses`, `target_kinds` + seeds.
- `storage/migrations/{sqlite,postgres}/0019_create_subscriptions.sql`
- `storage/migrations/{sqlite,postgres}/0020_create_audiences.sql` — `audiences` + `audience_members`.
- `storage/migrations/{sqlite,postgres}/0021_create_post_audiences.sql`

**`common` (shared types):**
- Create `common/src/visibility.rs` — `ViewerIdentity`, `Channel`, `SubscriptionStatus`, `TargetKind` enums (+ `TryFrom`/`as_str`), `AudienceTarget` targeting value type, `SubscriptionPolicy` trait + `OpenSubscriptionPolicy`.
- Modify `common/src/lib.rs` — `pub mod visibility;`.

**`storage` (traits + dual impls):**
- Create `storage/src/subscriptions.rs` (trait + `SubscriptionStore<DB>`), `storage/src/sqlite/subscriptions.rs`, `storage/src/postgres/subscriptions.rs`. (Backend-parametrized tests live in `server/tests/storage.rs`.)
- Create `storage/src/audiences.rs`, `storage/src/sqlite/audiences.rs`, `storage/src/postgres/audiences.rs`.
- Modify `storage/src/posts.rs` (+ `sqlite/posts.rs`, `postgres/posts.rs`) — persist `post_audiences`, add `ViewerIdentity` param + resolution filter.
- Modify `storage/src/sqlite/mod.rs` (Task 1 pragma + `make_app_state`), `storage/src/postgres/mod.rs` (`make_app_state`), `storage/src/app_state.rs`, `storage/src/lib.rs`, `storage/src/site_config.rs` (default-audience key/getter).

**`web` / `server`:**
- Create `web/src/viewer.rs` — `viewer_identity()` extractor (Anonymous | Channel{local,…}).
- Modify `web/src/posts/listing.rs`, `web/src/pages/posts.rs`, `web/src/posts/server.rs` — thread `ViewerIdentity`, audience picker save/load.
- Create `web/src/subscriptions/mod.rs` — Subscribe/Unsubscribe server fns.
- Create `web/src/audiences/mod.rs` — named-audience CRUD + membership server fns.
- Modify `web/src/pages/profile.rs` (Subscribe button), add account audience-management UI, modify post editor.
- Modify `server/src/context.rs` (`provide_app_state_contexts`), `server/src/atompub/posts.rs` (default audience on create/edit), `server/src/feed/regenerate.rs` (Public-only window).

**E2E:**
- Create `end2end/tests/visibility.spec.ts` (or repo's convention).

---

## Phase 1 — Foundation: SQLite foreign keys

### Task 1: Enable `PRAGMA foreign_keys = ON` on every SQLite connection

The composite FKs in Phase 3 are inert unless every pooled SQLite connection has foreign keys on. The app pool currently sets WAL/busy_timeout/cache_size but **not** `foreign_keys` (`storage/src/sqlite/mod.rs:78-104`), and SQLite defaults it off per-connection. A one-shot `execute` (like `cache_size`) only touches one connection — it must be set in `SqliteConnectOptions` so every connection in the pool gets it.

**Files:**
- Modify: `storage/src/sqlite/mod.rs:89-92` (the `options = options.journal_mode(...)` builder chain in `open_sqlite_database` — the **production** pool)
- Modify: `server/tests/storage.rs` `open_pool` (the **test** pool — must mirror production, else FK-dependent tests run with FKs off; SQLite `foreign_keys` is per-connection)
- Test: `server/tests/storage.rs` (`#[apply(sqlite_only)]`, using `open_pool`)

**Interfaces:**
- Produces: every SQLite connection — production *and* test — enforces FKs. No signature change. **This is a prerequisite for Tasks 5/7** (composite FKs are inert with `foreign_keys` off).

- [ ] **Step 1: Write the failing test** in `server/tests/storage.rs` — through the FK-enabled test pool, a child-row insert violating an existing FK is rejected.

```rust
#[apply(sqlite_only)]
#[tokio::test]
async fn sqlite_pool_enforces_foreign_keys(#[case] backend: Backend) {
    let env = backend.setup().await;
    let pool = open_pool(&env.base).await; // same DB file as env.state; FK-enabled by this task
    let result = sqlx::query(
        "INSERT INTO post_revisions (post_id, user_id, title, slug, body, format, rendered_html)
         VALUES (999999, 999999, 't', 's', 'b', 'markdown', '<p>b</p>')",
    )
    .execute(&pool)
    .await;
    assert!(result.is_err(), "FK violation must be rejected when foreign_keys is ON");
}
```

- [ ] **Step 2: Run to verify it FAILS** — `ctx_execute(shell, "cargo test -p jaunder --test storage sqlite_pool_enforces_foreign_keys")`. Expected: insert succeeds → assert fails (FKs currently off on both pools).

- [ ] **Step 3: Implement** — add `.foreign_keys(true)` to the production options chain in `open_sqlite_database`:

```rust
options = options
    .journal_mode(SqliteJournalMode::Wal)
    .busy_timeout(Duration::from_secs(5))
    .foreign_keys(true)
    .log_slow_statements(LevelFilter::Warn, sql_slow_query_threshold());
```

Then mirror it in the test pool so tests match production — in `server/tests/storage.rs::open_pool`, add `.foreign_keys(true)` to the `SqliteConnectOptions` before `connect_with` (it currently sets only `create_if_missing`). Audit other app-facing `SqlitePool::connect_with` sites for the same fix; the backup path (`sqlite/backup.rs`) deliberately toggles FKs itself and is exempt.

- [ ] **Step 4: Run to verify it PASSES** — same command. Expected: PASS.

- [ ] **Step 5: Run the broader suites** to confirm no existing data violates FKs now that they're enforced: `ctx_execute(shell, "cargo test -p storage")` then the storage integration tests `cargo test -p jaunder --test storage`. Fix any fixture that relied on FKs being off.

- [ ] **Step 6: Commit** — `git commit -m "fix(storage): enforce PRAGMA foreign_keys on the SQLite app pool"`

---

## Phase 2 — Lookup tables + enums + bijection tests

### Task 2: Migration `0018` — lookup tables and seeds

**Files:**
- Create: `storage/migrations/sqlite/0018_create_visibility_lookups.sql`
- Create: `storage/migrations/postgres/0018_create_visibility_lookups.sql`

**Interfaces:**
- Produces: tables `channels(channel_id, name)`, `subscription_statuses(status_id, name)`, `target_kinds(kind_id, name)`, seeded `channels←'local'`, `subscription_statuses←'active'`, `target_kinds←'public','subscribers','named'`.

- [ ] **Step 1: Write the SQLite migration**

```sql
CREATE TABLE channels (
    channel_id INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL UNIQUE
);
INSERT INTO channels (name) VALUES ('local');

CREATE TABLE subscription_statuses (
    status_id INTEGER PRIMARY KEY AUTOINCREMENT,
    name      TEXT NOT NULL UNIQUE
);
INSERT INTO subscription_statuses (name) VALUES ('active');

CREATE TABLE target_kinds (
    kind_id INTEGER PRIMARY KEY AUTOINCREMENT,
    name    TEXT NOT NULL UNIQUE
);
INSERT INTO target_kinds (name) VALUES ('public'), ('subscribers'), ('named');
```

- [ ] **Step 2: Write the Postgres migration** (same rows; `BIGSERIAL` PKs):

```sql
CREATE TABLE channels (
    channel_id BIGSERIAL PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE
);
INSERT INTO channels (name) VALUES ('local');

CREATE TABLE subscription_statuses (
    status_id BIGSERIAL PRIMARY KEY,
    name      TEXT NOT NULL UNIQUE
);
INSERT INTO subscription_statuses (name) VALUES ('active');

CREATE TABLE target_kinds (
    kind_id BIGSERIAL PRIMARY KEY,
    name    TEXT NOT NULL UNIQUE
);
INSERT INTO target_kinds (name) VALUES ('public'), ('subscribers'), ('named');
```

- [ ] **Step 3: Verify migrations apply** on both backends — the harness runs both backends' migrations in `backend.setup()`, so `ctx_execute(shell, "cargo test -p jaunder --test storage second_open_on_migrated_database_succeeds")` (an existing harness test) exercises them. Expected: PASS. Commit: `git commit -m "feat(storage): add visibility lookup tables (channels, subscription_statuses, target_kinds)"`

### Task 3: Rust enums + bijection tests

**Files:**
- Create: `common/src/visibility.rs`
- Modify: `common/src/lib.rs` (`pub mod visibility;`)
- Test: bijection tests in `server/tests/storage.rs` (one per lookup), plus the `open_pg_pool()`/`lookup_names()` helpers added there. Enum round-trip unit tests are in-file in `common/src/visibility.rs`.

**Interfaces:**
- Produces: `enum Channel { Local }`, `enum SubscriptionStatus { Active, Pending, Blocked }`, `enum TargetKind { Public, Subscribers, Named }`, each with `fn as_str(&self) -> &'static str` and `impl TryFrom<&str>`. **Note:** `SubscriptionStatus` declares `Pending`/`Blocked` variants now (reserved for M13/M15) but **no seed row exists for them yet** — so the bijection test for statuses asserts the *seeded* set `{active}` maps into the enum, and that every *seeded* name has a variant; it must NOT require every variant to have a row (see Step 3).

- [ ] **Step 1: Write the enums** in `common/src/visibility.rs`:

```rust
//! Shared visibility types: channels, subscription status, audience targeting,
//! the viewer identity, and the subscription-admission seam. See ADR-0020.

use std::fmt;

macro_rules! str_enum {
    ($name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$variant => $s),+ }
            }
        }
        impl TryFrom<&str> for $name {
            type Error = ();
            fn try_from(s: &str) -> Result<Self, ()> {
                match s { $($s => Ok(Self::$variant),)+ _ => Err(()) }
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }
    };
}

str_enum!(Channel { Local => "local" });
str_enum!(SubscriptionStatus { Active => "active", Pending => "pending", Blocked => "blocked" });
str_enum!(TargetKind { Public => "public", Subscribers => "subscribers", Named => "named" });
```

- [ ] **Step 2: Unit test the round-trip** (in `common/src/visibility.rs` `#[cfg(test)]` — `common` is not a dialect file, in-file tests are fine here):

```rust
#[test]
fn target_kind_roundtrips() {
    for k in [TargetKind::Public, TargetKind::Subscribers, TargetKind::Named] {
        assert_eq!(TargetKind::try_from(k.as_str()), Ok(k));
    }
    assert!(TargetKind::try_from("private").is_err());
}
```

- [ ] **Step 3: Write the bijection tests** in `server/tests/storage.rs` using the real harness (`#[apply(backends)]`). The rule: **every seeded row name maps to a variant** (no orphan seed), **and** every variant *that is meant to be seeded in this milestone* has a row. `channels` and `target_kinds` are fully seeded → exact set equality. `subscription_statuses` seeds only `active` now → assert seeded names ⊆ variants and `'active'` present; do not require `pending`/`blocked` rows. First add the raw-read helpers next to `open_pool`:

```rust
// server/tests/storage.rs — sibling of the existing SQLite-only open_pool().
async fn open_pg_pool() -> PgPool {
    PgPool::connect(&template_postgres_url().await).await.unwrap()
}
async fn lookup_names(backend: Backend, env: &TestEnv, table: &str) -> Vec<String> {
    let sql = format!("SELECT name FROM {table} ORDER BY name");
    match backend {
        Backend::Sqlite => sqlx::query_scalar(&sql).fetch_all(&open_pool(&env.base).await).await.unwrap(),
        Backend::Postgres => sqlx::query_scalar(&sql).fetch_all(&open_pg_pool().await).await.unwrap(),
    }
}
```

```rust
#[apply(backends)]
#[tokio::test]
async fn channels_bijection(#[case] backend: Backend) {
    let env = backend.setup().await;
    let names = lookup_names(backend, &env, "channels").await;
    for n in &names { assert!(Channel::try_from(n.as_str()).is_ok(), "unseeded enum for channel {n}"); }
    for c in [Channel::Local] { assert!(names.iter().any(|n| n == c.as_str()), "missing seed for {c}"); }
}
```

Mirror for `target_kinds` (variants `Public`/`Subscribers`/`Named`, exact bijection). For `subscription_statuses`, write `statuses_seed_maps_to_enum` (only the ⊆ direction + `active` present — `Pending`/`Blocked` variants exist without rows yet).

- [ ] **Step 4: Run** — `ctx_execute(shell, "cargo test -p jaunder --test storage bijection")` and `cargo test -p common visibility`. Expected: PASS.

- [ ] **Step 5: Commit** — `git commit -m "feat(common): visibility enums with lookup-table bijection tests"`

---

## Phase 3 — Core tables + composite-FK enforcement

### Task 4: Migration `0019` — `subscriptions`

**Files:**
- Create: `storage/migrations/{sqlite,postgres}/0019_create_subscriptions.sql`

**Interfaces:**
- Produces: `subscriptions(subscription_id, author_user_id, channel_id, subscriber_ref, status_id, created_at)`, unique `(author_user_id, channel_id, subscriber_ref)`, and composite unique `(subscription_id, author_user_id)` to serve as a composite-FK target for `audience_members`.

- [ ] **Step 1: SQLite migration**

```sql
CREATE TABLE subscriptions (
    subscription_id INTEGER PRIMARY KEY AUTOINCREMENT,
    author_user_id  INTEGER NOT NULL REFERENCES users(user_id),
    channel_id      INTEGER NOT NULL REFERENCES channels(channel_id),
    subscriber_ref  TEXT NOT NULL,
    status_id       INTEGER NOT NULL REFERENCES subscription_statuses(status_id),
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (author_user_id, channel_id, subscriber_ref),
    UNIQUE (subscription_id, author_user_id)
);
CREATE INDEX idx_subscriptions_author_status ON subscriptions(author_user_id, status_id);
```

- [ ] **Step 2: Postgres migration** — same with `BIGSERIAL`/`BIGINT`/`TIMESTAMPTZ DEFAULT NOW()`. Keep both `UNIQUE` constraints (the `(subscription_id, author_user_id)` one is required by PG for the downstream composite FK).

- [ ] **Step 3: Verify apply** on both backends; commit `feat(storage): add subscriptions table`.

### Task 5: Migration `0020` — `audiences` + `audience_members` (composite FKs)

**Files:**
- Create: `storage/migrations/{sqlite,postgres}/0020_create_audiences.sql`

**Interfaces:**
- Produces: `audiences(audience_id, author_user_id, name, created_at)` unique `(author_user_id, name)` + composite unique `(audience_id, author_user_id)`; `audience_members(audience_id, subscription_id, author_user_id, PK(audience_id, subscription_id))` with **two composite FKs** both pointing at the *same* `author_user_id` column.

- [ ] **Step 1: SQLite migration**

```sql
CREATE TABLE audiences (
    audience_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    author_user_id INTEGER NOT NULL REFERENCES users(user_id),
    name           TEXT NOT NULL,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (author_user_id, name),
    UNIQUE (audience_id, author_user_id)
);

CREATE TABLE audience_members (
    audience_id     INTEGER NOT NULL,
    subscription_id INTEGER NOT NULL,
    author_user_id  INTEGER NOT NULL,
    PRIMARY KEY (audience_id, subscription_id),
    FOREIGN KEY (audience_id, author_user_id)
        REFERENCES audiences (audience_id, author_user_id),
    FOREIGN KEY (subscription_id, author_user_id)
        REFERENCES subscriptions (subscription_id, author_user_id)
);
```

- [ ] **Step 2: Postgres migration** — identical structure, `BIGINT` columns, `BIGSERIAL` PK on `audiences`. The two composite FKs reference the composite UNIQUEs created in Tasks 4-5.

- [ ] **Step 3: Verify apply** on both backends; commit `feat(storage): add audiences and audience_members with same-owner composite FKs`.

### Task 6: Migration `0021` — `post_audiences`

**Files:**
- Create: `storage/migrations/{sqlite,postgres}/0021_create_post_audiences.sql`

**Interfaces:**
- Produces: `post_audiences(post_id, target_kind_id, audience_id NULL, PK(post_id, target_kind_id, audience_id))`. `audience_id` non-null **iff** kind is `named`. No new column on `posts`. Private = no rows.

- [ ] **Step 1: SQLite migration**

```sql
CREATE TABLE post_audiences (
    post_id        INTEGER NOT NULL REFERENCES posts(post_id),
    target_kind_id INTEGER NOT NULL REFERENCES target_kinds(kind_id),
    audience_id    INTEGER REFERENCES audiences(audience_id),
    PRIMARY KEY (post_id, target_kind_id, audience_id)
);
CREATE INDEX idx_post_audiences_kind_post ON post_audiences(target_kind_id, post_id);
```

> **SQLite note:** a NULL column in a PRIMARY KEY is permitted in SQLite (NULLs are distinct), which is the intended shape — `named` rows carry an `audience_id`, the single `public`/`subscribers` row per post has `audience_id IS NULL`. Postgres treats NULLs in a PK column as not allowed; therefore the Postgres table uses a partial unique-index strategy (Step 2) rather than a literal composite PK including the nullable column.

- [ ] **Step 2: Postgres migration**

```sql
CREATE TABLE post_audiences (
    post_id        BIGINT NOT NULL REFERENCES posts(post_id),
    target_kind_id BIGINT NOT NULL REFERENCES target_kinds(kind_id),
    audience_id    BIGINT REFERENCES audiences(audience_id)
);
-- named rows: one per (post, audience); non-named rows: one per (post, kind).
CREATE UNIQUE INDEX post_audiences_named
    ON post_audiences (post_id, audience_id) WHERE audience_id IS NOT NULL;
CREATE UNIQUE INDEX post_audiences_builtin
    ON post_audiences (post_id, target_kind_id) WHERE audience_id IS NULL;
CREATE INDEX idx_post_audiences_kind_post ON post_audiences (target_kind_id, post_id);
```

- [ ] **Step 3: Verify apply** on both backends; commit `feat(storage): add post_audiences targeting table`.

### Task 7: Composite-FK enforcement test (cross-author rejection)

**Files:**
- Test: `server/tests/storage.rs` (both backends, via `#[apply(backends)]`).

**Interfaces:**
- Consumes: the tables from Tasks 4-6, FKs from Task 1 (SQLite), and the `open_pool`/`open_pg_pool` FK-enabled raw helpers (Tasks 1/3). **No dependency on the Subscription/Audience stores** (Tasks 9/10) — seeding is raw SQL so this task stays in Phase 3.
- Produces: proof the **database** (not app code) rejects pairing an audience with a subscription owned by a different author. This is a deliberately *raw-SQL* test — `audience_members` has no trait insert that bypasses the owner column, so the raw insert is what isolates the FK as the enforcer. (The trait-level complement, `AudienceStorage::add_member` rejecting a cross-author pair, is Task 10.)

- [ ] **Step 1: Write the failing test.** Seed author A's audience and author B's subscription with **raw INSERTs** (ids resolved by natural key inside the membership INSERT, so there is no `RETURNING` vs `last_insert_rowid` divergence to handle). With `author_user_id = A` the `(subscription_id, author_user_id)` FK fails (the subscription is B's); with `B` the `(audience_id, author_user_id)` FK fails (the audience is A's) — either way the DB must reject it.

```rust
#[apply(backends)]
#[tokio::test]
async fn composite_fks_reject_cross_author_membership(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    // Users via the already-wired UserStore; audience + subscription via raw SQL.
    let a = state.users.create_user(&username("alice"), &password("password123"), None, false).await.unwrap();
    let b = state.users.create_user(&username("bob"), &password("password123"), None, false).await.unwrap();

    raw_exec(backend, &env, &format!(
        "INSERT INTO audiences (author_user_id, name) VALUES ({a}, 'Friends')")).await;
    raw_exec(backend, &env, &format!(
        "INSERT INTO subscriptions (author_user_id, channel_id, subscriber_ref, status_id) \
         VALUES ({b}, (SELECT channel_id FROM channels WHERE name='local'), '{b}', \
                 (SELECT status_id FROM subscription_statuses WHERE name='active'))")).await;

    for owner in [a, b] {
        let res = raw_try_exec(backend, &env, &format!(
            "INSERT INTO audience_members (audience_id, subscription_id, author_user_id) VALUES (\
               (SELECT audience_id FROM audiences WHERE author_user_id={a} AND name='Friends'), \
               (SELECT subscription_id FROM subscriptions WHERE author_user_id={b} AND subscriber_ref='{b}'), \
               {owner})")).await;
        assert!(res.is_err(), "cross-author membership must be rejected by the DB (owner={owner})");
    }
}
// raw_exec / raw_try_exec: run a statement on the FK-enabled pool for `backend`
//   (open_pool(&env.base) for SQLite, open_pg_pool() for Postgres). _exec unwraps;
//   _try_exec returns the Result so the test asserts rejection. Small per-backend
//   helpers mirroring open_pool/open_pg_pool. Inlining integer ids via format! is
//   safe here (test-only, no untrusted input) and sidesteps placeholder divergence.
```

- [ ] **Step 2: Run to verify it PASSES** (the FKs already exist) on both backends — `cargo test -p jaunder --test storage composite_fks_reject_cross_author_membership`. If the insert does *not* error, the FK or the SQLite pragma (Task 1, incl. the `open_pool` mirror) is wrong — fix before proceeding.

- [ ] **Step 3: Commit** — `git commit -m "test(storage): composite FKs reject cross-author audience membership"`

---

## Phase 4 — Shared viewer + admission seam

### Task 8: `ViewerIdentity`, `AudienceTarget` targeting type, `SubscriptionPolicy`

**Files:**
- Modify: `common/src/visibility.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum ViewerIdentity {
      Anonymous,
      Channel { channel_id: i64, subscriber_ref: String },
  }
  impl ViewerIdentity {
      /// Local viewer constructor used by Layer A.
      pub fn local(user_id: i64, local_channel_id: i64) -> Self { ... }
  }

  /// What a post is addressed to, as chosen in the editor / API.
  pub enum AudienceTarget { Public, Private, Subscribers, Named(i64) }

  pub trait SubscriptionPolicy: Send + Sync {
      fn initial_status(&self, author_user_id: i64, channel_id: i64, subscriber_ref: &str)
          -> SubscriptionStatus;
  }
  pub struct OpenSubscriptionPolicy;
  impl SubscriptionPolicy for OpenSubscriptionPolicy {
      fn initial_status(&self, _a: i64, _c: i64, _r: &str) -> SubscriptionStatus {
          SubscriptionStatus::Active // Layer A NOOP auto-approve; M13 swaps this here.
      }
  }
  ```
  `channel_id`/`subscriber_ref` are carried (not `Channel` enum) because resolution joins on the numeric `channels.channel_id`; the local channel id is looked up once and cached (Task 11).

- [ ] **Step 1: Write the types** above into `common/src/visibility.rs`.

- [ ] **Step 2: Unit-test the policy NOOP and fail-closed intent:**

```rust
#[test]
fn open_policy_returns_active() {
    assert_eq!(OpenSubscriptionPolicy.initial_status(1, 1, "1"), SubscriptionStatus::Active);
}
```

- [ ] **Step 3: Run** `cargo test -p common visibility`. Expected: PASS. Commit `feat(common): ViewerIdentity, AudienceTarget, SubscriptionPolicy seam`.

---

## Phase 5 — SubscriptionStore

### Task 9: `SubscriptionStore` trait + dual impls + admission seam wiring

**Files:**
- Create: `storage/src/subscriptions.rs` (trait `SubscriptionStorage`, generic `SubscriptionStore<DB>`); backend-parametrized tests go in `server/tests/storage.rs`, not in this file
- Create: `storage/src/sqlite/subscriptions.rs`, `storage/src/postgres/subscriptions.rs`
- Modify: `storage/src/lib.rs` (module + re-exports), `storage/src/app_state.rs` (+ field), `storage/src/sqlite/mod.rs` + `storage/src/postgres/mod.rs` (`make_app_state`), `server/src/context.rs` (`provide_app_state_contexts`)

**Interfaces:**
- Consumes: `Channel`/`SubscriptionStatus`/`ViewerIdentity`/`SubscriptionPolicy` (Task 8), tables (Task 4).
- Produces (mirror `PostStorage` shape):
  ```rust
  #[async_trait]
  pub trait SubscriptionStorage: Send + Sync {
      /// Routes through the admission seam to pick the initial status, then upserts.
      async fn subscribe(&self, author_user_id: i64, channel_id: i64, subscriber_ref: &str)
          -> sqlx::Result<i64>;                         // returns subscription_id
      async fn unsubscribe(&self, author_user_id: i64, channel_id: i64, subscriber_ref: &str)
          -> sqlx::Result<()>;
      async fn is_subscriber(&self, author_user_id: i64, viewer: &ViewerIdentity)
          -> sqlx::Result<bool>;                        // Anonymous → Ok(false)
      async fn list_subscribers(&self, author_user_id: i64)
          -> sqlx::Result<Vec<SubscriptionRecord>>;     // active only
  }
  pub struct SubscriptionRecord {
      pub subscription_id: i64, pub channel_id: i64,
      pub subscriber_ref: String, pub status: SubscriptionStatus, pub created_at: DateTime<Utc>,
  }
  ```
  `SubscriptionStore<DB>` holds the pool **and** an `Arc<dyn SubscriptionPolicy>` (default `OpenSubscriptionPolicy`); `subscribe` calls `policy.initial_status(...)`, resolves the resulting status name to its `status_id`, and inserts with `ON CONFLICT(author_user_id, channel_id, subscriber_ref)` no-op (idempotent subscribe).

- [ ] **Step 1: Write the trait + record** in `storage/src/subscriptions.rs`, no impl yet. Add `pub mod subscriptions; pub use subscriptions::*;` to `lib.rs`.

- [ ] **Step 2: Write failing tests** in `server/tests/storage.rs` using `#[apply(backends)]` + `backend.setup()` + `state.subscriptions`. The tests need the seeded `local` channel id; get it from a **raw test helper** (the trait method `local_channel_id()` is not introduced until Task 11, and using it here would be a forward dependency):

```rust
// sibling of lookup_names (Task 3): raw SELECT of the local channel id.
async fn local_channel_id(backend: Backend, env: &TestEnv) -> i64 {
    let sql = "SELECT channel_id FROM channels WHERE name = 'local'";
    match backend {
        Backend::Sqlite => sqlx::query_scalar(sql).fetch_one(&open_pool(&env.base).await).await.unwrap(),
        Backend::Postgres => sqlx::query_scalar(sql).fetch_one(&open_pg_pool().await).await.unwrap(),
    }
}
```

```rust
#[apply(backends)]
#[tokio::test]
async fn subscribe_is_idempotent_and_active(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = state.users.create_user(&username("alice"), &password("password123"), None, false).await.unwrap();
    let bob = state.users.create_user(&username("bob"), &password("password123"), None, false).await.unwrap();
    let local = local_channel_id(backend, &env).await;
    let id1 = state.subscriptions.subscribe(author, local, &bob.to_string()).await.unwrap();
    let id2 = state.subscriptions.subscribe(author, local, &bob.to_string()).await.unwrap();
    assert_eq!(id1, id2, "subscribe is idempotent");
    assert!(state.subscriptions.is_subscriber(author, &ViewerIdentity::local(bob, local)).await.unwrap());
    assert!(!state.subscriptions.is_subscriber(author, &ViewerIdentity::Anonymous).await.unwrap());
    state.subscriptions.unsubscribe(author, local, &bob.to_string()).await.unwrap();
    assert!(!state.subscriptions.is_subscriber(author, &ViewerIdentity::local(bob, local)).await.unwrap());
}
```
Plus a **fail-closed admission test**: the default `state.subscriptions` uses `OpenSubscriptionPolicy`, so to exercise a `Pending` outcome construct the store directly with a stub policy. This is a pure policy-dispatch + resolution check, so run it `#[apply(sqlite_only)]` against a raw pool:

```rust
#[apply(sqlite_only)]
#[tokio::test]
async fn pending_subscription_is_not_admitted(#[case] backend: Backend) {
    let env = backend.setup().await;
    let pool = open_pool(&env.base).await;                 // same DB file as env.state
    struct StubPending;
    impl SubscriptionPolicy for StubPending {
        fn initial_status(&self, _a: i64, _c: i64, _r: &str) -> SubscriptionStatus { SubscriptionStatus::Pending }
    }
    let store = SqliteSubscriptionStorage::new(pool, Arc::new(StubPending));
    let author = env.state.users.create_user(&username("alice"), &password("pw1234567"), None, false).await.unwrap();
    let bob = env.state.users.create_user(&username("bob"), &password("pw1234567"), None, false).await.unwrap();
    let local = local_channel_id(backend, &env).await;
    store.subscribe(author, local, &bob.to_string()).await.unwrap();
    // resolution admits only `active` → a pending subscriber is excluded (fails closed).
    assert!(!store.is_subscriber(author, &ViewerIdentity::local(bob, local)).await.unwrap());
}
```

- [ ] **Step 3: Run, verify FAIL** (no impl). `ctx_execute(shell, "cargo test -p jaunder --test storage subscribe_is_idempotent")`.

- [ ] **Step 4: Implement** `SqliteSubscriptionStorage` and `PostgresSubscriptionStorage`. SQLite `subscribe`:

```sql
INSERT INTO subscriptions (author_user_id, channel_id, subscriber_ref, status_id)
VALUES (?, ?, ?, (SELECT status_id FROM subscription_statuses WHERE name = ?))
ON CONFLICT (author_user_id, channel_id, subscriber_ref) DO NOTHING;
```
then `SELECT subscription_id FROM subscriptions WHERE (author_user_id, channel_id, subscriber_ref) = (?,?,?)`. `is_subscriber` for a `Channel{channel_id, subscriber_ref}` viewer:

```sql
SELECT EXISTS(
  SELECT 1 FROM subscriptions s
  JOIN subscription_statuses st ON st.status_id = s.status_id
  WHERE s.author_user_id = ? AND s.channel_id = ? AND s.subscriber_ref = ? AND st.name = 'active');
```
`Anonymous` short-circuits to `Ok(false)` without a query. Postgres mirrors with `$n` placeholders and `ON CONFLICT … DO NOTHING`.

- [ ] **Step 5: Run, verify PASS** on both backends.

- [ ] **Step 6: Wire into `AppState`** — add `pub subscriptions: Arc<dyn SubscriptionStorage>` to `app_state.rs`; construct `Arc::new(SqliteSubscriptionStorage::new(pool.clone(), Arc::new(OpenSubscriptionPolicy)))` in both `make_app_state`; add `expect_context`/Extension provisioning in `server/src/context.rs::provide_app_state_contexts`. Build: `ctx_execute(shell, "cargo xtask check")`.

- [ ] **Step 7: Commit** — `git commit -m "feat(storage): SubscriptionStore with admission seam (local channel)"`

---

## Phase 6 — AudienceStore

### Task 10: `AudienceStore` trait + dual impls + tests

**Files:**
- Create: `storage/src/audiences.rs`, `storage/src/sqlite/audiences.rs`, `storage/src/postgres/audiences.rs`
- Modify: `storage/src/lib.rs`, `storage/src/app_state.rs`, both `make_app_state`, `server/src/context.rs`

**Interfaces:**
- Consumes: tables (Task 5); `state.subscriptions` + the `local_channel_id(backend, &env)` test helper (Task 9) to seed subscriptions for membership tests.
- Produces:
  ```rust
  #[async_trait]
  pub trait AudienceStorage: Send + Sync {
      async fn create_audience(&self, author_user_id: i64, name: &str) -> Result<i64, AudienceError>;
      async fn rename_audience(&self, author_user_id: i64, audience_id: i64, name: &str) -> Result<(), AudienceError>;
      async fn delete_audience(&self, author_user_id: i64, audience_id: i64) -> sqlx::Result<()>;
      async fn list_audiences(&self, author_user_id: i64) -> sqlx::Result<Vec<AudienceRecord>>;
      async fn add_member(&self, author_user_id: i64, audience_id: i64, subscription_id: i64) -> Result<(), AudienceError>;
      async fn remove_member(&self, audience_id: i64, subscription_id: i64) -> sqlx::Result<()>;
      async fn list_members(&self, audience_id: i64) -> sqlx::Result<Vec<i64>>; // subscription_ids
  }
  pub struct AudienceRecord { pub audience_id: i64, pub name: String, pub created_at: DateTime<Utc> }
  pub enum AudienceError { DuplicateName, NotFound, Storage(sqlx::Error) }
  ```
  All write methods scope by `author_user_id`; `add_member` passes `author_user_id` so the composite FKs do the same-owner check (no app-level check). `delete_audience` deletes its `audience_members` then the `audiences` row (or relies on cascade — Layer A uses an explicit two-step delete in one transaction, since the migrations declare no `ON DELETE CASCADE`).

- [ ] **Step 1:** trait + records + error in `storage/src/audiences.rs`; register module.

- [ ] **Step 2: Failing tests** (`#[apply(backends)]` in `server/tests/storage.rs`): create→list→rename→delete; duplicate-name → `DuplicateName`; add_member/list_members/remove_member happy path; `add_member` with a cross-author subscription surfaces the FK error as `AudienceError::Storage`/rejected (complements Task 7's raw-SQL test at the trait layer). Run, verify FAIL.

- [ ] **Step 3: Implement** both backends. `create_audience` maps the unique `(author_user_id, name)` violation to `DuplicateName`. Run, verify PASS on both.

- [ ] **Step 4: Wire into `AppState`** + context. `cargo xtask check`.

- [ ] **Step 5: Commit** — `git commit -m "feat(storage): AudienceStore for named audiences and membership"`

---

## Phase 7 — PostStore: persistence + resolution filter

### Task 11: `local_channel_id()` accessor on `SubscriptionStorage`

**Files:**
- Modify: `storage/src/subscriptions.rs` (trait), `storage/src/sqlite/subscriptions.rs`, `storage/src/postgres/subscriptions.rs`

**Interfaces:**
- Consumes: the `channels` lookup (Task 2) and the `SubscriptionStorage` trait (Task 9).
- Produces: `async fn local_channel_id(&self) -> sqlx::Result<i64>` on `SubscriptionStorage` (both impls run `SELECT channel_id FROM channels WHERE name = 'local'`). This is the **production** accessor the web `viewer_identity()` extractor (Task 15) and `subscribe_to` (Task 17) use to construct `ViewerIdentity::local`. (The storage tests in Tasks 7/9/10 use a raw test-helper of the same name and do **not** depend on this method — so introducing it here, after the stores exist, creates no forward dependency.) The web layer memoizes the result once per process (see Task 15) rather than querying per request; storage just exposes the lookup.

- [ ] **Step 1:** add `local_channel_id()` to the trait + both impls; add a test (`#[apply(backends)]` in `server/tests/storage.rs`) asserting it returns the same id as the seeded `'local'` row on both backends.
- [ ] **Step 2:** Run `cargo test -p jaunder --test storage local_channel_id`; commit `feat(storage): expose local channel id lookup`.

### Task 12: Persist `post_audiences` on create/edit

**Files:**
- Modify: `storage/src/posts.rs` (`CreatePostInput`, `UpdatePostInput`, trait), `storage/src/sqlite/posts.rs`, `storage/src/postgres/posts.rs`

**Interfaces:**
- Consumes: `AudienceTarget` (Task 8), `target_kinds` ids (Task 2).
- Produces: `CreatePostInput`/`UpdatePostInput` gain `pub audiences: Vec<AudienceTarget>`. On create/update, after writing the post, the store replaces the post's `post_audiences` rows to match: `Public`→one `public` row; `Subscribers`→one `subscribers` row; `Named(id)`→a `named` row with `audience_id=id`; `Private` or empty vec → **no rows**. Update does a delete-all-then-insert within the existing post-update transaction.

- [ ] **Step 1: Failing test** — create a post targeting `[Public, Named(aud)]`, assert two `post_audiences` rows with the right kinds/audience_id; update it to `[Private]` (empty), assert zero rows; update to `[Subscribers]`, assert one `subscribers` row. Both backends.

- [ ] **Step 2: Implement** — add a private `replace_post_audiences(tx, post_id, &[AudienceTarget])` helper used by both create and update. Map `AudienceTarget` → `(kind_id, audience_id?)` via a `SELECT kind_id FROM target_kinds WHERE name=?`. Run, verify PASS both backends.

- [ ] **Step 3: Commit** — `git commit -m "feat(storage): persist per-post audience targeting"`

### Task 13: Resolution filter on every read path

**Files:**
- Modify: `storage/src/posts.rs` trait signatures, `storage/src/sqlite/posts.rs`, `storage/src/postgres/posts.rs`

**Interfaces:**
- Consumes: `ViewerIdentity` (Task 8), `post_audiences`/`subscriptions`/`audience_members`.
- Produces: a `viewer: &ViewerIdentity` parameter added to the **read** methods that surface published content to arbitrary viewers: `get_post_by_id`, `get_post_by_permalink`, `list_published`, `list_published_by_user`, `list_posts_by_tag`, `list_user_posts_by_tag`, and `list_published_in_window` (feeds). Owner-scoped methods that already filter by `user_id` and only ever serve the author their own posts (`list_drafts_by_user`, `list_collection_by_user`) do **not** take a viewer — but see Task 21 for the AtomPub owner-access note. The resolution predicate, expressed once as a reusable SQL fragment:

```sql
-- :is_author = 1 when viewer is Channel{local, <this row's user_id>}, else 0
-- :viewer_channel / :viewer_ref bound only for the Channel case
( p.user_id = :viewer_author_id   -- author-sees-own (only fires for local viewer)
  OR EXISTS (
    SELECT 1 FROM post_audiences pa
    JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id
    WHERE pa.post_id = p.post_id AND (
         tk.name = 'public'
      OR (tk.name = 'subscribers' AND EXISTS (
            SELECT 1 FROM subscriptions s JOIN subscription_statuses st ON st.status_id=s.status_id
            WHERE s.author_user_id = p.user_id AND s.channel_id = :viewer_channel
              AND s.subscriber_ref = :viewer_ref AND st.name='active'))
      OR (tk.name = 'named' AND EXISTS (
            SELECT 1 FROM audience_members am
            JOIN subscriptions s ON s.subscription_id = am.subscription_id
            JOIN subscription_statuses st ON st.status_id = s.status_id
            WHERE am.audience_id = pa.audience_id AND s.channel_id = :viewer_channel
              AND s.subscriber_ref = :viewer_ref AND st.name='active'))
  ))
)
```
For `Anonymous`, the whole predicate collapses to `EXISTS(... tk.name='public')` and the author branch is dead (bind a sentinel `viewer_author_id = -1`, `viewer_channel = -1`, `viewer_ref = ''`). Implement as a Rust helper `resolution_where(viewer) -> (sql_fragment, binds)` per backend, folded into each query's `WHERE`. The fragment is identical across backends except placeholder syntax (`?` vs `$n`).

- [ ] **Step 1: Write the resolution matrix tests** in `server/tests/storage.rs` (`#[apply(backends)]`, exercising `state.posts` with each `ViewerIdentity`) — the spec's matrix. Seed: author A; viewers = anonymous, A (author), active local subscriber S, named-member M (in audience G), non-member N. Posts: `Public`, `Private`, `Subscribers`, `Named(G)`, `Named(G2)`, `Public+Named(G)`. Assert each viewer's `get_post_by_id` and presence in `list_published` matches the truth table:

| post \ viewer | anon | author A | subscriber S | member M (G) | non-member N |
|---|---|---|---|---|---|
| Public | ✓ | ✓ | ✓ | ✓ | ✓ |
| Private | ✗ | ✓ | ✗ | ✗ | ✗ |
| Subscribers | ✗ | ✓ | ✓ | ✓ (S also subscribed) | ✗ |
| Named(G) | ✗ | ✓ | ✗ | ✓ | ✗ |
| Named(G2) | ✗ | ✓ | ✗ | ✗ | ✗ |
| Public+Named(G) | ✓ | ✓ | ✓ | ✓ | ✓ |

(Set up S, M as active subscribers; M additionally in G. "Subscribers" row: M sees it only if M has an active subscription — make M a subscriber too, so ✓.)

- [ ] **Step 2: Run, verify FAIL** (methods don't take a viewer yet).

- [ ] **Step 3: Implement** — change the seven read signatures listed above to add `viewer: &ViewerIdentity`, build the fragment, fold into each `WHERE`. Keep the existing cursor/limit/order logic. Run, verify PASS both backends.

- [ ] **Step 4:** Run the full storage integration suite (`cargo test -p jaunder --test storage`) to catch call-site breakage. Commit `feat(storage): viewer-aware resolution filter on post reads`.

---

## Phase 8 — Configuration

### Task 14: `posts.default_audience`

**Files:**
- Modify: `storage/src/site_config.rs` (add `POSTS_DEFAULT_AUDIENCE_KEY`, `get_default_audience`/`set_default_audience`)

**Interfaces:**
- Consumes: `AudienceTarget`/`TargetKind` (Task 8).
- Produces: `async fn get_default_audience(&self) -> sqlx::Result<AudienceTarget>` returning `Public` when unset/invalid; only the built-ins `public`/`subscribers`/`private` are valid defaults (a named default is out of scope — named audiences are per-author, site config is instance-wide). Setter stores the string.

- [ ] **Step 1: Test** — unset → `Public`; set `"private"` → `Private`; set garbage → `Public`. (Default getters are in `SiteConfigStorage` with bodies; test via either backend's store.)
- [ ] **Step 2: Implement** following the `get_feeds_min_items` pattern (key constant + parse + default). Run, commit `feat(storage): posts.default_audience site config`.

---

## Phase 9 — Web: viewer extractor + threading

### Task 15: `viewer_identity()` extractor

**Files:**
- Create: `web/src/viewer.rs`
- Modify: `web/src/lib.rs` (module)

**Interfaces:**
- Consumes: `AuthUser` (existing), `SubscriptionStorage::local_channel_id` (Task 11), `ViewerIdentity` (Task 8).
- Produces: `async fn viewer_identity() -> ViewerIdentity` for use inside `#[server]` fns: resolves the account session via `leptos_axum::extract::<AuthUser>()`; on success returns `ViewerIdentity::local(auth.user_id, local_channel_id)`, else `Anonymous`. The `local_channel_id` is read once and cached in a Leptos context / `OnceCell` (avoid a query per request). **Layer A constructs only `Anonymous` and `Channel{local,…}`** — the precedence ladder (account → viewer session → anonymous) is Layer C; leave a doc-comment marking the insertion point.

- [ ] **Step 1: Test** — a unit/integration test asserting an authed request yields `Channel{local, user_id}` and an unauthed one yields `Anonymous` (mirror an existing `web` server-fn test).
- [ ] **Step 2: Implement.** Run, commit `feat(web): viewer_identity extractor (local channel)`.

### Task 16: Thread `ViewerIdentity` through read server-fns

**Files:**
- Modify: `web/src/posts/listing.rs` (`list_local_timeline`, `list_user_posts`, `list_home_feed`, `list_posts_by_tag`, `list_user_posts_by_tag`), `web/src/pages/posts.rs` (single-post fetch), `web/src/posts/server.rs` as needed

**Interfaces:**
- Consumes: `viewer_identity()` (Task 15), the new viewer-param `PostStorage` methods (Task 13).
- Produces: every public read server-fn extracts `viewer_identity()` and passes it to the store. The current `viewer_user_id: Option<i64>` (used by `timeline_post_summary` for *display* of owner controls) is **kept** for display but is now derived from the same `ViewerIdentity` (author-only when `Channel{local,…}`); filtering moves into the store query, so post-fetch `filter_map` no longer needs to drop unauthorized rows.

- [ ] **Step 1: Test** — extend/adjust existing listing tests: anonymous timeline shows only Public; an authed non-subscriber sees Public + own; a subscriber additionally sees Subscribers/named posts they're admitted to. (These are server-fn level tests; the exhaustive matrix is Task 13.)
- [ ] **Step 2: Implement** the threading across each server-fn. Run, verify PASS.
- [ ] **Step 3:** `cargo xtask check`; commit `feat(web): enforce visibility on timelines and post pages`.

---

## Phase 10 — Web UI: subscribe, audiences, picker

### Task 17: Subscribe / Unsubscribe (profile)

**Files:**
- Create: `web/src/subscriptions/mod.rs` (server fns `subscribe_to`, `unsubscribe_from`)
- Modify: `web/src/pages/profile.rs` (button + state)

**Interfaces:**
- Consumes: `SubscriptionStorage` (Task 9), `viewer_identity()`/`AuthUser`.
- Produces: `#[server] async fn subscribe_to(author_username) -> WebResult<()>` — requires an authed local user (Layer A), resolves author's `user_id`, calls `subscribe(author, local_channel_id, viewer_user_id.to_string())` (routes the admission seam → `active`). `unsubscribe_from` mirrors. Profile page shows Subscribe when not subscribed, Unsubscribe when subscribed (query via `is_subscriber`). Self-subscribe is rejected/hidden.

- [ ] **Step 1: Test** the two server fns (authed subscribe makes `is_subscriber` true; unsubscribe reverses; self-subscribe rejected). Run FAIL → implement → PASS.
- [ ] **Step 2:** Profile UI wiring. `cargo xtask check`; commit `feat(web): subscribe/unsubscribe on profiles (local channel)`.

### Task 18: Named-audience management (account area)

**Files:**
- Create: `web/src/audiences/mod.rs` (server fns: `create_audience`, `rename_audience`, `delete_audience`, `list_my_audiences`, `add_subscriber_to_audience`, `remove_subscriber_from_audience`, `list_my_subscribers`)
- Modify: account-area page (under `web/src/pages/`) to add an "Audiences" management screen.

**Interfaces:**
- Consumes: `AudienceStorage` (Task 10), `SubscriptionStorage::list_subscribers` (Task 9), `AuthUser` (author scope).
- Produces: author-scoped CRUD over named audiences and assignment of one's active subscribers into them. Every server fn derives `author_user_id` from `AuthUser` (never from a client param) and passes it to the store so composite FKs enforce ownership.

- [ ] **Step 1: Test** each server fn (auth required; create/rename/delete; add/remove member; duplicate-name surfaced as a user-facing error). FAIL → implement → PASS.
- [ ] **Step 2:** UI screen (list audiences, create/rename/delete, multiselect subscribers per audience). `cargo xtask check`; commit `feat(web): named-audience management`.

### Task 19: Post-editor audience picker

**Files:**
- Modify: the post editor component/page (`web/src/pages/posts.rs` / editor module), `web/src/posts/server.rs` (create/edit server fns)

**Interfaces:**
- Consumes: `AudienceStorage::list_audiences` (for the multiselect), `SiteConfigStorage::get_default_audience` (initial selection), `CreatePostInput`/`UpdatePostInput.audiences` (Task 12).
- Produces: a picker offering **Public / Private / Subscribers** (mutually exclusive built-ins) **and/or** a multiselect of the author's named audiences (union semantics: e.g. Public+Friends is allowed and means "everyone"). The create/edit server fns translate the picker selection into `Vec<AudienceTarget>` and pass it through. Initial selection on a new post = `get_default_audience()`. Editing an existing post pre-selects its current targeting (read its `post_audiences`).

- [ ] **Step 1:** add `async fn get_post_audiences(post_id) -> Vec<AudienceTarget>` to `PostStorage` (owner-only fetch for pre-selecting the editor) + test. FAIL → implement → PASS.
- [ ] **Step 2: Test** the create/edit server fns translate picker → `AudienceTarget` vec → persisted rows (round-trip via Task 12). FAIL → implement → PASS.
- [ ] **Step 3:** Picker UI. `cargo xtask check`; commit `feat(web): post-editor audience picker`.

---

## Phase 11 — Feeds + AtomPub

### Task 20: Public-only published feeds (M8)

**Files:**
- Modify: `server/src/feed/regenerate.rs` (and/or the feed worker call into `list_published_in_window`)

**Interfaces:**
- Consumes: `list_published_in_window(viewer = Anonymous, …)` (Task 13 added the viewer param).
- Produces: feed generation passes `ViewerIdentity::Anonymous`, so published feeds contain only Public posts. Where M8 already calls `list_published_in_window`, this is just supplying the new arg; the resolution fragment for `Anonymous` reduces to the `public` EXISTS.

- [ ] **Step 1: Test** — a feed-generation test with a mix of Public/Subscribers/Private posts emits only the Public ones. FAIL → implement → PASS.
- [ ] **Step 2:** Commit `feat(server): published feeds contain only Public posts`.

### Task 21: AtomPub default audience + owner access

**Files:**
- Modify: `server/src/atompub/posts.rs` (create/edit mapping), `server/src/atompub/mapping.rs`

**Interfaces:**
- Consumes: `SiteConfigStorage::get_default_audience` (Task 14), `CreatePostInput.audiences` (Task 12), the author's own AtomPub collection read path.
- Produces: posts created/edited via AtomPub (which has no native visibility field) take `posts.default_audience`. The author always sees their own posts in their AtomPub collection regardless of audience — `list_collection_by_user`/`list_drafts_by_user` remain owner-scoped (no viewer filter), so this holds without change. Setting a non-default audience stays a web-UI action (an Atom extension element is future work).

- [ ] **Step 1: Test** — an AtomPub `POST` creates a post whose persisted targeting equals the configured default; the author's AtomPub collection still lists their non-Public posts. FAIL → implement → PASS.
- [ ] **Step 2:** Commit `feat(server): AtomPub posts adopt posts.default_audience`.

---

## Phase 12 — End-to-end + final gate

### Task 22: Playwright E2E (Nix VM)

**Files:**
- Create: `end2end/tests/visibility.spec.ts` (match repo's existing spec layout/helpers)

**Interfaces:**
- Consumes: the whole stack.
- Produces: an E2E proving the chain. Scenarios:
  1. Author marks post **Private** → logged-out visitor and a non-subscriber logged-in user do **not** see it; author does.
  2. Author marks post **Subscribers** → a second local user Subscribes via the profile → now sees it; after Unsubscribe → no longer sees it.
  3. Author creates a **named audience** "Friends", marks a post to Friends, adds user X but not Y → X sees it, Y does not.
  4. **Public** post is visible to anonymous; appears in the published feed; a Subscribers post does **not** appear in the feed.

- [ ] **Step 1:** Write the spec following the existing E2E harness (seed users, login helpers, mail/feed capture as used elsewhere).
- [ ] **Step 2: Run** the e2e via `cargo xtask validate` (sqlite + postgres). Iterate until green.
- [ ] **Step 3:** Commit `test(e2e): content visibility end-to-end`.

### Task 23: Full CI-faithful gate + docs status flip

**Files:**
- Modify: `docs/superpowers/specs/2026-06-18-content-visibility-design.md` (Status: draft → implemented, link this plan), backfill the `Beads:` line with the real issue ids.

- [ ] **Step 1: Run the full gate** — `ctx_execute(shell, "cargo xtask validate")` (bare; pass/fail = exit code). Read detail from `.xtask/last-result.json` if needed. Expected: green.
- [ ] **Step 2:** Confirm coverage policy holds (CONTRIBUTING's coverage discipline; watch the dialect-file coverage gotcha — no `#[cfg(test)]` in `sqlite/*.rs`/`postgres/*.rs`).
- [ ] **Step 3:** Flip the Layer A spec status, commit `docs: mark content-visibility Layer A implemented`.

---

## Layer B / Layer C sequencing notes (not built here)

This plan deliberately builds the seams Layers B and C need so they are **additive**:

- **`ViewerIdentity` is already the read currency.** Layer C adds a viewer-session producer and Layer B adds an HTTP-Signature/Authorized-Fetch producer; both yield `Channel{channel_id, subscriber_ref}` and feed the *same* `resolution_where` fragment (Task 13). No `PostStorage` signature changes when they land.
- **The admission seam is the only subscription gate** (Task 8/9). M13 swaps `OpenSubscriptionPolicy` for the per-author open/invite-only policy in one place; `subscription_statuses` already reserves `pending`/`blocked` and resolution admits only `active`, so the gate is latent and fails closed today.
- **Channel-parameterized subscriptions.** `subscribe(author, channel_id, subscriber_ref)` is already channel-generic; Layer C's Mastodon/email channels call it verbatim after proving identity. **Constraint for Layer B/C:** the canonical `subscriber_ref` for `activitypub` must be the **AP actor URI**, identical between Layer C's "Sign in with Mastodon" and Layer B's inbound `Follow`, so browse-time and federated subscriptions converge on one row (Layer C spec §"Layer B forward-compatibility").
- **Channel capability split.** `local` has neither authenticate nor deliver. Layer C adds `channels` seed rows `activitypub`/`email` (+ enum variants — the bijection test from Task 3 will then require those variants), authenticate capability, the `ChannelAuthenticator` registry, viewer sessions, and the `signin/:channel` endpoints. Layer B adds deliver (AP `to`/`cc` mapping with the point-in-time push caveat; email newsletter).
- **No Layer B spec exists yet** — write one (M12 + email delivery) before planning its implementation; this plan does not cover it.

## Beads registration

Registered under epic **`jaunder-i3il`** (label `visibility,layer-a`) as a linear `blocks`-chain of 13 phase issues, each parented to the epic with a `design` field linking this plan and its task numbers:

| Issue | Covers | Tasks |
|---|---|---|
| `jaunder-i3il.1` | enforce SQLite `foreign_keys` (app + test pools) | 1 |
| `jaunder-i3il.2` | lookup tables + enums + bijection tests | 2-3 |
| `jaunder-i3il.3` | core tables + composite-FK enforcement | 4-7 |
| `jaunder-i3il.4` | `ViewerIdentity` / `AudienceTarget` / `SubscriptionPolicy` | 8 |
| `jaunder-i3il.5` | `SubscriptionStore` + admission seam | 9 |
| `jaunder-i3il.6` | `AudienceStore` | 10 |
| `jaunder-i3il.7` | `PostStore` — `local_channel_id`, persist targeting, resolution | 11-13 |
| `jaunder-i3il.8` | `posts.default_audience` config | 14 |
| `jaunder-i3il.9` | viewer extractor + thread `ViewerIdentity` | 15-16 |
| `jaunder-i3il.10` | subscribe/unsubscribe, audience mgmt, editor picker | 17-19 |
| `jaunder-i3il.11` | Public-only feeds + AtomPub default | 20-21 |
| `jaunder-i3il.12` | Playwright E2E | 22 |
| `jaunder-i3il.13` | full CI-faithful gate + flip spec status | 23 |

Only `jaunder-i3il.1` is `bd ready`; the rest unblock as predecessors close. (Task 23 / `.13` backfills the Layer A spec's `Beads:` line with these ids.)

## Self-review notes

- **Spec coverage:** data model (T2,4,5,6) · no-app-invariants/composite FK (T1,5,7) · resolution rule incl. ViewerIdentity & anonymous reduction (T8,13) · admission seam fail-closed (T8,9) · SubscriptionStore/AudienceStore/extended PostStore, both backends (T9,10,12,13) · bijection tests (T3) · default_audience (T14) · web surfaces timelines/profile/account/editor (T16,17,18,19) · feeds Public-only (T20) · AtomPub default + owner access (T21) · E2E (T22). All spec sections map to a task.
- **Open items resolved:** SQLite `foreign_keys` confirmed off and fixed in T1; `post_audiences` nullable-PK divergence between SQLite (NULL-in-PK ok) and Postgres (partial unique indexes) handled in T6.
