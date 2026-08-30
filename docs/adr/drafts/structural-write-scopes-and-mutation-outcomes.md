# ADR-DRAFT: Structural Write Scopes and Mutation Outcomes

- Status: proposed
- Date: 2026-08-30
- Issue: [#363](https://github.com/jaunder-org/jaunder/issues/363)

## Context

Application storage mutations must sometimes compose across storage traits, but
a pool-backed mutation or a storage-owned transaction makes that composition
non-atomic. The former leaves a later operation unable to roll back an earlier
one; the latter makes the transaction boundary an implementation detail of one
storage handle rather than an explicit caller-owned boundary.

The boundary must preserve each backend's existing concurrency discipline.
SQLite must use `BEGIN IMMEDIATE` for a write scope so it does not attempt a
deferred read-to-write upgrade. PostgreSQL must retain the row locks required by
the operation, including the post-row and slug-ordered tag locks used by
post-tag reconciliation. An unfinished scope must not return an open transaction
or a SQLite write lock to the pool.

A callback failure can be reported as an operation failure because the scope did
not commit. A commit acknowledgement failure is different: a PostgreSQL
connection can fail after the server has committed but before that fact reaches
the client. Treating it as rollback-confirmed would tell callers a result that
the database may contradict.

## Decision

The runtime-selected storage backend factory mints a concrete, backend-erased
`WriteScope` beside the exact object-safe storage trait handles it supplies to
server context and workers. `WriteScope` is factory-owned: downstream code can
enter a scope, but cannot construct one, choose a backend, look up storage from
it, or execute arbitrary SQL through it. `WriteScope::run` is the explicit,
smallest-coherent-set commit boundary.

`WriteScope::run` gives its callback a sealed, backend-erased mutable write
capability. Audited application storage mutations take that capability and have
no pool-backed, auto-committing, standalone, or compatibility form. The adapter
creates the concrete transaction behind that capability; a backend mismatch is
an internal wiring error, not an application choice.

On callback `Err`, the scope rolls back or drops the transaction and reports a
rollback-confirmed operation failure. On callback `Ok`, it makes exactly one
commit attempt. A successful acknowledgement is a confirmed commit; any commit
error is commit-indeterminate, never an operation failure. The scope records
this bounded outcome on the span that owns the decision, following
[ADR-0147](../0147-decision-path-observability.md).

Server mutation outputs represent confirmed and indeterminate commits with typed
`MutationOutcome<T>` variants. Both variants cause client invalidation and
revalidation; indeterminate remains visibly error-like rather than claiming a
confirmed result.

## Consequences

Callers acquire a scope only around the mutations that must be atomic. They
prepare validation, rendering, ordinary password hashing, filesystem work, and
network work before acquisition or after completion, except for the existing
claim-before-Argon2 cases. This is necessary to preserve bounded SQLite write
lock occupancy.

The post-tag slice proves the boundary while retaining SQLite `BEGIN IMMEDIATE`,
PostgreSQL post-row and slug-ordered tag locks, snapshot ordering, and injected
error behaviour. Future vertical cutovers use the same capability; they do not
introduce a second mutation implementation or a general SQL executor.

This decision supersedes [ADR-0021](../0021-sqlite-transaction-discipline.md)'s
single-statement-autocommit preference for the audited application mutations
only. Administrative lifecycle operations, including migrations, whole-store
backup/restore, and PostgreSQL bootstrap, retain their top-level transaction or
connection ownership. It does not create a distributed transaction across the
database and filesystem, SMTP, WebSub, or any other remote system.

<!--
Shipping an ADR includes updating docs/ARCHITECTURE.md (and CONTEXT.md when
the ubiquitous language changes) in the same change — the view is the home
of current truth. Later addenda to a shipped ADR are written in past tense
("as of <date>, Y held; current state: ARCHITECTURE.md §Z"), never as
present-tense patches: an ADR is an immutable event. See
docs/adr/0127-architecture-view-materialized-from-adrs.md.
-->
