# Plan — Issue #715: id decodes + the `sqlx-newtype-decode` gate

**Spec:**
[`../specs/2026-07-30-issue-715-query-scalar-id-decodes.md`](../specs/2026-07-30-issue-715-query-scalar-id-decodes.md)
**Issue:** [#715](https://github.com/jaunder-org/jaunder/issues/715) **For
agentic workers:** drive with `jaunder-iterate`; delegate an individual task
with `jaunder-dispatch` where useful. Tick checkboxes in real time.

## Goal

Every id-yielding sqlx decode under `storage/src` produces its id newtype
directly, and a new syn-AST gate keeps it that way by **enumerating** the decode
population rather than searching for violation spellings.

## Scope

**In** — the 11 `storage/src` decode sites (spec §"Violations"); the two
`server/tests/storage/mod.rs` twins; `audience_target_row`'s encode twin; the
generic bounds that move with them; the `sqlx-newtype-decode` xtask gate with
its 9 seed allowlist entries; the "enumerate, don't search" ADR draft; the #716
scope update.

**Out** — the bind direction (#716); the string decode family and the
out-of-population record (#728, already filed, blocked by this); `SiteConfigKey`
(#687); the `#[server]`/struct-field adoption gate (#697); `raw_ids`' `Vec<i64>`
seam (#716's class, spec D9).

**Separable concerns already filed:** #728. No task needed.

## Tasks

1. Sweep the five self-contained `query_scalar` id sites + their bounds
2. Sweep the `tag_id` pair in both dialect files
3. Sweep `feed_events::enqueue` + its `(i64,)` bound
4. Sweep the two feed-events row mappers + the `corrupt`/`purge_corrupt` seam
5. Sweep the audiences decode + its encode twin + the four bounds it moves
6. Sweep the two `server/tests` twins + the stale `email.rs` comment
7. Build, seed, and wire the `sqlx-newtype-decode` gate (one commit — see below)
8. Finalise the ADR draft, link it from the bind gate, update #716, write the PR
   notes

## Key risks and decisions

- **The issue's "transposition becomes a compile error" claim is false** (spec
  §Correction). No task attempts to demonstrate it; task 8 records the
  correction. An agent that "fixes" this by adding a compile-fail test is wrong.
- **Task 7 is one commit and cannot be split.** `xtask/src/lib.rs:22` declares
  `mod steps` **private**, so gate code that is not yet called from `run` is
  unreachable and fires `dead_code`; `static_checks.rs:119-131` runs clippy with
  `-D warnings`, so the pre-commit `cargo xtask check` would be red. This is the
  recorded xtask pub-API / dead-code commit boundary. Core, allowlist, and
  wiring land together.
- **The gate's position-precedence rule is load-bearing** (spec D4). Positions
  1–3 overlap on live code — `postgres/backup.rs:320-326` hits turbofish _and_
  fn-return — and without "one record per call, nearest declared type wins" the
  seed allowlist's counts never match and the gate fails on a clean tree.
- **Do not let the gate read SQL.** Any `contains("_id")` or
  `contains("COUNT(")` in the rule or the exemption path reintroduces the blind
  spot the ADR forbids. Reviewers should grep the gate for SQL-text inspection
  and reject it.
- **Multiplicity is not decoration.** Byte-identical decode pairs exist
  (`backup.rs:773`/`:778`, `test_support.rs:111`/`:112`). Entry `count` must
  fail on a gain, or the allowlist is region-scoped in disguise (spec D7).
- **Unused `where` bounds do not warn.** Sweeping a decode can strand an `i64`
  bound that neither rustc nor clippy will flag. Each task that touches an
  `impl` tries removing its now-suspect `i64`-family bounds and keeps the
  removal if the crate still builds.
- **Ordering:** sweeps (1–6) land before the gate (7), so each sweep commit
  passes a tree where the gate does not yet exist, and the gate lands green on a
  swept tree.

## Global constraints

- Complete Rust, no placeholders. Exact crate names: `storage`, `common`,
  `xtask`, server crate package is **`jaunder`**.
- Storage tests follow the dual-backend template (`CONTRIBUTING.md` "backend
  parity"); a bare `#[tokio::test]` that should be dual-backend fails
  `test-backend-pattern`.
- No test logic in ADR-0019 per-backend dialect files.
- Every commit: run `cargo xtask check` **first** and green (the pre-commit hook
  runs it anyway) — see `jaunder-commit`. Run it in the foreground with
  `timeout: 600000`.
- **No `Co-Authored-By` trailer.**
- `xtask` is excluded from the workspace:
  `cargo test --manifest-path xtask/Cargo.toml`.
- Work from the worktree at
  `/home/mdorman/src/jaunder/.claude/worktrees/issue-715-query-scalar-id-decodes`.

---

## Task 1 — Five self-contained `query_scalar` id sites

- [x] Done

**Files**

- `storage/src/users.rs` — `:267`, the rewrap at `:285`, and the **decode
  bound** at `:227`
- `storage/src/postgres/mod.rs` — `:120` and `Ok(id) => UserId::from(id)` at
  `:134`
- `storage/src/sqlite/mod.rs` — `:236` and `Ok(id) => UserId::from(id)` at
  `:250`
- `storage/src/posts.rs` — `:908` + `Ok(post_id.map(PostId::from))` at `:915`;
  `:1917` + `let post_id = PostId::from(post_id);` at `:1943`; bounds `:825` and
  `:1911`

**Change**

Each turbofish moves from `i64` to the id newtype and the rewrap is deleted:

```rust
// storage/src/users.rs:267
let result = sqlx::query_scalar::<_, UserId>(
    "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
     VALUES ($1, $2, $3, $4, $5)
     RETURNING user_id",
)
// … binds unchanged …
.await;

match result {
    Ok(id) => Ok(id),
    Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
        Err(CreateUserError::UsernameTaken)
    }
    Err(error) => Err(CreateUserError::Internal(error)),
}
```

```rust
// storage/src/posts.rs:908
let post_id = sqlx::query_scalar::<_, PostId>(
    "SELECT post_id FROM idempotency_keys WHERE user_id = $1 AND key = $2",
)
.bind(user_id)
.bind(key)
.fetch_optional(&self.pool)
.await?;
Ok(post_id)
```

**Bounds — two different shapes, don't assume the tuple one:**

`posts.rs:825` and `:1911` are `FromRow` tuple bounds and become `(PostId,)`.
But `users.rs` has **no** `FromRow` tuple bound; its `query_scalar` resolves
through a `Decode` bound at `:227`, which gains the newtype:

```rust
// storage/src/users.rs:227
    for<'r> UserId: sqlx::Decode<'r, DB> + sqlx::Type<DB>,
```

`posts.rs:1943`'s `let post_id = PostId::from(post_id);` is deleted outright —
the binding is already a `PostId`.

Then try removing each now-suspect `i64` bound in the touched impls
(`users.rs:227`'s old `i64: Decode` line, `posts.rs:832`) and keep the removal
if the crate still builds.

**Interfaces** — no public signature changes. `create_user` already returns
`Result<UserId, _>`; `post_id_for_idempotency_key` already returns
`Result<Option<PostId>, _>`.

**Test** — no new tests. These paths are covered by the existing dual-backend
storage suite; the change is type-level and the compiler is the check.

**Run**

```console
$ cargo check -p storage --all-features --all-targets
$ cargo xtask check
```

Expected: PASS.

**Commit** —
`types: decode user/post ids into their newtypes at query_scalar sites (#715)`

---

## Task 2 — The `tag_id` pair

- [x] Done

**Files**

- `storage/src/postgres/posts.rs` — `:147-151`
- `storage/src/sqlite/posts.rs` — `:163-167`

**Change** (both dialects, identically):

```rust
let tag_id = sqlx::query_scalar::<_, TagId>("SELECT tag_id FROM tags WHERE tag_slug = $1")
    .bind(&slug)
    .fetch_one(&mut *tx)   // `&mut *conn` in the sqlite twin
    .await?;
```

The `let tag_id: i64 =` ascription goes with it. The `post_tags` INSERT below is
unchanged — `.bind(post_id).bind(tag_id)` now binds two typed ids via the
ADR-0071 bridge.

Add `use common::ids::TagId;` if not already in scope.

**Interfaces** — none. `tag_id` is a local.

**Test** — none new; `tag_post` is covered by the existing dual-backend tagging
tests.

**Do NOT** add a test asserting that transposing the two binds fails to compile.
It does not — see spec §Correction. sqlx's `bind<T: Encode + Type>` is per-call
generic.

**Run**

```console
$ cargo check -p storage --all-features --all-targets
$ cargo nextest run -p jaunder --test integration tag
```

Expected: PASS.

**Commit** — `types: decode tag_id into TagId in both dialects (#715)`

---

## Task 3 — `feed_events::enqueue`

- [x] Done

**Files** — `storage/src/feed_events.rs` (`:170` bound, `:178-183`)

**Change**

```rust
    (FeedEventId,): for<'r> sqlx::FromRow<'r, DB::Row>,
```

```rust
async fn enqueue(&self, feed_path: &FeedPath) -> Result<FeedEventId, FeedEventError> {
    let id = sqlx::query_scalar::<_, FeedEventId>(
        "INSERT INTO feed_events (feed_url) VALUES ($1) RETURNING id",
    )
    .bind(feed_path)
    .fetch_one(&self.pool)
    .await?;
    Ok(id)
}
```

The `let id: i64` ascription and `FeedEventId::from(id)` both go.

**Interfaces** — none; `enqueue` already returns `FeedEventId`.

**Test** — none new.

**Run**

```console
$ cargo check -p storage --all-features --all-targets
```

Expected: PASS.

**Commit** —
`types: decode the feed-event id into FeedEventId at enqueue (#715)`

---

## Task 4 — The two feed-events row mappers and the `corrupt` seam

- [x] Done

**Files**

- `storage/src/postgres/feed_events.rs` — `:21-36` (`purge_corrupt`), `:77`,
  `:83`, `:87`
- `storage/src/sqlite/feed_events.rs` — its `purge_corrupt` (`:25-42`), `:77`,
  `:83`, `:88`

**Change** (both dialects):

```rust
let id: FeedEventId = r.get("id");
let Ok(feed_path) = r.try_get::<FeedPath, _>("feed_url") else {
    corrupt.push(id);
    continue;
};
records.push(FeedEventRecord {
    id,
    feed_path,
    // … rest unchanged …
});
```

`corrupt` infers `Vec<FeedEventId>`; `purge_corrupt`'s parameter becomes
`&[FeedEventId]`.

**Postgres needs `raw_ids` internally — this is not optional.** `purge_corrupt`
binds its slice to `DELETE FROM feed_events WHERE id = ANY($1)` (`:26-27`), and
the ADR-0071 bridge (`macros/src/sqlx_bridge.rs:28-70`) emits
`Type`/`Encode`/`Decode` but **no `PgHasArrayType`**, so `.bind(&[FeedEventId])`
has no array encoding and will not compile:

```rust
async fn purge_corrupt(pool: &Pool<Postgres>, ids: &[FeedEventId]) {
    if ids.is_empty() {
        return;
    }
    tracing::warn!("feed_events: purging rows with an unparseable feed_url");
    if let Err(e) = sqlx::query("DELETE FROM feed_events WHERE id = ANY($1)")
        .bind(raw_ids(ids))
        .execute(pool)
        .await
    { /* … unchanged … */ }
}
```

That is the same shape `mark_regenerated`/`mark_pinged`/`mark_failed` already
use (`:108`, `:119`, `:134`). The SQLite twin binds per-id and needs no such
routing.

**Boundary (spec D9):** `postgres/feed_events.rs:40`'s
`fn raw_ids(ids: &[FeedEventId]) -> Vec<i64>` **stays as-is**. It is a bind-side
strip laundered through a helper — #716's class, not this issue's. Adding a
fourth caller does not widen the scope; removing the helper would.

**Interfaces** — `purge_corrupt` is private to each dialect module.

**Test** — the corrupt-row purge path is covered by the existing feed-worker
tests (`server/tests/feed/feed_worker.rs`). No new test; verify the existing
ones still pass.

**Run**

```console
$ cargo check -p storage --all-features --all-targets
$ cargo nextest run -p jaunder --test integration feed
```

Expected: PASS.

**Commit** —
`types: decode feed-event row ids into FeedEventId in both mappers (#715)`

---

## Task 5 — Audiences: the decode, its encode twin, and four bounds

- [ ] Done

**Files** — `storage/src/posts.rs`: bounds `:829`, `:851`, `:1893`, `:1979`; the
query at `:949-966`; the mapper pair at `:1838-1846` and `:1856-1863`;
`replace_post_audiences`'s bind at `:1989-1992`; the in-file tests at
`:2288-2305`

**Change**

```rust
// posts.rs:829
    (String, Option<AudienceId>): for<'r> sqlx::FromRow<'r, DB::Row>,
```

```rust
let rows: Vec<(String, Option<AudienceId>)> = sqlx::query_as(
    "SELECT tk.name, pa.audience_id \
     FROM post_audiences pa \
     JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id \
     WHERE pa.post_id = $1 \
     ORDER BY tk.name, pa.audience_id",
)
.bind(post_id)
.fetch_all(&self.pool)
.await?;
```

Both mapper halves lose their conversion:

```rust
fn audience_target_row(target: &AudienceTarget) -> Option<(&'static str, Option<AudienceId>)> {
    use common::visibility::TargetKind;
    match target {
        AudienceTarget::Public => Some((TargetKind::Public.into(), None)),
        AudienceTarget::Subscribers => Some((TargetKind::Subscribers.into(), None)),
        AudienceTarget::Named(id) => Some((TargetKind::Named.into(), Some(*id))),
        AudienceTarget::Private => None,
    }
}

fn audience_target_from_row(kind: &str, audience_id: Option<AudienceId>) -> Option<AudienceTarget> {
    match TargetKind::try_from(kind) {
        Ok(TargetKind::Public) => Some(AudienceTarget::Public),
        Ok(TargetKind::Subscribers) => Some(AudienceTarget::Subscribers),
        Ok(TargetKind::Named) => audience_id.map(AudienceTarget::Named),
        Err(_) => None,
    }
}
```

**Three `Encode` bounds move with the encode twin** — `replace_post_audiences`
now binds `Option<AudienceId>`, so at `posts.rs:851`, `:1893`, and `:1979`:

```rust
    for<'q> Option<AudienceId>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
```

Missing these is the failure mode the plan review caught: an unused `where`
bound does not warn, so the old `Option<i64>` line survives silently while the
build fails on the missing one. Confirm the `sqlx-newtype-bind` gate stays green
after the bind changes.

**Interfaces** — both mapper fns are private to `posts.rs`.

**Test** — update the existing in-file unit tests at `:2288-2305` to pass
`Some(AudienceId::from(7))` instead of `Some(7)`; assertions otherwise
unchanged. These are `#[test]`, not dual-backend.

**Run**

```console
$ cargo nextest run -p storage audience_target
$ cargo check -p storage --all-features --all-targets
```

Expected: PASS.

**Commit** —
`types: decode audience_id into AudienceId and type its encode twin (#715)`

---

## Task 6 — The `server/tests` twins and the stale comment

- [ ] Done

**Files**

- `server/tests/storage/mod.rs` — `local_channel_id` (`:240-253`),
  `post_audience_rows` (`:2446-2471`) and its caller, the `common::ids` import
  at `:2`
- `storage/src/email.rs` — the comment at `:106`

**Change**

```rust
async fn local_channel_id(backend: Backend, env: &TestEnv) -> ChannelId {
    let sql = "SELECT channel_id FROM channels WHERE name = 'local'";
    match backend {
        Backend::Sqlite => sqlx::query_scalar::<_, ChannelId>(sql)
            .fetch_one(&open_pool(&env.base).await)
            .await
            .unwrap(),
        Backend::Postgres => {
            let pool = env.base.pool().postgres();
            sqlx::query_scalar::<_, ChannelId>(sql)
                .fetch_one(pool)
                .await
                .unwrap()
        }
    }
}
```

```rust
async fn post_audience_rows(
    backend: Backend,
    env: &TestEnv,
    post_id: PostId,
) -> Vec<(String, Option<AudienceId>)> {
    // … sql unchanged …
    match backend {
        Backend::Sqlite => sqlx::query_as(&sql.replace("$1", "?"))
            .bind(post_id)
            .fetch_all(&open_pool(&env.base).await)
            .await
            .unwrap(),
        Backend::Postgres => {
            let pool = env.base.pool().postgres();
            sqlx::query_as(sql).bind(post_id).fetch_all(pool).await.unwrap()
        }
    }
}
```

Update the caller's expected values to `Option<AudienceId>`; add `AudienceId` to
the `common::ids` import at `:2`.

Fix the stale comment at `email.rs:106`: the bound at `:101` is
`(UserId, Email)`, not `(i64, Email)`.

**Interfaces** — test-local helpers only.

**Test** — the helpers _are_ test code; the audience-targeting persistence test
that calls `post_audience_rows` is the check.

**Run**

```console
$ cargo nextest run -p jaunder --test integration audience
$ cargo nextest run -p jaunder --test integration channel
```

Expected: PASS.

**Commit** —
`types: decode ids in the storage test harness; fix a stale bound comment (#715)`

---

## Task 7 — The `sqlx-newtype-decode` gate (core + allowlist + wiring, one commit)

- [ ] Done

**This task is deliberately one commit and must not be split.**
`xtask/src/lib.rs:22` declares `mod steps` private, so any gate item not
reachable from a called `run()` is `dead_code`, and `static_checks.rs:119-131`
runs clippy with `-D warnings` — an unwired-gate commit would fail its own
pre-commit `cargo xtask check`. Write it in the order below; commit once at the
end.

**Files** — new `xtask/src/steps/sqlx_newtype_decode_check.rs`;
`xtask/src/lib.rs` (declare in `mod steps { … }` alphabetically after
`sqlx_newtype_bind_check`, and call `run` from **both** arms — `Command::Check`
at `:346` and `Command::Validate` at `:383`, alongside
`steps::sqlx_newtype_bind_check::run(&mut result);`)

### 7a — The AST reader

Follow `rendered_html_from_trusted_check.rs`'s shape: `syn::parse_file`, a
`Scanner` implementing `syn::visit::Visit` and tracking the enclosing-fn
context, `syn::visit::visit_file`.

```rust
/// One decode site the scan found: where it is, and what it decodes into.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Decode {
    /// Enclosing function name, e.g. `get_post_audiences`. Empty at item level.
    pub function: String,
    /// The decode target, rendered from the AST (`i64`, `Vec<(String, Option<i64>)>`).
    pub target: String,
    /// The SQL literal or column name when the AST carries one, else the call shape.
    /// Recorded for the allowlist key and the error message **only** — nothing branches
    /// on it, because branching on SQL text is the blind spot this gate exists to close.
    pub what: String,
    pub line: usize,
}

/// Whether `ty` resolves to the `i64` family — recursing through `Vec`, `Option`,
/// `Result`, and tuples, so `Vec<(String, Option<i64>)>` is in population and
/// `Vec<(String, DateTime<Utc>)>` is not. Pure, unit-tested directly.
fn is_i64_family(ty: &syn::Type) -> bool
```

**The four positions (spec D4) and the precedence rule.** Collect **one record
per decode call expression** — a call to `query_scalar`, `query_as`, `get`, or
`try_get` — and take its target from the nearest declared type:

1. turbofish on the call itself; else
2. the enclosing `let`'s `Pat::Type` ascription; else
3. the enclosing `fn` return type, when the call is in tail position (through
   `match` arms and `?`).

A `let` or `fn` covering several calls yields one record **each** —
`backup.rs:733-745` is one `let live_count: i64 = match { … }` over two calls
and must produce two records, which is what allowlist entry A's `count: 2`
depends on. Without this precedence, `postgres/backup.rs:320-326` (a
`-> Result<i64, _>` fn whose body is `query_scalar::<_, Option<i64>>(…)?`)
records twice and the gate fails on a clean tree.

Position 4 is a **separate** population, recorded per declared field, not per
call: `#[derive(FromRow)]` struct fields and tuple `type` aliases under
`storage/src`. `syn` cannot tell a `query_as` target alias from any other tuple
alias, so the rule polices **every** tuple `type` alias in the root — today that
is only `feed_cache.rs:40`'s `CacheTuple`, so the cost is zero. Say so in the
module doc rather than pretending the rule is narrower than it is.

**Out of population, and documented in the module doc** (spec D5): a
`.get`/`try_get` with neither turbofish nor ascription. `syn` cannot tell
`sqlx::Row::get` from `serde_json::Map::get`, and both live in the root
(`postgres/backup.rs:177`, `sqlite/backup.rs:160`). Keying on the receiver name
would be a pattern search **and** would miss the real sites, whose receiver is
`r`.

### 7b — The allowlist

```rust
/// A decode exempt from the guard. Keyed by (file, function, target, what) — all
/// reflow-stable, none positional — plus the number of identical sites that key covers.
///
/// The count is load-bearing, not decoration. `sqlx-newtype-bind`'s substring needles
/// exempt "every matching line under the policed root" (its own doc, `:60-62`), which is
/// a region-scoped exemption; the live population here contains byte-identical decode
/// pairs (`backup.rs:773`/`:778`, `test_support.rs:111`/`:112`) that no needle can
/// separate. Stating the multiplicity means gaining a third identical decode is a
/// mismatch and a failure, not a silent absorption.
struct Allowed {
    file: &'static str,
    function: &'static str,
    target: &'static str,
    what: &'static str,
    count: usize,
    /// Why this decode legitimately yields a primitive.
    reason: &'static str,
}

/// Every in-population decode not matched by an [`ALLOWLIST`] entry, plus every entry
/// whose observed count differs from its declared one. Pure given the `(path, source)`
/// pairs, so it is unit-tested directly.
pub fn problems(scanned: &[(String, String)]) -> Option<String>
```

Report the three failure directions distinctly: an unmatched decode ("type it or
justify it"), a count mismatch ("this entry declared N, the tree has M"), and a
stale entry (declared but no matching site) — a stale entry is a failure too, or
the allowlist stops tracking the tree.

**Seed exactly these 9 entries.** Function names verified against the tree:

| File                    | Fn                                                      | Target        | Count | Reason                                                                         |
| ----------------------- | ------------------------------------------------------- | ------------- | ----- | ------------------------------------------------------------------------------ |
| `backup.rs`             | `backup_covers_every_table_or_deliberately_excludes_it` | `i64`         | 2     | `COUNT(*)` of live tables, two dialect arms under one `let`                    |
| `backup.rs`             | `database_is_empty_ignores_only_seeded_lookups`         | `i64`         | 2     | `COUNT(*)` per seeded table; byte-identical, SQL built in `format!`            |
| `sqlite/mod.rs`         | `database_is_empty`                                     | `i64`         | 1     | `SELECT EXISTS(…)` — SQLite has no bool; SQL built in `format!`                |
| `postgres/schema.rs`    | `every_foreign_key_is_deferrable`                       | `i64`         | 1     | `COUNT(*)` of non-deferrable FK constraints                                    |
| `postgres/backup.rs`    | `schema_version`                                        | `Option<i64>` | 1     | `MAX(version)` migration version                                               |
| `sqlite/backup.rs`      | `schema_version`                                        | `Option<i64>` | 1     | `MAX(version)` migration version                                               |
| `test_support.rs`       | `scalar_i64`                                            | `i64`         | 2     | Generic test scalar helper; SQL is a runtime `&str`, type from the `fn` return |
| `subscriptions.rs`      | `is_subscriber`                                         | `(i64,)`      | 1     | Existence flag, not an id (`subscriptions.rs:149` already says so)             |
| `sqlite/feed_events.rs` | `claim_pending_batch`                                   | `i64`         | 1     | `attempts` retry counter                                                       |

Re-confirm each against the tree before committing; the _set_ is fixed by the
spec, the fn names are the key and must match.

### 7c — Wiring

`pub fn run(result: &mut CommandResult)` mirroring
`sqlx_newtype_bind_check::run`: scan `POLICED_ROOT` (`storage/src`) via
`files::with_extension`, and make a missing/renamed root a **hard failure**
(AC14), never a silent skip.

Module doc must record (AC15): the two unreadable classes from D5; the
tuple-alias over-reach above; and why the root is `storage/src` only — the two
`server/tests/storage/mod.rs` sites are fixed by this issue but unpoliced,
because a regression there surfaces as a failing test rather than a production
transposition (spec D9).

**Test** — in-file `#[cfg(test)]` over synthetic sources, matching the sibling
gate's style. All of AC8's six bite assertions plus AC9's three clean ones:

```rust
#[test] fn turbofish_i64_is_collected() {}
#[test] fn ascribed_let_is_collected() {}

#[test]
fn vec_of_tuple_with_option_i64_is_collected() {
    // Site #9's shape. A gate that misses this misses the site it was built for.
    let src = r#"
        fn f() {
            let rows: Vec<(String, Option<i64>)> = sqlx::query_as("SELECT a, b").fetch_all(p).await?;
        }
    "#;
    assert_eq!(decodes(src).len(), 1);
}

#[test]
fn ascribed_row_get_is_collected() {
    // The live shape at both feed_events mappers before task 4 swept them. Only a
    // synthetic test can prove the gate bites here now.
    let src = "fn f() { let id: i64 = r.get(\"id\"); }";
    assert_eq!(decodes(src).len(), 1);
}

#[test]
fn fn_return_type_covers_each_match_arm() {
    // `test_support.rs:109-113`'s shape: one fn return type, a decode in EACH arm.
    // Entry G's `count: 2` depends on this producing two records, not one.
    let src = r#"
        async fn scalar_i64(&self, sql: &str) -> Result<i64, sqlx::Error> {
            match self {
                A(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,
                B(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,
            }
        }
    "#;
    assert_eq!(decodes(src).len(), 2);
}

#[test]
fn turbofish_wins_over_the_enclosing_fn_return() {
    // `postgres/backup.rs:320-326`: `-> Result<i64, _>` around `query_scalar::<_, Option<i64>>`.
    // One record, target `Option<i64>` — or the seed allowlist can never match.
    let src = r#"
        async fn schema_version(c: &mut C) -> Result<i64, BackupError> {
            Ok(sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(version) FROM m")
                .fetch_one(c).await?.unwrap_or_default())
        }
    "#;
    let d = decodes(src);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].target, "Option < i64 >");
}

#[test] fn from_row_struct_field_is_collected() {}
#[test] fn typed_decode_is_not_collected() {}          // query_scalar::<_, PostId>
#[test] fn bool_and_string_targets_are_not_collected() {}
#[test] fn struct_literal_row_get_is_not_collected() {} // D6
#[test] fn unascribed_get_is_not_collected() {}         // D5 — the serde_json Map case

#[test] fn an_unallowlisted_i64_decode_is_flagged() {}  // proves it bites
#[test] fn an_allowlisted_decode_is_clean() {}
#[test] fn a_count_of_two_passes_on_two_and_fails_on_three() {}  // D7
#[test] fn an_entry_exempts_only_the_decode_it_names() {}
#[test] fn a_stale_entry_with_no_matching_site_is_reported() {}
```

**Run**

```console
$ cargo test --manifest-path xtask/Cargo.toml sqlx_newtype_decode
$ cargo xtask check
```

Expected: PASS, with `sqlx-newtype-decode` reported as an ok step.

**Then demonstrate the bite (AC12):** revert one swept site to its `i64` decode,
run `cargo xtask check`, confirm `sqlx-newtype-decode` fails and names the site,
restore it. Note the site and the message for the PR body (task 8). Manual
demonstration, not a committed test.

**Commit** —
`xtask: enumerate sqlx decode targets under storage/src and gate the i64 family (#715)`

---

## Task 8 — ADR, the bind-gate link, #716, and the PR notes

- [ ] Done

**Files** — `docs/adr/drafts/static-type-safety-gates-enumerate.md` (drafted
during `jaunder-start`), `xtask/src/steps/sqlx_newtype_bind_check.rs`, issue
#716

**Change**

Review the existing draft against what task 7 actually built and correct any
drift — the draft predates the gate and its conformance claims must match the
shipped code.

Confirm the draft carries: the general statement over all static type-safety
gates; `sqlx-newtype-decode` as conforming; `sqlx-newtype-bind` as
non-conforming on **both** counts (spelling search at `:95-101`, region-scoped
`ALLOWLIST` at `:60-62`); and the detection-versus-attribution split for #716.

Add the ADR link to `sqlx_newtype_bind_check.rs`'s module doc at the existing
"What it still cannot see" note (`:28-32`). Reference the draft **by path**
(`docs/adr/drafts/static-type-safety-gates-enumerate.md`) so
`cargo xtask adr promote` rewrites it to the assigned number at ship.

Update issue #716 so its scope matches what the ADR asserts (AC19) — its body
currently offers two options and its acceptance asks for "a decision recorded on
whether the gate should grow a parameter-shape rule". The ADR decides that. Edit
via `jaunder-issues`.

**PR body notes (AC6, AC12)** — write them here, where the PR is imminent,
rather than stranding them in a sweep task:

- Typing these ids does **not** make a `.bind` transposition a compile error —
  sqlx's `bind<T: Encode + Type>` is per-call generic, and the same is true of
  `FromRow` tuple positions. The protection is downstream: a `TagId` cannot
  reach a `PostId` parameter, field, or comparison. The issue's contrary claim
  is wrong and is corrected in the spec.
- The gate bite demonstration from task 7: which site was reverted and what
  `sqlx-newtype-decode` reported.

**Test** — `doc-links` and `adr-format` cover the draft; drafts are gitignored
and invisible to both (`docs/adr/drafts/README.md` §"Gate invisibility"), so the
link check applies once `promote` runs at ship.

**Run**

```console
$ cargo xtask check
```

Expected: PASS.

**Commit** — `docs: record the enumerate-don't-search gate principle (#715)`

---

## Self-review

- Every spec acceptance criterion maps to a task: AC1–5 → tasks 1–6; AC6 → task
  8; AC7–15 → task 7; AC16–19 → task 8; AC20 → every task's `cargo xtask check`.
- Tasks 1–6 are independently verifiable by `cargo check -p storage` plus the
  named suite, and each leaves the tree green. Task 7 is the first that can fail
  the tree, and by then the sweep is done.
- Task 7 is large because the xtask dead-code boundary makes it indivisible, not
  because it was under-decomposed; its internal 7a/7b/7c order is the working
  sequence.
- No task smuggles work the spec didn't authorise. `raw_ids`, the string decode
  family, and the bind direction are all explicitly out (#716, #728). Task 4's
  fourth `raw_ids` caller is a forced consequence of typing `purge_corrupt`, not
  a widening.
- Separable concerns were filed before planning (#728), not deferred to ship.
