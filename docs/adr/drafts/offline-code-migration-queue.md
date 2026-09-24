# ADR-DRAFT: SQLx-triggered offline Rust migrations

- Status: proposed
- Date: 2026-09-24
- Issue: [#1656](https://github.com/jaunder-org/jaunder/issues/1656)

## Context

SQLx migrations can alter schemas but cannot safely recompute derived Post HTML,
Media references, and public feed projections with Jaunder's Rust rendering
logic. An unconditional open-time Media repair rechecks old data on every open,
and a one-shot Rust migration tied to an in-memory startup cannot survive a
crash between the SQL migration and the repair. Backup and restore must carry
pending work across SQLite and PostgreSQL without pretending it was completed.

[ADR-0092](../0092-sqlite-bounded-write-lock-occupancy.md) ordinarily prohibits
unbounded rendering or loops inside a SQLite write transaction. That
request-time rule would prevent a single atomic rebuild of all current Posts.

## Decision

SQLx remains the schema authority. A SQLx migration may enqueue a named Rust
operation in `pending_code_migrations`; the table is portable application data,
not a record of successful work. After SQLx migrations, the database opener
consumes rows in monotonic queue-ID order. For each row, it opens one
transaction, runs a closed, shared Rust dispatcher for either backend, deletes
the row in the same transaction, and commits. Failure leaves the row and all its
work pending, and prevents handing the database to a server or ordinary CLI
operation. A later SQLx migration may enqueue an existing operation name again;
an unknown name fails closed. Do not derive queue state from SQLx's private
history table.

The command boundary owns the storage-directory `database.lock` throughout SQLx
migration and queue drain, including for a remote PostgreSQL database. The
server owns `runtime.lock` for its lifetime; a CLI refuses queued offline work
when that lock indicates a live same-directory server, but can open for ordinary
work when the queue is empty. Backup snapshots join the database lock; restore
holds it from emptiness preflight through database, Theme, and Media placement
and refuses a live same-directory server.

**Narrow exception to ADR-0092:** only offline queue-row transactions may render
and update an unbounded set of current Posts while holding SQLite's write lock.
No requests or workers run concurrently under this directory's server lock, and
the transaction must atomically commit or roll back all rendering, Media, feed
effects, and queue deletion. Ordinary writes retain ADR-0092's bounded occupancy
rule and ADR-0021's write-first discipline.

## Consequences

An interrupted upgrade or exact-schema restore resumes pending operations at the
next open; no separate completed-work ledger needs reconciliation. Backups
retain queued rows as data and do not consume them during import. Offline
rebuilds can be expensive and delay startup; cross-directory PostgreSQL access
and older binaries are outside this lock's coordination contract. New operations
need a companion enqueue migration and a handler with backend-parity and
rollback/retry tests; changing a handler's meaning requires considering pending
rows in existing backups. The current-Post rebuild preserves authored content,
semantic edit times, and historical Post Revisions.
