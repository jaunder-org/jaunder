# Plan — absence is named at its source (#343)

Spec:
[`docs/superpowers/specs/2026-08-09-issue-343-rownotfound-class.md`](../specs/2026-08-09-issue-343-rownotfound-class.md)
Issue: [#343](https://github.com/jaunder-org/jaunder/issues/343)

## Review header

**Goal.** Stop `sqlx::Error` escaping `storage` for row access, so a
`RowNotFound` can no longer reach the error boundary as an anonymous pageable
bug. Delete the blanket `From<sqlx::Error> for InternalError`; make the
guarantee compiler- and clippy-enforced.

**Scope — in.** `StorageError` + the `fetch_exactly_one` wrapper; retyping 19
`sqlx::Error` payloads across 18 enums, the 9 `automock`'d traits plus
`InviteStorage` and `UserConfigStorage`, and every consumer that constructs
them; the 33-site `fetch_one` triage; the `clippy.toml` guard and a durable
self-test; `subscribe`'s atomic upsert; the ADR; amending #343.

**Scope — out.** `ErrorKind`/`ErrorClass` changes; `web/src/error/server.rs`'s
`project`; the connection/migration paths in `db.rs` (spec D4's row-access
boundary); `set_post_tags`'s insert-then-select (filed as a follow-up in Task
1).

**Tasks.**

- [x] **1.** File the separable follow-up issue. → #883
- [x] **2.** `StorageError`, its `From` impl, and the `fetch_exactly_one`
      wrapper. **Merged into Task 4** — see the note on Task 2 below.
- [x] **3.** `subscribe` becomes one atomic upsert (spec D5) — independent,
      closes today's only reachable race.
- [x] **4.** Slice: `subscriptions` — retype, triage, and the missing-seed
      backend test. Feasibility gate passed: `channels` has exactly one inbound
      FK (`subscriptions.channel_id`) and a fresh DB has no subscriptions, so
      the seed row deletes on both backends.
- [x] **5.** Slice: `posts` — mechanical retype, including `post_service` and
      its `atompub` consumers. **Merged with Task 6** — the retype breaks every
      bare `?` on a sqlx call, so the conversions cannot land separately.
- [x] **6.** Slice: `posts` — the `fetch_one` triage judgements (landed with 5).
- [x] **7.** Slice: `users`, `password`, `email`.
- [x] **8.** Slice: `sessions` — including `SessionDialect::touch_and_load`, a
      public trait method AC1 requires; retyping it removed the need for a
      hand-written `From<sqlx::Error>` here.
- [x] **9.** Slice: `media`, `feed_cache`, `feed_events` — including
      `media_manager` and `atompub`.
- [x] **9b.** Slice: `posts` **trait methods**. The Task 5/6 brief enumerated
      the four error payloads and missed that `PostStorage` has ~20 methods
      returning `sqlx::Result` directly — AC1 covers those too. Found by the
      Task 9 agent noticing `feed_urls_needing_catchup` still on `sqlx::Result`.
      **Lesson: retype every public fn and trait method returning `sqlx::Result`
      in the module, not just the payloads named in the brief** — the later
      briefs were reframed as a measurement (drive the `rg` count to its
      justified floor) rather than a list of line numbers.
- [x] **10.** Slice: `site_config` — 19 trait methods plus the `smtp` coupling.
- [x] **11.** Slice: `audiences`.
- [x] **12.** Slice: `invites`, `user_config`.
- [x] **13.** Slice: `backup`, `atomic`, `postgres/bootstrap` — including both
      `mod.rs` `AtomicOps` impls.
- [x] **14.** `test_support`, its `server/tests` consumers, and the final sweep.
- [x] **15.** `server/atompub`'s own `From` impl — the sqlx one is **deleted**
      (its last raw-`sqlx::Error` caller went with Task 12) and replaced by
      `From<StorageError>`.
- [x] **16.** Turn the clippy guard on; rewrite the 6 `server/tests` sites. Gate
      green on the first run — the triage that rode the slices was complete.
- [ ] **17.** Durable self-test proving the guard rejects a bare `fetch_one`.
- [x] **18.** Delete the blanket `From<sqlx::Error>` and its pinning test. The
      workspace still compiles, which is the proof no row-access path leaks.
