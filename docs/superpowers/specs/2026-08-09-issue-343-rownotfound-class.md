# Spec — absence is named at its source; `sqlx::Error` stops escaping `storage` (#343)

Issue: [#343](https://github.com/jaunder-org/jaunder/issues/343) Milestone:
Observability & diagnostics

## Problem

`impl From<sqlx::Error> for InternalError` (`host/src/error.rs:355-362`)
delegates to `InternalError::storage`, mapping **every** `sqlx::Error` —
`RowNotFound` included — to kind `Storage`, class `Bug`. `ErrorClass::Bug` logs
at ERROR and increments `jaunder.errors{error.class="bug"}`, which is the pager
(ADR-0011).

Because this is a `From` impl it fires on a bare `?`. #334 added it, so any
storage helper returning `sqlx::Result` that a server fn `?`s can lift a missing
row into a pageable ERROR — an operator woken for a non-event.

The issue proposes special-casing `RowNotFound` to a lower class. **This spec
rejects that and fixes the cause instead.** See AC21: #343's own acceptance
criterion is superseded and must be amended.

## What was measured

Counts below were derived by reading the tree, not by a single command. Where an
acceptance criterion depends on a count, the criterion names the artifact that
makes it checkable rather than asking a reviewer to re-derive the number.

- **67** `async fn`s in `storage/src` return `sqlx::Result<…>`; **76** call
  sites across `web/src`, `server/src`, `storage/src`, `host/src` and `cli/src`
  lift one by bare `?` (a further 15 use an explicit `map_err`).
- **33** `fetch_one` sites in `storage/src` versus **47** `fetch_optional`, so
  "absence is an `Option`" is the established idiom. The 33 are the triage
  surface, and the clippy guard (D3) enumerates them mechanically — the plan
  does not rely on this number being right.
- **19 variants across 18 public error enums** in `storage/src` carry
  `sqlx::Error` in their public payload: `backup.rs:74`,
  `posts.rs:155,169,379,401`, `feed_events.rs:38`, `media.rs:52,63`,
  `feed_cache.rs:29`, `postgres/bootstrap.rs:26`, `sessions.rs:41`,
  `email.rs:27`, `users.rs:55`, `password.rs:27`, `atomic.rs:30,72`,
  `post_service.rs:142,309`, `smtp.rs:66`. (An earlier draft said "16 across 15"
  and missed the last three, all public via `lib.rs:60,72`.)
- Two further public row-access traits carry `sqlx::Result` and are in scope:
  `InviteStorage` (`invites.rs:45`, methods `:49,:59`) and `UserConfigStorage`
  (`user_config.rs:17`, methods `:19,:22,:25`, plus the public free fns
  `:38,:56`).
- **9** storage traits are `mockall::automock`'d (`subscriptions.rs:38`,
  `posts.rs:569`, `site_config.rs:30`, `audiences.rs:72`, `sessions.rs:61`,
  `users.rs:128`, `feed_events.rs:103`, `media.rs:83`, `feed_cache.rs:32`).
  Consumers construct the old error types directly at
  `server/src/feed/handlers.rs:221,241`,
  `server/src/feed/worker.rs:386,429,584,632`, `web/src/posts/api.rs:914`,
  `web/src/timeline/server.rs:310,333`, `web/src/auth/server.rs:330,372`.

**Sites that can produce `RowNotFound` today.** Of the 76 bare-`?` sites,
exactly two:

- `local_channel_id` (`storage/src/subscriptions.rs:248-253`) — `fetch_one` on
  `SELECT channel_id FROM channels WHERE name = 'local'`, a schema-seeded row.
  Callers: `web/src/subscriptions/api.rs:28` and `:44`.
- `subscribe` (`storage/src/subscriptions.rs:156-169`) —
  `INSERT … ON CONFLICT DO NOTHING` then a **separate**
  `SELECT_SUBSCRIPTION_ID … fetch_one`. A concurrent delete between the two
  statements yields `RowNotFound`. Caller: `web/src/subscriptions/api.rs:30`.

