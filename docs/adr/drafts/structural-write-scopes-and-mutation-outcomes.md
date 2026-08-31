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

`WriteScope::run` gives its callback a sealed, backend-erased mutable
`WriteTransaction` capability. The audited application surface is a closed
census of 48 methods: `AudienceStorage` (5), `EmailVerificationStorage` (2),
`FeedCacheStorage` (2), `FeedEventStorage` (7), `InviteStorage` (1),
`MediaStorage` (2), `PasswordResetStorage` (2), `PostStorage` (7),
`SessionStorage` (3), `SiteConfigStorage` (6), `SubscriptionStorage` (2),
`UserConfigStorage` (2), `UserStorage` (5), and `AtomicOps` (2). Every one takes
`&mut WriteTransaction`; it has no pool-backed, auto-committing, standalone,
alias, or compatibility form. The adapter creates the concrete transaction
behind that capability; a backend mismatch is an internal wiring error, not an
application choice.

The structural contract derives the observed trait census and compares it with
the closed authoritative 48-method list. It rejects an unknown, missing, or
duplicate declaration, requires the mutable capability on every audited method,
and rejects production transaction starts that bypass the
`WriteScope`/`WriteTransaction` composition. Administrative lifecycle work
(migrations, backup and restore, and PostgreSQL bootstrap), backend dialect
code, and internal helpers are outside that application-method census.

On callback `Err`, the scope rolls back or drops the transaction and returns
`WriteScopeError::Operation`: that failure is rollback-confirmed. On callback
`Ok`, it makes exactly one commit attempt: a successful acknowledgement returns
`MutationOutcome::Confirmed`, while any commit error returns
`MutationOutcome::CommitIndeterminate`, never a rollback-confirmed operation
failure. The scope records this bounded outcome on the span that owns the
decision, following [ADR-0147](../0147-decision-path-observability.md).

Server mutation outputs preserve the same outcome algebra: a rollback-confirmed
`WriteScopeError`, `MutationOutcome::Confirmed`, or
`MutationOutcome::CommitIndeterminate`. Confirmed and indeterminate results both
cause client invalidation and revalidation; indeterminate remains visibly
error-like rather than claiming a confirmed result.

## Consequences

Callers acquire a scope only for the narrow set of mutations that must be
atomic. They prepare validation, rendering, ordinary password hashing, streamed
upload bytes, filesystem work, and network work before acquisition or after
completion. Media upload and reclamation use a cross-process, per-content file
lock to serialize target placement and cleanup around their short database
identity-lock scopes; no database transaction spans filesystem I/O. Existing
claim-before-Argon2 cases remain explicit. This preserves bounded SQLite
write-lock occupancy.

The post-tag and media reconciliation paths prove the boundary while retaining
SQLite `BEGIN IMMEDIATE`, PostgreSQL post-row and slug-ordered tag locks,
snapshot ordering, and injected-error behaviour. Every audited application
mutation uses the same capability; no cutover retains a second mutation path or
a general SQL executor.

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