- [x] **19.** ADR draft → `docs/adr/drafts/absence-is-named-at-its-source.md`
      (gitignored until `cargo xtask adr promote` numbers it at ship).
- [x] **20.** Amend #343's body — the superseded acceptance criterion is stated
      as superseded, with the replacement criteria.
- [ ] **21.** Full gate.

**Key risks and decisions.**

- **Ordering is load-bearing.** The guard (16) and the deletion (18) come
  _last_. Turning the guard on early fails all 33 sites at once; deleting the
  impl early breaks every call site at once. Every earlier task must leave the
  tree green on its own.
- **Slices follow the dependency graph, not module names.** A cold review found
  the first draft's slices would not compile: retyping `posts` breaks
  `post_service.rs` and three `atompub/mod.rs` sites; `media` breaks
  `media_manager.rs:513`; `site_config` breaks `smtp.rs`'s `classify`. Each
  slice's file list below is drawn from the real reverse-dependency set. **If a
  slice does not compile, widen that slice — do not defer the breakage to a
  later task.**
- **Zero lint suppressions.** Every site becomes `fetch_optional` or
  `fetch_exactly_one`; the wrapper itself is built on `fetch_optional`. See
  "Zero suppressions" below. Needing an `#[allow]` means a site was mis-triaged.
- **Retyping an error payload forces its `fetch_one` conversions into the same
  commit.** Rust does not chain `From`, so the moment `Internal` carries
  `StorageError`, a bare `?` on a raw sqlx call stops converting. Each retyped
  enum therefore keeps `Internal(#[from] StorageError)` **and** gains a
  hand-written `From<sqlx::Error>` wrapping via `StorageError::Db`. That is not
  a hole: `RowNotFound` cannot reach that arm, because `fetch_one` is banned.
- **The coverage gate rejects an unreachable arm.** Both fetch wrappers delegate
  absence to one shared `require_row`, so `MissingRow` is constructed on exactly
  one line, covered once by the subscriptions seed-deletion test — rather than
  one uncovered line per wrapper shape.
- **The `fetch_one` triage rides the slices**, not Task 16. By the time the
  guard turns on, no `fetch_one` should remain in `storage/src` at all; Task 16
  is then a green confirmation plus the 6 test-site rewrites.
- **`MissingRow` maps through `InternalError::server`, never `server_message`**
  — the latter discards the typed source (spec D2).
- **AC14's feasibility is unproven.** Deleting the seeded `local` channel row
  may be blocked by the FK from `subscriptions.channel_id`. Task 4 confirms it
  first and substitutes another required-row site if not.

**For agentic workers.** Drive with **`jaunder-iterate`**; delegate an
individual task via **`jaunder-dispatch`** where useful. Tick checkboxes in real
time.

## Zero suppressions

**This cycle adds no `#[allow]` or `#[expect]` at all**, so
CONTRIBUTING.md:111-116's approval gate is never engaged.

An earlier draft budgeted ~17 suppressions. All were avoidable:

- The **wrapper** is built on `fetch_optional`, not `fetch_one`, so it needs no
  suppression and never constructs a `RowNotFound`.
- The **10 "row-guaranteed" sites** route through the wrapper instead of keeping
  a `fetch_one` behind an `#[allow]`. Cost: one `what` string each. Benefit: if
  an `ON CONFLICT DO NOTHING` is ever added to one of the `INSERT … RETURNING`
  statements, the row silently becomes optional and the wrapper reports a named
  `MissingRow` — the suppression would have preserved that hazard behind a
  comment.
- The **6 `server/tests` sites** become `fetch_optional(…).await?.expect("…")`;
  `clippy.toml`'s `allow-expect-in-tests` already permits `.expect()` there.

So every one of the 33 sites becomes `fetch_optional` or `fetch_exactly_one`,
and `fetch_one` appears nowhere in the workspace. If a task seems to need a
suppression, that is a signal the site was mis-triaged — fix the site, and do
not add one without asking.

## Global constraints

- **Backend parity (ADR-0019).** Any dialect-file change lands in both
  `storage/src/sqlite/` and `storage/src/postgres/` in the same commit. Storage
  tests use the dual-backend template (`#[apply(backends)]`); a bare
  `#[tokio::test]` that should be dual-backend fails the `test-backend-pattern`
  guard.