Two further sites produce `RowNotFound` but reach the boundary through a **typed
enum** rather than the blanket `From`, and are pageable false alarms all the
same. They are the key evidence that typing alone does not fix this:

- `set_post_tags` (`storage/src/sqlite/posts.rs:182-188`,
  `storage/src/postgres/posts.rs:169-175`) — escapes as `TaggingError::Internal`
  → `InternalError::server`, kind `Internal`, class **`Bug`** (`posts.rs:389`).
  Unreachable today only because nothing ever issues `DELETE FROM tags`.
- `posts.rs:1256` — the re-read of an updated post, wrapped in
  `UpdatePostError::Internal`, which maps straight back to
  `InternalError::storage`, class `Bug` (`posts.rs:182`, pinned at
  `:3803-3805`). Safe only because posts are soft-deleted and no
  `DELETE FROM posts` exists.

## The trap, stated precisely

A false page requires two things: an author picks `fetch_one` where the row may
be absent, **and** the resulting `RowNotFound` reaches the boundary still
classified as a bug. Fixing only the second is reclassification, which this spec
rejects (D1). Fixing only the first is a convention that decays. Both halves are
closed here.

**A typed error does not by itself exclude `RowNotFound`.** Any storage error
type needs a catch-all arm to carry pool timeouts and I/O failures;
`RowNotFound` rides through that arm untouched. `UpdatePostError` and
`TaggingError` are the proof — both are typed, both still emit class `Bug` on a
missing row. So the refactor is worth doing only because D3 also removes the
ability to _reach_ that arm with a `RowNotFound`.

**Verified sqlx property (the load-bearing one).** In sqlx 0.8.6,
`Error::RowNotFound` is constructed at exactly three sites, all `fetch_one`:
`query.rs:464` (`Map::fetch_one`), `query_as.rs:170`, and `executor.rs:126`
(`Executor::fetch_one`); `query_scalar` and `raw_sql` delegate to those. Nothing
in `sqlx-postgres` or `sqlx-sqlite` produces it. `execute`, `fetch_all` and
`fetch_optional` therefore **cannot** produce `RowNotFound`, which is what makes
D2's convenience `#[from]` safe.

## Decisions

### D1 — Absence is named at its source; the blanket `From<sqlx::Error>` is deleted

A `RowNotFound` reaching the boundary means a call site used `fetch_one` where
the row may legitimately be absent. That is a **caller defect**, not a
misclassification by the callee.

The blanket `impl From<sqlx::Error> for InternalError` is **removed**, which is
ADR-0017's "remove footguns rather than document around them" applied literally.

_Rejected:_ mapping `RowNotFound` to `NotFound`/`Client`. It is a **wire**
change, not merely an observability one: `project`
(`web/src/error/server.rs:71`) turns kind `NotFound` into
`WebError::NotFound { message }`, so a masked 500 would silently become a 404
whose body is the conversion's public message — and a `From` impl has no
resource name to put there.

_Rejected:_ `Storage`/`Transient` (WARN, wire unchanged). Cheaper, but
`Transient` means "retryable infrastructure failure"; a missing row is not
retryable, so this mislabels the event to buy silence.

_Rejected:_ keeping the blanket impl and documenting the rule. That was this
cycle's original scope; it was promoted precisely because documentation does not
contain a latent trap.

### D2 — `StorageError` is the shared base; bespoke enums where they earn it

```rust
// storage/src/error.rs
#[derive(Debug, Error)]
pub enum StorageError {
    /// Infrastructure failure — pool, I/O, protocol, constraint. Never absence.
    #[error("database operation failed")]
    Db(#[from] sqlx::Error),
    /// A row the caller requires is absent. `what` names it for the operator.
    #[error("expected row is missing: {what}")]
    MissingRow { what: &'static str },
}
```

