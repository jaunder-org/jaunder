# ADR-0021: SQLite dialect transaction discipline: avoid read-then-write deferred transactions

- Status: accepted
- Date: 2026-06-26
- Deciders: Michael Alan Dorman

## Context

The Nix-VM e2e suite intermittently failed with SQLite `database is locked`
(issue #18). The cause was the _shape_ of `claim_pending_batch` in the SQLite
feed-event dialect: a deferred transaction that did
`SELECT … ; UPDATE … ; SELECT …`. sqlx's `pool.begin()` issues a bare `BEGIN`,
which in SQLite is _deferred_ — it acquires no lock. The opening `SELECT` takes
a shared (read) lock; the `UPDATE` must then upgrade it to a reserved (write)
lock. In WAL mode, if another connection commits a write between the `SELECT`
and the `UPDATE`, the upgrade is against a stale snapshot and SQLite returns
`SQLITE_BUSY` — and the `busy_timeout` cannot rescue it, because retrying does
not refresh the transaction's snapshot. Under concurrent load (the always-on
feed worker plus live request handlers writing the single shared db) this
surfaced as the flake.

Postgres is immune: MVCC plus `FOR UPDATE SKIP LOCKED` lets it express the same
claim as one atomic `UPDATE … RETURNING` statement.

## Decision

In SQLite dialect code, prefer a **single autocommit statement** over a
read-then-write deferred transaction:

- Express "read to validate, then write" as one
  `UPDATE/INSERT/DELETE … WHERE <validation> RETURNING …` (or a write driven by
  a subquery). The write takes the reserved lock immediately; there is no
  shared→reserved upgrade, and `busy_timeout` applies cleanly to the one
  statement. A failure-path read (zero rows affected → `SELECT` to disambiguate
  the error) is fine: it runs outside any write and holds only a read lock.
- When multiple interdependent statements are genuinely required, take the write
  lock up front with `BEGIN IMMEDIATE` rather than a deferred `BEGIN`.
- Read-only transactions and write-first / write-only transactions are
  unaffected — they never perform a shared→reserved upgrade.

This codifies an already-established convention: `confirm_password_reset`,
`use_email_verification`, and the session `touch_and_load` (whose comment notes
it was restructured update-then-select specifically to avoid `SQLITE_BUSY`)
already follow it.

## Consequences

- `claim_pending_batch` and `use_invite` are collapsed to single-statement
  claims (issue #18). The classification rule here is what their audit used.
- An audit of every explicit-transaction SQLite path against this rule found
  three remaining read-then-write sites too substantial for a mechanical
  collapse (`create_user_with_invite`, `update_post`, `tag_post`); they are
  tracked as follow-up issues rather than reshaped here.
- Builds on ADR-0001 (storage backends) and ADR-0019 (generic store + per-trait
  dialect): this is transaction _discipline within_ a SQLite dialect, distinct
  from ADR-0019's structural mechanism.