- **No test in an ADR-0019 per-backend dialect file.** Tests live in
  `server/tests/<module>.rs` or an in-file `#[cfg(test)]` module where that is
  the crate's convention.
- **Every task ends green.** Run
  `devtool run --cwd <worktree> -- cargo xtask check` before committing; the
  pre-commit hook runs the full gate (**`jaunder-commit`**).
- **No `Co-Authored-By` trailer.** Commit messages reference `(#343)`.

## Tasks

### Task 1 — File the separable follow-up issue

**Files:** none in-tree (tracker only). Use **`jaunder-issues`**.

**`set_post_tags`: insert-then-select-the-id is the same TOCTOU shape as #343's
`subscribe`.** Sites: `storage/src/sqlite/posts.rs:182-188`,
`storage/src/postgres/posts.rs:169-175`. Unreachable today because nothing
issues `DELETE FROM tags`; #343 gives it a named `MissingRow` but does not make
it atomic. The fix mirrors D5: `ON CONFLICT DO UPDATE … RETURNING`. Label
`ready-for-agent`, milestone **Correctness & data integrity**, link #343.

---

### Task 2 — `StorageError`, its `From` impl, and `fetch_exactly_one`

**Files:** `storage/src/error.rs` (new), `storage/src/lib.rs`.

```rust
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database operation failed")]
    Db(#[from] sqlx::Error),
    #[error("expected row is missing: {what}")]
    MissingRow { what: &'static str },
}
```

`From<StorageError> for InternalError`: `Db(e)` → `InternalError::storage(e)`;
`MissingRow` → `InternalError::server(error)` — **not** `server_message`, which
discards the typed source.

`fetch_exactly_one` is built on **`fetch_optional`** — no `fetch_one`, no
suppression, and no `RowNotFound` ever constructed:

```rust
match query.fetch_optional(pool).await? {
    Some(row) => Ok(row),
    None => Err(StorageError::MissingRow { what }),
}
```

Settle whether `QueryAs` and `QueryScalar` need two functions or one extension
trait — both are in live use.

**Correction, found during execution:** this task cannot stand alone. With no
caller, `fetch_exactly_one` and `fetch_exactly_one_scalar` are `dead_code`,
which is a hard error under the gate's `-D warnings` — an unused wrapper cannot
be committed. **Task 2 therefore lands as part of Task 4**, whose
`local_channel_id` is the wrapper's first caller. Task 3 was resequenced ahead
of it (it needs no wrapper) so it stays independently revertible, as the spec
intends.

A third test named in an earlier draft — asserting the typed error survives into
the source chain by downcasting — is **not written**: `InternalError` does not
implement `Error` and keeps `source` private, so the house pattern
(`host/src/error.rs:554`) asserts the _rendered_ chain via `operator_message()`
instead. That distinguishes `server` from `server_message` only for the `Db`
arm, which carries a nested cause; `MissingRow` has none today, so the two would
render identically. The `Db` arm is tested; the `MissingRow` rationale is
documented in the code rather than asserted.

**Test** (in-file `#[cfg(test)]`, modelled on `storage/src/posts.rs:3789-3805`):

- `storage_error_db_maps_to_storage_kind_and_bug_class`
- `storage_error_missing_row_maps_to_internal_kind_bug_class_and_names_the_row`
  — asserts `what` appears in `operator_message()` **and** that the source chain
  still downcasts to `StorageError` (exactly what `server_message` would have
  broken)
- `fetch_exactly_one_maps_an_absent_row_to_missing_row` — the `None` arm of
  `fetch_optional`; note it never sees a `RowNotFound`, because none is
  constructed
- `fetch_exactly_one_propagates_driver_errors_as_db`

**Run:** `cargo nextest run -p storage error::` — expect FAIL, then PASS.

---

### Task 3 — `subscribe` becomes one atomic upsert (spec D5)

**Files:** `storage/src/{sqlite,postgres}/subscriptions.rs`,
`storage/src/subscriptions.rs`, `server/tests/storage/mod.rs`.

Both dialects: `INSERT_SUBSCRIPTION` becomes one statement ending