`From<StorageError> for InternalError` maps `Db(e)` to
`InternalError::storage(e)` (kind `Storage`, class `Bug`) and `MissingRow` to
**`InternalError::server(error)`** — _not_ `server_message`. `server_message`
builds its source from `anyhow::Error::msg(String)`
(`host/src/error.rs:191-199`), discarding the typed error; `server` keeps the
typed `StorageError` in the chain and still renders the naming text through
`Display`. That preserves ADR-0017 §3's typed-source rule, which the blanket
impl's own doc comment (`host/src/error.rs:351-353`) claims as a virtue.

**Both arms still page.** `MissingRow` is not a downgrade; it is a _legible_
bug, replacing `"storage operation failed"` with a message naming the missing
row.

Retaining `#[from] sqlx::Error` keeps a bare `?` ergonomic inside `storage` for
`execute`, `fetch_all` and `fetch_optional`, none of which can produce
`RowNotFound` (verified above). Stated plainly: an **allowlisted** `fetch_one`
(D3) still routes a `RowNotFound` into `Db` → class `Bug` with the generic
message — i.e. today's behaviour. The guarantee is "safe modulo the allowlist",
and AC7 is what keeps the allowlist honest.

**Promotion rule.** A fn returns a bespoke enum instead of `StorageError` when
**a caller must branch on the failure to produce different observable
behaviour** — a different HTTP status, or a different UI outcome. Otherwise it
returns `StorageError`. `UpdatePostError` (`NotFound`/`Unauthorized` project
to 404) and `ListByTagError` (`TagNotFound` becomes an empty list) both satisfy
the rule and keep their domain variants. Every enum in the 15 listed above has
its `sqlx::Error` payload retyped to `StorageError`; enums whose _only_ content
was that payload collapse into `StorageError` outright.

Absence a caller must merely _handle_, rather than branch on, stays an `Option`
via `fetch_optional` — unchanged, and still the dominant idiom.

### D3 — `fetch_one` is disallowed; one sanctioned wrapper names the row

**This mechanism was spiked and proven before approval; the spike is recorded
here because two of the spec's earlier claims about it were wrong.**

