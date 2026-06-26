# Issue #18 — SQLite `claim_pending_batch` lock flake + explicit-transaction audit

- Issue: [#18](https://github.com/jaunder-org/jaunder/issues/18) — *e2e: sqlite 'database is locked' flakiness under parallel Playwright load*
- Milestone: 1 — Verify-gate hardening
- Date: 2026-06-26
- Status: approved (design)

## Problem

The Nix-VM e2e suite intermittently fails with SQLite `database is locked`
(`posts.spec.ts` `create_post` returns not-ok; the feed worker logs the same).
A single re-run passes, so it is load-timing contention, not a logic bug.

The issue speculated three mitigations (raise `busy_timeout`, throttle the feed
worker during tests, cap Playwright workers). None is the real cause. The Nix VM
already runs Playwright with `workers: 1` (`flake.nix:445`), so the contention is
**not** parallel browser workers — it is the feed worker (`server/src/feed/worker.rs`,
every 10s) plus the live server's request handlers all writing the single shared
`/var/lib/jaunder/data/jaunder.db`, against the 5s `busy_timeout`
(`storage/src/sqlite/mod.rs:102`, WAL mode at `:101`).

The root cause is the **shape** of `claim_pending_batch` in the SQLite dialect
(`storage/src/sqlite/feed_events.rs:27-94`):

```
BEGIN (deferred)          -- sqlx pool.begin(): acquires no lock
  SELECT id ...           -- acquires a SHARED read lock
  UPDATE ... SET claimed  -- must UPGRADE shared -> reserved (write) lock
  SELECT ... (re-read)
COMMIT
```

In WAL mode a deferred transaction that **reads then writes** must upgrade its
lock against a possibly-stale snapshot. If another writer commits between the
`SELECT` and the `UPDATE`, SQLite returns `SQLITE_BUSY` on the upgrade, and
`busy_timeout` cannot reliably rescue it (retrying does not refresh the snapshot).
Postgres is immune: it does the whole claim as one atomic
`WITH eligible AS (… FOR UPDATE SKIP LOCKED) UPDATE … RETURNING` statement
(`storage/src/postgres/feed_events.rs:22-40`).

## Scope

Three deliverables, one branch / one merge against #18:

1. **Phase 1 — fix `claim_pending_batch`** (the #18 root cause).
2. **ADR-0021** — document the anti-pattern and its remediation as a durable,
   citable decision.
3. **Phase 2 — audit** every explicit-transaction SQLite path for the same
   failure mode and remediate per triage.

**Out of scope:** `busy_timeout` changes, feed-worker throttling, Playwright
worker caps, and relocating the pre-existing in-file dialect tests (ADR-0019
cleanup is a separate concern).

---

## Phase 1 — collapse `claim_pending_batch`

Replace the `BEGIN → SELECT → UPDATE → SELECT → COMMIT` transaction in
`storage/src/sqlite/feed_events.rs` with a single autocommit statement, mirroring
the Postgres shape (SQLite ≥ 3.35 supports `UPDATE … RETURNING`):

```sql
UPDATE feed_events SET status = 'claimed', claimed_at = ?
WHERE id IN (
    SELECT id FROM feed_events
    WHERE (status = 'pending' AND next_attempt_at <= ?)
       OR (status = 'claimed' AND claimed_at < ?)
    ORDER BY next_attempt_at ASC
    LIMIT ?
)
RETURNING id, feed_url, status, attempts, last_error, next_attempt_at,
          claimed_at, created_at, regenerated_at, pinged_at
```

One statement → SQLite takes the write lock immediately (no deferred read-then-
upgrade), so `busy_timeout` applies cleanly to the single statement and the
BUSY-on-upgrade trap disappears. Removes the `tx`, the empty-`ids` early return,
and the dynamic `placeholders()` use in this method. The `placeholders()` helper
stays — `mark_*` still use it.

### Behavior parity (must hold)

- `RETURNING` yields post-`UPDATE` values (`status='claimed'`, `claimed_at=now`),
  identical to what the old trailing `SELECT` returned.
- Eligibility predicate and `ORDER BY next_attempt_at ASC LIMIT` preserved inside
  the subquery.
- Empty case: 0 rows updated → empty `RETURNING` → `Ok(vec![])`, same as the old
  early return.
- Result ordering: the old final `SELECT … WHERE id IN (…)` had no `ORDER BY`
  either, so unordered-result parity holds.
- The existing in-file unit tests
  (`claim_returns_eligible_pending_row`, `double_claim_returns_no_rows_within_lease`,
  `lease_expired_rows_are_reclaimable`, `mark_pinged_marks_done_and_removes_from_queue`,
  …) must all still pass unchanged.

### Implementation notes

- Confirm (do not assume) the bundled SQLite is ≥ 3.35 for `RETURNING`. sqlx's
  bundled libsqlite3 is well past this.
- Parameter binding: either bind `now` twice (plain `?`) or use SQLite numbered
  params (`?1` reused) so `now` is bound once — an implementation detail, decided
  in the plan.

### Verification (kept, but `#[ignore]`d)

You cannot prove a negative, so a timing-based concurrency test must not run in CI
(it would itself be a flake source — ironic, here). But it is worth keeping as an
on-demand reproduction/regression tool rather than throwing away.

- Add a test that opens an **on-disk WAL** SQLite database (a tempfile —
  `:memory:` does not reproduce file locking) and runs N concurrent tasks
  hammering `claim_pending_batch` alongside competing writes, asserting no
  `database is locked` error.
- Gate during development: it must be reliably **red on the current code** (proving
  the bug is reproduced) and **green after the collapse**, within a bounded run.
- **Keep it**, marked `#[ignore]`, with a comment explaining what it is: a manual
  reproduction of the #18 lock-upgrade flake, excluded from CI because it is
  timing-based; run on demand with `cargo test … -- --ignored <name>`.
- **Placement — out-of-file, not in the dialect source.** Put it as an integration
  test under `server/tests/feed/` (alongside the existing `feed_events`/
  `feed_worker` tests), calling the public `claim_pending_batch` API. An `#[ignore]`d
  test placed *inside* an instrumented dialect file (`storage/src/sqlite/*.rs`) runs
  in neither coverage pass — the SQLite pass skips `#[ignore]`, the PG pass is
  `-p jaunder` only — so its instrumented-but-unexecuted lines tank that file's
  per-file coverage (the mechanism behind a prior false 82%→16% in `posts.rs`). As a
  separate test target it cannot distort dialect-file coverage.
- Parity stays covered by the existing `claim_pending_batch` unit tests, which run
  in CI normally.

---

## ADR-0021 — SQLite dialect transaction discipline

A new ADR (next number after 0020), following the house format
(Status / Date / Deciders / Context / Decision / Consequences). It is a new ADR,
**not** an amendment to ADR-0019: ADR-0019 documents the *structural* generic-store
+ dialect mechanism; this is *transaction discipline within a SQLite dialect* — a
separable, citable rule.

- **Context:** SQLite deferred `BEGIN` + read-then-write upgrades a shared lock to
  reserved against a possibly-stale WAL snapshot → `SQLITE_BUSY`-on-upgrade that
  `busy_timeout` cannot reliably rescue under concurrency (the #18 flake). Postgres
  is unaffected (MVCC + `FOR UPDATE SKIP LOCKED`).
- **Decision:** In SQLite dialect code, prefer a **single autocommit statement**
  (`UPDATE/INSERT/DELETE … RETURNING`, or a write driven by a subquery) over a
  read-then-write deferred transaction. When multiple interdependent statements are
  genuinely required, take the write lock up front (`BEGIN IMMEDIATE`) rather than a
  deferred `BEGIN`. Read-only and write-first transactions are fine.
- **Consequences:** the rule the Phase 2 audit classifies against; cites ADR-0001
  (storage backends) and ADR-0019 (dialect layer); may trigger a one-line wording
  fix in ADR-0019 (see Phase 2).

---

## Phase 2 — audit SQLite explicit transactions

Find every explicit-transaction SQLite code path subject to the same failure mode
and remediate per triage, classifying against ADR-0021.

### Vulnerable pattern (classification criterion)

A *deferred* transaction (sqlx `pool.begin()` issues `BEGIN`, acquiring no lock)
that **reads, then writes** — the write forces a shared→reserved lock upgrade
against a possibly-stale snapshot. Transactions whose **first** statement is a
write (immediate reserved lock, no upgrade), and read-only transactions, are
**not** vulnerable. Classify each site: *vulnerable* (read-then-write) or *safe*
(write-first / write-only / read-only).

### Audit surface

SQLite dialect sites (`storage/src/sqlite/`):

- `feed_events.rs:33` — Phase 1 (fixed).
- `mod.rs:157`, `mod.rs:225`
- `posts.rs:31`, `posts.rs:106`
- `sessions.rs:19` — this is ADR-0019's named proof-of-concept
  (`SessionStorage::authenticate`, "SQLite explicit tx vs Postgres data-modifying
  CTE"). If it is vulnerable and we collapse it, update ADR-0019's wording.

Backend-agnostic sites (generic over the pool; execute against SQLite when that
backend is active — verify each truly runs on the SQLite pool):

- `posts.rs:538`, `audiences.rs:181`, `email.rs:98`, `invites.rs:104`

### Remediation triage

- **Trivial → fix in place.** A clean one-to-one analog of `claim_pending_batch`
  (a read-select feeding a single write) that collapses to one
  `… RETURNING` / `INSERT … RETURNING` / single statement with a subquery. Lands
  in this branch.
- **Substantial → file a follow-up issue.** Anything with multiple interdependent
  writes, semantic risk, or that needs a `BEGIN IMMEDIATE`-style remediation rather
  than a collapse. Filed via the jaunder-issues skill, milestone 1; not expanded
  into this cycle.

### Deliverable

A findings table (in the plan and the PR body): each site, its classification
(vulnerable / safe), and its disposition (fixed-here / issue-#NNN / no-action) —
so the audit's completeness is visible even where no code changed.

---

## Success criteria

- `claim_pending_batch` is a single autocommit statement; existing unit tests pass;
  the concurrency test was red-before / green-after and is kept under
  `server/tests/feed/`, `#[ignore]`d with an explanatory comment.
- ADR-0021 exists and is referenced by the Phase 2 dispositions.
- Every audit-surface site is classified and dispositioned in the findings table;
  every *trivial* vulnerable site is fixed here; every *substantial* one has a filed
  issue.
- `cargo xtask validate` is green (full CI-faithful gate, incl. e2e sqlite+postgres).