```
ON CONFLICT (author_user_id, channel_id, subscriber_ref)
DO UPDATE SET subscriber_ref = excluded.subscriber_ref
RETURNING subscription_id
```

in each dialect's placeholder style. Delete `SELECT_SUBSCRIPTION_ID` from the
trait (`subscriptions.rs:93`) and both impls. Rewrite the `INSERT_SUBSCRIPTION`
doc comment (`:87-89`): no longer "no-ops on the conflict", and state that it
returns the id row on both paths. `subscribe` issues one query; signature
unchanged.

**Test:** `subscribe_is_idempotent_and_active`
(`server/tests/storage/mod.rs:301`) must pass on both backends. Do **not** add a
`created_at` equality assertion — SQLite's default is second-granularity, so it
passes vacuously. The existing `assert_eq!(id1, id2)` plus the single-row
`list_subscribers` assertion are the evidence.

**Run:** `cargo nextest run -p server subscribe_is_idempotent_and_active`.

**Watch for:** `SQLITE_BUSY` (`sqlite/sessions.rs:19-20` records the
correlated-subquery hazard; ours is uncorrelated). If it appears, fall back to
the transaction form for SQLite only and record the split in the spec.

---

### Task 4 — Slice: `subscriptions`

**Files:** `storage/src/subscriptions.rs`, `web/src/subscriptions/api.rs`,
`server/tests/storage/mod.rs`.

Retype the trait's row-access methods (including `local_channel_id`,
`subscriptions.rs:80`) to `Result<_, StorageError>`; update the `automock`
consumers. `web/src/subscriptions/api.rs:28,30,44` keep their bare `?`.

Triage the 4 `fetch_one` sites: `:167` disappeared in Task 3; `:204` and `:215`
are `COUNT`-shaped — route through `fetch_exactly_one` naming the aggregate;
`:250` becomes `fetch_exactly_one` naming the seeded `local` channel row.

**Do the feasibility gate FIRST.** `subscriptions.channel_id` FKs to `channels`,
and SQLite enforcement depends on the harness's `foreign_keys` pragma. If the
seeded row cannot be deleted on a fresh database, substitute another
required-row site and record the substitution here and in the PR.

**Test (AC14):** dual-backend — remove the seeded `local` channel row, call
`local_channel_id`, assert `MissingRow` naming that row.

**Run:** `cargo nextest run -p server subscriptions`.

---

### Task 5 — Slice: `posts`, mechanical retype

**Files:** `storage/src/posts.rs`, `storage/src/post_service.rs`,
`storage/src/{sqlite,postgres}/posts.rs`, `web/src/posts/api.rs`,
`server/src/atompub/posts.rs`, `server/src/atompub/mod.rs`.

Retype the four payloads (`posts.rs:155,169,379,401`) plus `post_service.rs:142`
(`PerformUpdateError::Storage`) and `:309` (`PerformCreationError::Storage`) —
the spec's first inventory missed these two, and `post_service.rs` holds
`From<UpdatePostError> for PerformUpdateError`, so `posts` does not compile
without it. `UpdatePostError`, `ListByTagError` and `TaggingError` keep their
domain variants and projections; only their `Internal`/`Storage` arms retype.
Retype the trait method `post_id_for_idempotency_key` (`posts.rs:591`).

`server/src/atompub/mod.rs:462,482` construct `PerformCreationError::Storage(…)`
and `PerformUpdateError::Storage(…)` — they break with this task and are fixed
here, not deferred.

**Test:** the `UpdatePostError` / `ListByTagError` mapper tests
(`posts.rs:3789-3805`, `:3873-3876`) update for the new payload type and must
still assert the same kind and class.

**Run:** `cargo nextest run -p storage posts`, then
`cargo nextest run -p server posts`.

---

### Task 6 — Slice: `posts`, the `fetch_one` triage

**Files:** `storage/src/posts.rs`, `storage/src/{sqlite,postgres}/posts.rs`.

Split from Task 5 deliberately: the spec says risk concentrates in the triage
judgements, not the signature churn, so they get their own reviewable commit.