Correcting the record: `disallowed-methods` is **not** established practice in
this repo. `clippy.toml` has no such section today, ADR-0016:221 mentions it
only hypothetically ("clippy `disallowed-methods` **if it binds**, else a
scanning test") and was never implemented in either form, and
`xtask/src/steps/no_full_reload_check.rs:3-4` records the repo _rejecting_ it
for the nearest analogous guard.

**Spike result (clippy 0.1.95, this worktree):**

- It binds. With the paths below, clippy flagged **33 of 33** `fetch_one` sites
  in `storage/src`.
- An unresolvable entry is **loud, not silent** — the failure mode most feared.
  A deliberately bogus control entry produced
  `warning: … does not refer to a reachable function`. The gate already runs
  `cargo clippy --all-targets -- -D warnings`
  (`xtask/src/steps/static_checks.rs:56`), so a stale or mistyped path is a
  **hard build failure**.
- The matching paths are the `sqlx_core::` ones, not the `sqlx::` facade — sqlx
  re-exports from `sqlx_core`, and the facade path resolves but matches no call
  site. `sqlx_core::query_as::QueryAs::fetch_one` covers 10 sites and
  `sqlx_core::query_scalar::QueryScalar::fetch_one` covers 17. The remaining
  definitions — `query::Query`, `query::Map`, `raw_sql::RawSql`,
  `executor::Executor` — resolve cleanly with no current call sites and are
  listed anyway, so a future use is caught rather than silently admitted.
- Coverage needs `--all-targets`: without it, 6 test-gated sites in `backup.rs`,
  `postgres/schema.rs` and `postgres/teardown.rs` are invisible. The gate
  already passes it.
- The guard is **workspace-wide** — `clippy.toml` is repo-root and the lint
  fires on resolved `DefId`s regardless of crate. Roughly six legitimate
  `fetch_one` calls outside `storage/src`
  (`server/tests/storage/mod.rs:273,279,5804,5810`,
  `server/tests/misc/postgres/commands.rs:43,119`) each need their own
  `#[allow]`.

A single wrapper in `storage/src/error.rs` is the one door for a required row:

```rust
// Contract, not a final signature — the plan settles the generics.
async fn fetch_exactly_one<…>(query: …, pool: …, what: &'static str)
    -> Result<O, StorageError>
{
    match query.fetch_optional(pool).await? {
        Some(row) => Ok(row),
        None => Err(StorageError::MissingRow { what }),
    }
}
```

**It is built on `fetch_optional`, not `fetch_one`** — so it needs no lint
suppression, and no `RowNotFound` is ever constructed anywhere in the tree.
Requiring `what` is the forcing function: an author cannot fetch a required row
without writing down which row it is.

**Zero suppressions (decided during plan review).** An earlier draft kept
`fetch_one` at ten "row-guaranteed" sites (`COUNT`/aggregate and
`INSERT … RETURNING`) behind `#[allow]`s justified by a comment. That is
rejected: routing them through the wrapper costs one `what` string each and
catches the very invariant the comment was asking a reader to take on trust — if
an `ON CONFLICT DO NOTHING` is ever added to one of those `INSERT … RETURNING`
statements, the row silently becomes optional and the wrapper reports a named
`MissingRow` instead of an unexplained failure. The unreachable arm lives in the
shared wrapper and is covered by its own unit test, so the dead path costs
nothing.

`fetch_one` therefore appears **nowhere in the workspace**. The rule the ADR
states is absolute rather than "banned except in these places", and
CONTRIBUTING.md:111-116's suppression-approval gate is never engaged.

`query_scalar(…).fetch_one` is a distinct type from `query_as(…).fetch_one`; the
plan decides whether that needs a second wrapper or an extension trait covering
both. The invariant is that sanctioned `fetch_one` calls live in exactly one
module of `storage`.

Each of the 33 sites becomes exactly one of two things: **`fetch_optional`**,
where the row may legitimately be absent and the caller receives an `Option`, or
**`fetch_exactly_one`**, where the row is required and is named. There is no
third option — no site keeps a bare `fetch_one`.

The six `server/tests` sites caught by the workspace-wide guard become
`fetch_optional(…).await?.expect("…")`, which `clippy.toml`'s
`allow-expect-in-tests` already permits and which gives a better failure message
than a bare `fetch_one` panic.

### D4 — `sqlx::Error` stops escaping the `storage` crate, for row access

**The boundary is row access, not the sqlx dependency.** Connection, migration
and bootstrap failures never cross a server-fn boundary — they abort startup —
so they are deliberately exempt and keep their `sqlx` types:
`storage/src/db.rs:179`
(`impl FromStr for DbConnectOptions { type Err = sqlx::Error }`) and
`db.rs:247,262,286` (`open_database`, `open_existing_database`,
`database_is_empty`). Retyping them would add ceremony to paths where
`MissingRow` is meaningless.

Everything that reads or writes rows changes:

- Every public row-access fn in `storage/src` returns `Result<_, StorageError>`
  or a bespoke enum per D2 — including the trait methods
  `SubscriptionStorage::local_channel_id` (`subscriptions.rs:80`) and
  `post_id_for_idempotency_key` (`posts.rs:591`), and the public helpers in
  `test_support.rs:90,109,126`.
- The 15 enums listed above have their `sqlx::Error` payloads retyped.
- The 9 `automock`'d traits change with their trait definitions, and the 11
  consumer sites in `server/` and `web/` that construct the old error types are
  updated. **This is real cross-crate churn** and is budgeted here rather than
  discovered during implementation.
- `server/src/atompub/mod.rs:194` declares its own
  `impl From<sqlx::Error> for HandlerError`. Its sources now yield
  `StorageError`, so it is retyped to `From<StorageError>`.

The 76 bare-`?` call sites keep their bare `?`, lifting through
`From<StorageError> for InternalError` instead, so #334's ergonomics survive.

**What D1's deletion does and does not rest on.** It does _not_ rest on "no
`sqlx::Error` exists outside `storage`" — that is false, and stays false:
`server/src/cli.rs:93,144` carry `Url(#[from] sqlx::Error)` from
`PgConnectOptions` parsing, `server/src/test_support.rs:18-27` calls
`sqlx::SqlitePool::connect_with` and `sqlx::migrate!` directly, and
`storage/src/postgres/mod.rs:118,204,265` construct `sqlx::Error::Io` by hand.
It rests on the narrower and true claim that **no `sqlx::Error` reaches a
server-fn boundary** once row access is retyped, so the impl has no remaining
legitimate consumer. Those startup and CLI paths handle their errors locally and
never lift them into an `InternalError`.

### D5 — `subscribe` becomes one atomic statement

Independent of the typing work: a genuine TOCTOU race that no error type
removes.

```sql
INSERT INTO subscriptions (author_user_id, channel_id, subscriber_ref, status_id)
VALUES (?, ?, ?, (SELECT status_id FROM subscription_statuses WHERE name = ?))
ON CONFLICT (author_user_id, channel_id, subscriber_ref)
DO UPDATE SET subscriber_ref = excluded.subscriber_ref
RETURNING subscription_id
```

`DO UPDATE SET subscriber_ref = excluded.subscriber_ref` is a deliberate no-op
write: it sets the column to the value it already holds, which is what forces
`RETURNING` to emit the conflicting row. `DO NOTHING` returns nothing on
conflict, which is precisely why the second `SELECT` existed.

Semantics are preserved with **no observable side effect**. `status_id` is not
in the `SET` list, so an existing subscription keeps its status — the outcome
`DO NOTHING` gave. Neither dialect's table has a trigger or an `updated_at`, and
`created_at` is untouched. The write is not literally a no-op at the storage
layer (Postgres writes a new tuple version, SQLite a page) but nothing in the
schema or API exposes that. The conflict target is valid on both dialects —
`UNIQUE (author_user_id, channel_id, subscriber_ref)` is declared in both
`0019_create_subscriptions.sql` files.

`SELECT_SUBSCRIPTION_ID` loses its only caller and is deleted from the
`SubscriptionDialect` trait and both impls.

Two engine caveats, both benign. Postgres raises `serialization_failure` (40001)
on `ON CONFLICT DO UPDATE` against a concurrently-updated row under REPEATABLE
READ or SERIALIZABLE; this codebase runs the READ COMMITTED default, which the
race analysis assumes. `RETURNING` requires SQLite ≥ 3.35, already proven
in-tree at `sqlite/posts.rs:97`, `sqlite/mod.rs:239`/`:298` and
`sqlite/feed_events.rs:64`.

_Rejected:_ a transaction around the two statements — closes the race on SQLite
but not on Postgres under READ COMMITTED, where a concurrently committed delete
is still visible to the second statement.

_Rejected:_ dropping the `SubscriptionId` return value (production discards it
at `web/src/subscriptions/api.rs:29-31`). The audience tests attach members by
subscription id, and the upsert supplies it for free.

**Risk to validate, not assume.** `storage/src/sqlite/sessions.rs:19-20` records
that SQLite's `RETURNING` with a _correlated_ subquery causes `SQLITE_BUSY`
under concurrency. The subquery here is scalar and uncorrelated. AC17 confirms
it; if `SQLITE_BUSY` appears, fall back to the transaction form for SQLite only
and record the split.

### D6 — `local_channel_id` needs no bespoke enum

Under D2 and D3 it is an ordinary instance of the general rule:
`fetch_exactly_one` with `what` naming the seeded `local` channel row, returning
`StorageError`. Its absence stays pageable — a missing schema seed is a broken
install — but the operator message now names the row. The promotion rule does
not apply: no caller branches on it.

### D7 — Record the durable principle as an ADR

The ADR states the **principle**, not the mechanism: absence is modelled inside
`storage` as an `Option` or a named `MissingRow`, and never escapes as a raw
driver error; a `RowNotFound` at the boundary is a caller defect. It records the
row-access boundary from D4 (connection paths are exempt), the promotion rule
from D2, and that `fetch_one` is reachable only through one audited wrapper.

It cites **ADR-0011** (class `Bug` drives ERROR and the `jaunder.errors` pager,
which is what makes misclassification cost something), **ADR-0017** (§1 absence
is a discrete modelled state; §3 typed sources; the remove-footguns driver),
**ADR-0059** (:150, `host` classifies raw `sqlx::Error` — a grant this spec
hands back by moving classification into `storage`; :146-154, the layering
floor), and **ADR-0016** (:221, corrected: `disallowed-methods` was hypothetical
there and is _established_ by this cycle's spike, not inherited from it).

ADR-0059:138 says the boundary **field set** is unchanged; that still holds,
since no `ErrorKind` or `ErrorClass` variant is added. Some field _values_
change — sites moving from `InternalError::storage` to `server` shift
`error.kind` from `Storage` to `Internal` and the masked 500 body from
`"storage operation failed"` to `"server operation failed"`. The ADR notes the
distinction explicitly.

Authored as a numberless draft in `docs/adr/drafts/`, numbered at ship by
`cargo xtask adr promote`.

## Scope and risk

This is a **large** cycle, deliberately taken as one: 67 signatures, ~15 enums,
9 mock surfaces, 11 consumer sites in `server`/`web`, 33 `fetch_one` triage
decisions, one `From` impl deleted and one in `server/atompub` retyped, plus the
independent D5 SQL fix. An earlier draft under-estimated this by roughly
threefold; the numbers above are the corrected budget.

The plan stages it so each task is independently verifiable and orders it
`StorageError` → enum retyping → mocks and consumers → guard and triage →
**deletion last**, because until the retyping completes the deletion breaks
every call site at once.

The judgement that recurs 33 times is "can this row be absent?". Erring safe
(`fetch_exactly_one` where `fetch_optional` was right) reproduces today's
behaviour with a better message rather than introducing a new fault.

## Acceptance criteria

**The guarantee (D1, D3, D4)**

- **AC1** No public **row-access** fn, trait method, or error-variant payload in
  `storage/src` names `sqlx::Error` in its public signature. The exempt
  connection paths are `db.rs:179` (the `FromStr::Err` associated type),
  `db.rs:247,262,286`, their two backend `database_is_empty` halves in
  `{sqlite,postgres}/mod.rs`, and `postgres::resolved_postgres_options` — which
  reads a password file, not a row. Each carries a comment citing D4's
  row-access boundary. No other exemption exists.

  _(The backend halves and `resolved_postgres_options` were added to this list
  during the ship review, which found them exempt in substance but unlisted. The
  ~16 hand-written `impl From<sqlx::Error> for <Enum>` conversions are **not**
  exemptions: they are the non-chaining hop D2 requires, and a conversion is not
  a row-access signature — `RowNotFound` cannot reach one, because `fetch_one`
  is banned.)_

- **AC2** `impl From<sqlx::Error> for InternalError` no longer exists in
  `host/src/error.rs`, and no `sqlx::Error` is lifted by `?` into an
  `InternalError` anywhere.
- **AC3** The test `from_sqlx_error_matches_storage_constructor`
  (`host/src/error.rs:635`) is deleted with the impl it pins.
  `InternalError::storage` is retained — `StorageError::Db` maps through it —
  with its coverage in `constructors_set_kind_and_class` unchanged.
- **AC4** `clippy.toml` lists all six sqlx `fetch_one` definitions under
  `disallowed-methods` by their `sqlx_core::` paths, each with a message naming
  the wrapper.
- **AC5** Two separate things, split during the ship review because the original
  wording conflated them:

  **(a) The guard rejects a bare `fetch_one` — demonstrated once, against the
  final `clippy.toml`.** A real `fetch_one` was planted at
  `storage/src/subscriptions.rs:275` and
  `cargo clippy -p storage --all-targets -- -D warnings` failed with:

  ```
  error: use of a disallowed method `sqlx_core::query_scalar::QueryScalar::fetch_one`
     --> storage/src/subscriptions.rs:275:14
      = note: use fetch_optional, or storage::error::fetch_exactly_one{,_scalar}
              to name the required row (#343)
      = note: `-D clippy::disallowed-methods` implied by `-D warnings`
  ```

  The plant was then reverted. This is a one-time proof that the mechanism binds
  and that the reason string reaches the developer at the call site — it is
  deliberately **not** a recurring gate step, because re-proving it on every run
  would cost a crate compile forever to re-establish a fact that does not
  change.

  **(b) The configuration cannot decay silently — durably checked.** The
  `fetch-one-guard` xtask step runs in the gate and fails if any of the six
  paths is removed or if `allow-invalid` is set. That covers the failure mode
  nothing else does: entries edited away by a bad merge or a cleanup pass. A
  path that stops resolving after an sqlx upgrade is already self-announcing
  (`does not refer to a reachable function`, a hard error under `-D warnings`),
  which is why `allow-invalid` is refused.

  Known residual: if clippy itself stopped being applied to the workspace,
  neither mechanism would notice. That is out of scope — it would disable every
  lint in the repo, not just this one.

  _An earlier draft of this AC asked for a committed fixture that runs clippy on
  every gate. That was reduced deliberately: it buys only the residual case
  above, at the cost of a crate compile per gate run._

- **AC6** **No `#[allow]` or `#[expect]` for `clippy::disallowed_methods` exists
  anywhere in the workspace**, and no bare `fetch_one` call remains — including
  in `storage/src`, `server/tests` and the wrapper itself, which is built on
  `fetch_optional`. CONTRIBUTING.md:111-116's suppression-approval gate is
  therefore not engaged by this cycle.

**The error type (D2)**

- **AC7** `StorageError` exists in `storage/src/error.rs` with exactly
  `Db(sqlx::Error)` and `MissingRow { what: &'static str }`.
- **AC8** `From<StorageError> for InternalError` maps `Db(e)` via
  `InternalError::storage` (kind `Storage`, class `Bug`) and `MissingRow` via
  **`InternalError::server`** (kind `Internal`, class `Bug`), with `what`
  present in the operator message and the typed `StorageError` retained in the
  source chain — asserted, not assumed, since `server_message` would discard it.
  Both arms are pinned by a test **in `storage`** (ADR-0059 layering puts `host`
  below `storage`), modelled on `storage/src/posts.rs:3789-3805`.
- **AC9** All 19 `sqlx::Error` variant payloads across the 18 enums listed in
  "What was measured" carry `StorageError` instead, and the `InviteStorage` and
  `UserConfigStorage` traits are retyped with them. Enums whose only content was
  that payload are collapsed into `StorageError`. `UpdatePostError` and
  `ListByTagError` keep their domain variants and existing projections:
  `NotFound`/`Unauthorized` still reach a 404, `TagNotFound` still yields an
  empty list.
- **AC10** No fn gained a bespoke error enum unless a caller branches on it for
  different observable behaviour (D2's promotion rule). Any addition is argued
  in the PR body.
- **AC11** The 9 `automock`'d traits compile against the new error types, and
  the 11 consumer sites named in "What was measured" construct the new types.
  `server/src/atompub/mod.rs:194` is `From<StorageError> for HandlerError`.

**The triage (D3)**

- **AC12** Every `fetch_one` site in `storage/src` is `fetch_optional`,
  `fetch_exactly_one`, or kept-with-reason. Completeness is established by the
  guard itself — `cargo xtask validate` is green only when no unannotated
  `fetch_one` remains — not by matching a count in this spec.
- **AC13** `fetch_exactly_one` maps an absent row (`fetch_optional`'s `None`) to
  `MissingRow` and every driver error to `Db`; both arms covered by a test. It
  contains no `fetch_one` call, so no `RowNotFound` is constructed.
- **AC14** A backend test drives `local_channel_id` against a database whose
  seeded `local` channel row has been removed and asserts `MissingRow` naming
  that row. This executes the mapping arm in situ, not merely the wrapper's unit
  test. The plan confirms the row can be deleted given the FK from
  `subscriptions.channel_id` and SQLite's `foreign_keys` pragma state in the
  harness; if it cannot, the plan substitutes another required-row site and says
  so.

**`subscribe` (D5)**

- **AC15** `INSERT_SUBSCRIPTION` in **both** dialect files is a single statement
  ending
  `ON CONFLICT (author_user_id, channel_id, subscriber_ref) DO UPDATE SET subscriber_ref = excluded.subscriber_ref RETURNING subscription_id`,
  in each dialect's placeholder style.
- **AC16** `SELECT_SUBSCRIPTION_ID` no longer exists — removed from the
  `SubscriptionDialect` trait (`subscriptions.rs:93`) and both impls, with no
  occurrence remaining in `storage/src`. The `INSERT_SUBSCRIPTION` doc comment
  (`:87-89`) no longer says the statement "no-ops on the conflict" and documents
  that it returns the id row on both paths.
- **AC17** `subscribe_is_idempotent_and_active`
  (`server/tests/storage/mod.rs:299`) passes on both backends. Its idempotency
  evidence is the existing `assert_eq!(id1, id2)` plus `list_subscribers`
  returning exactly one row — **not** a `created_at` equality assertion, which
  would pass vacuously: SQLite's `created_at` default is
  `strftime('%Y-%m-%dT%H:%M:%SZ','now')`, second-granularity, so two
  back-to-back `subscribe` calls share a timestamp regardless of behaviour.
- **AC18** `SubscriptionStore::subscribe` issues exactly one query, verified by
  reading the function body at review time — no gate counts queries. Listed
  because it is the point of D5.

**Record (D7, and the issue)**

- **AC19** A numberless ADR draft in `docs/adr/drafts/` states the principle,
  the row-access boundary, the promotion rule and the rejected alternatives, and
  cites ADR-0011, ADR-0016, ADR-0017 and ADR-0059.
- **AC20** The ADR corrects the record on ADR-0016:221 — `disallowed-methods`
  was hypothetical there; this cycle establishes it — and states the guard's
  workspace-wide scope.
- **AC21** #343's body is amended. Its first acceptance bullet ("a benign
  `RowNotFound` … does not log at ERROR or count as a bug") is **superseded**:
  this cycle keeps such a failure at class `Bug` deliberately and makes it
  legible instead. Editing the body, not just commenting, is what stops a later
  reader holding the merged code to a criterion it intentionally does not meet.

**Gate**

- **AC22** `cargo xtask validate` is green, including the full
  `{sqlite,postgres}×{chromium,firefox}` e2e matrix.

## Verification

- Backend parity is mandatory (ADR-0019): D5's SQL lands in both dialect files
  together, and `server/tests/storage` runs every case under
  `#[apply(backends)]` against SQLite and PostgreSQL.
- Tight loop: `cargo nextest run -p storage`, then `cargo xtask check`.
- The signature churn is compiler-verified by construction; risk concentrates in
  the 33 triage judgements, and the guard is what makes their completeness
  checkable.
- Full gate before the PR: `cargo xtask validate`.

## Out of scope

- Any change to `ErrorKind`, `ErrorClass`, their variants, or the log-level
  mapping. `Transient` remains unused.
- The `project` function in `web/src/error/server.rs`. Some masked-500 bodies
  and `error.kind` values change as D7 notes; the projection logic does not.
- `InternalError::storage`, `::server` and `::server_message`, all retained
  unchanged.
- Migrating `set_post_tags`'s insert-then-select to an atomic upsert. It is the
  same shape as D5 but unreachable today, and the D3 triage will give it a named
  `MissingRow` rather than a silent one, which is sufficient here.