Nine decisions, each resolving to `fetch_optional` or `fetch_exactly_one`.
`posts.rs:1579` and `:1673` (`SELECT COUNT(*) > 0`) and `posts.rs:2217`
(`INSERT … RETURNING`) are row-guaranteed but still route through
`fetch_exactly_one` — the `what` string documents the expectation and the
unreachable arm catches a later `ON CONFLICT` being added. `posts.rs:1256`,
`sqlite/posts.rs:124,167,188` and `postgres/posts.rs:126,175` likewise take
`fetch_exactly_one` with a naming `what`.

**Run:** `cargo nextest run -p server posts`.

---

### Task 7 — Slice: `users`, `password`, `email`

**Files:** `storage/src/{users,password,email}.rs`, `web/src/auth/server.rs`,
`web/src/timeline/server.rs`.

Retype `users.rs:55`, `password.rs:27`, `email.rs:27`; update the `users` trait,
its mock, and consumers at `auth/server.rs:330,372` and
`timeline/server.rs:310,333`. Triage `users.rs:283` — plain
`INSERT … RETURNING`; route through `fetch_exactly_one` naming the inserted user
row.

**Run:** `cargo nextest run -p storage users password email`, then the `server`
auth suites.

---

### Task 8 — Slice: `sessions`

**Files:** `storage/src/sessions.rs`,
`storage/src/{sqlite,postgres}/sessions.rs`.

Retype `sessions.rs:41` and the trait; update mock and consumers. No `fetch_one`
sites — `sqlite/sessions.rs:35` already uses `fetch_optional`.

**Run:** `cargo nextest run -p server sessions`.

---

### Task 9 — Slice: `media`, `feed_cache`, `feed_events`

**Files:** `storage/src/{media,feed_cache,feed_events}.rs`,
`storage/src/media_manager.rs`, dialect files, `server/src/feed/handlers.rs`,
`server/src/feed/worker.rs`, `server/src/atompub/mod.rs`.

Retype `media.rs:52,63`, `feed_cache.rs:29`, `feed_events.rs:38`. Update three
traits, their mocks, and the feed consumers at `handlers.rs:221,241` and
`worker.rs:386,429,584,632`. **Also** `media_manager.rs:513`, which constructs
`CreateMediaError::Internal(sqlx::Error::PoolClosed)`, and `atompub/mod.rs:494`
(`DeleteMediaError::Internal(…)`) — both break with this slice.

Triage `feed_events.rs:250` (`INSERT … RETURNING`), `sqlite/media.rs:18` and
`postgres/media.rs:21` (`COALESCE(SUM(...),0)` aggregates) — all three
row-guaranteed, all three routed through `fetch_exactly_one` with a naming
`what`.

**Run:** `cargo nextest run -p server feed media`.

---

### Task 10 — Slice: `site_config` (and `smtp`)

**Files:** `storage/src/site_config.rs`, `storage/src/smtp.rs`, plus the ~21
bare-`?` consumers (they keep their `?` and lift through `From<StorageError>`).

Larger than it looks and second only to `posts`: **19 trait methods** return
`sqlx::Result`, most with default bodies that `?` through `self.get(...)`.
Retype all of them, plus `label_decode_error` (`site_config.rs:321`), which
constructs `sqlx::Error::ColumnDecode` by hand.

**`smtp.rs` is coupled and must land here.**
`smtp.rs:107 fn classify(error: sqlx::Error)` consumes exactly these results and
matches on `sqlx::Error::ColumnDecode`, and `smtp.rs:66`
(`SmtpConfigError::Read`) is a public `sqlx::Error` payload the spec's first
inventory missed.

Triage `site_config.rs`'s `fetch_one` sites (all `fetch_optional` today —
confirm none remain).

**Run:** `cargo nextest run -p storage site_config smtp`, then
`cargo nextest run -p server site`.

---

### Task 11 — Slice: `audiences`

**Files:** `storage/src/audiences.rs`, `web/src/audiences/api.rs`.

Retype the trait and its mock consumers. Triage `audiences.rs:184`
(`INSERT … RETURNING`) — row-guaranteed, routed through `fetch_exactly_one`
naming the inserted audience row.

**Run:** `cargo nextest run -p server audiences`.

---

### Task 12 — Slice: `invites`, `user_config`

**Files:** `storage/src/invites.rs`, `storage/src/user_config.rs`, plus
`web/src/invites/api.rs` and `server/src/commands.rs`.

Both traits are public row-access surfaces the spec's first inventory missed:
`InviteStorage` (`invites.rs:45`, methods `:49,:59`) and `UserConfigStorage`
(`user_config.rs:17`, methods `:19,:22,:25`, plus the public free fns
`:38,:56`). AC1 requires both.

**Run:** `cargo nextest run -p server invites user_config`.

---

### Task 13 — Slice: `backup`, `atomic`, `postgres/bootstrap`

**Files:** `storage/src/{backup,atomic}.rs`,
`storage/src/postgres/bootstrap.rs`, `storage/src/{sqlite,postgres}/backup.rs`,
`storage/src/sqlite/mod.rs`, `storage/src/postgres/mod.rs`,
`storage/src/postgres/{schema,teardown}.rs`.

Retype `backup.rs:74`, `atomic.rs:30,72`, `postgres/bootstrap.rs:26`. Both
`mod.rs` files hold the `AtomicOps` impls (`sqlite/mod.rs:185,280`;
`postgres/mod.rs:89,155`), so they retype with `atomic.rs`.

Triage: `sqlite/backup.rs:296`, `postgres/backup.rs:325`,
`postgres/schema.rs:27`, `postgres/teardown.rs:36`, `sqlite/mod.rs:154,246`,
`postgres/mod.rs:130,311`, and the four test-gated sites at
`backup.rs:734,740,771,776`.

**Note:** the `backup.rs` sites are inside `#[cfg(test)]` and invisible to
clippy without `--all-targets`. The gate passes it; a bare local
`cargo clippy -p storage` will under-report.

**Run:** `cargo nextest run -p server backup`.

---

### Task 14 — `test_support`, its consumers, and the final sweep

**Files:** `storage/src/test_support.rs` and the `server/tests/**` call sites
that use its helpers.

Retype the public helpers at `:90,109,126` — their `Result<_, sqlx::Error>`
ripples into `server/tests`, so those consumers change here too. Triage the
`fetch_one` sites at `:111,112,555`.

**Sweep (AC1):** confirm by search that no public row-access fn, trait method,
or error payload in `storage/src` still names `sqlx::Error`, and that the only
remaining mentions are the D4-exempt connection paths — `db.rs:179` (the
`FromStr::Err` associated type) and `db.rs:247,262,286` — each gaining a comment
citing the row-access boundary. If the sweep finds a straggler, fix it here.

**Run:** `devtool run -- cargo xtask check`.

---

### Task 15 — `server/atompub`'s own `From` impl

**Files:** `server/src/atompub/mod.rs`.

`:194` declares `impl From<sqlx::Error> for HandlerError`. Its sources now yield
`StorageError`, so retype to `From<StorageError>`. (The _call sites_ in this
file were already fixed in Tasks 5 and 9; this is the impl itself.)

**Run:** `cargo nextest run -p server atompub`.

---

### Task 16 — Turn the clippy guard on

**Files:** `clippy.toml`, plus the 6 `server/tests` call sites.

Add all six sqlx `fetch_one` definitions under `disallowed-methods` by their
**`sqlx_core::`** paths — the `sqlx::` facade resolves but matches nothing:

- `sqlx_core::query::Query::fetch_one`
- `sqlx_core::query::Map::fetch_one`
- `sqlx_core::query_as::QueryAs::fetch_one`
- `sqlx_core::query_scalar::QueryScalar::fetch_one`
- `sqlx_core::raw_sql::RawSql::fetch_one`
- `sqlx_core::executor::Executor::fetch_one`

Each with a `reason` naming `fetch_exactly_one`. The guard is
**workspace-wide**, so rewrite the 6 legitimate calls outside `storage/src`
(`server/tests/storage/mod.rs:273,279,5804,5810`,
`server/tests/misc/postgres/commands.rs:43,119`) as
`fetch_optional(…).await?.expect("…")` — permitted by `clippy.toml`'s
`allow-expect-in-tests`, and a better failure message than a bare `fetch_one`
panic. **Do not annotate them.**

**Check:** `xtask/src/steps/sqlx_newtype_decode_check.rs:2122,2136` contains
fixture text with `.fetch_one(...)`. Confirm those fixtures are not compiled or
linted; if they are, the new guard will fire on them.

**Verify:** `devtool run -- cargo xtask check` green, and **no
`#[allow]`/`#[expect]` for `clippy::disallowed_methods` exists anywhere** (AC6).
If the gate is red, a site was missed in Tasks 4–14 — fix the site, never add a
suppression.

**Note:** an unresolvable path emits `does not refer to a reachable function`, a
hard error under `-D warnings`. That is the intended safety property; do not
silence it with `allow-invalid`.

---

### Task 17 — Durable self-test for the guard

**Files:** `xtask/src/steps/` and its in-file tests.

AC5 requires a check that **executes**, not a log pasted into the PR. Plant a
fixture containing a bare `fetch_one`, assert clippy rejects it, and assert a
clean tree passes. Closest precedent is
`xtask/src/steps/sqlx_newtype_decode_check.rs`, which already plants sqlx
fixtures containing `.fetch_one(...)`; `thin_components.rs` shows the
fixture-driven test shape.

**Run:** `cargo nextest run -p xtask` — expect FAIL, then PASS.

---

### Task 18 — Delete the blanket `From<sqlx::Error>`

**Files:** `host/src/error.rs`.

Delete the impl (`:355-362`) and its pinning test
`from_sqlx_error_matches_storage_constructor` (`:635-644`). Keep
`InternalError::storage` — `StorageError::Db` maps through it — with its
coverage in `constructors_set_kind_and_class` untouched.

**Verify:** the workspace compiles. Any breakage means a row-access path was
missed earlier; fix it there rather than reinstating the impl.

**Run:** `devtool run -- cargo xtask check`.

---

### Task 19 — ADR draft

**Files:** `docs/adr/drafts/<slug>.md` — **numberless**, heading
`# ADR-DRAFT: <Title>` (**`jaunder-adr`**).

State the **principle**: absence is modelled inside `storage` as an `Option` or
a named `MissingRow` and never escapes as a raw driver error; a `RowNotFound` at
the boundary is a caller defect. Record D4's row-access boundary (connection
paths exempt), D2's promotion rule, and that `fetch_one` is reachable only
through one audited wrapper.

Cite ADR-0011 (class `Bug` drives ERROR and the `jaunder.errors` pager),
ADR-0017 (§1 absence as a modelled state, §3 typed sources, the remove-footguns
driver), ADR-0059 (:150 the classification grant, handed back by moving
classification into `storage`; :146-154 the layering floor), and ADR-0016 (:221
— **correct the record**: `disallowed-methods` was hypothetical there and never
implemented; this cycle establishes it, and the guard is workspace-wide).

Note that ADR-0059:138's "boundary field set unchanged" still holds — no new
`ErrorKind`/`ErrorClass` — while some field _values_ change, since sites moving
from `InternalError::storage` to `server` shift `error.kind` from `Storage` to
`Internal`.

**Note:** `docs/adr/drafts/` is **gitignored** (CONTRIBUTING.md:126-129), so the
draft lives out of git until `cargo xtask adr promote` numbers it at ship. Do
not expect it in the PR diff.

---

### Task 20 — Amend #343's body

**Files:** none (tracker only). Use **`jaunder-issues`**.

Edit the body — do not merely comment. Its first acceptance bullet ("a benign
`RowNotFound` … does not log at ERROR or count as a bug on `jaunder.errors`") is
**superseded**: this cycle keeps such a failure at class `Bug` deliberately and
makes it legible instead. State the replacement acceptance and why, so a later
reader does not hold the merged code to a criterion it intentionally does not
meet.

---

### Task 21 — Full gate

**Run:** `devtool run --cwd <worktree> -- cargo xtask validate` — the full
`{sqlite,postgres}×{chromium,firefox}` matrix. Use Bash background mode; it is
long.

**Verify:**

- Gate green (AC22).
- The follow-up issue from Task 1 exists; #343's body is amended (AC21).
- The ADR draft exists on disk, ready for `adr promote` at ship (AC19, AC20).
- **AC10:** the PR body argues any bespoke error enum added beyond
  `StorageError`, or states that none was added.
