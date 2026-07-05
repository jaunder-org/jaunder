# ADR-0053: Storage test homing and the dual-backend presumption

- Status: proposed
- Date: 2026-07-05
- Deciders: Michael Alan Dorman
- Issue: [#135](https://github.com/jaunder-org/jaunder/issues/135)

## Context

Milestone #5's charter is that _every test asserting backend-common behavior
runs on both SQLite and Postgres._ ADR-0019 deduped the storage backends into a
generic `XStore<DB>` per trait, with each backend's divergence isolated in an
`XDialect` impl under `storage/src/{sqlite,postgres}/*.rs`. #126 then converted
the backend-common tests in the _root_ storage files to run on both backends via
`#[apply(backends)]`, and #170 / PR #242 built the backend-generic
fault-injection harness (`CloseablePool` + `TestBase::close_pool`) that makes a
single fault reproducible on either backend.

What remained were the per-backend **dialect-dir** tests. Most of them assert
backend-common behavior but run on SQLite only — Postgres coverage gaps left
over from before the #242 harness existed. Two questions had no written answer:
(1) _where_ should a storage test live, and (2) _when_ is a single-backend test
legitimate rather than an accidental coverage hole.

An older belief also lingered: that **dialect files carry no in-file tests, for
coverage reasons** — rooted in a two-pass coverage model in which only
`server/tests` got Postgres instrumentation. That model no longer exists. The
coverage gate is now a single workspace-wide instrumented nextest pass with an
ephemeral Postgres live for the whole run (`CONTRIBUTING.md` ~L428); an
`#[apply(backends)]` test gets both backends instrumented **wherever it lives**.
Test placement is therefore coverage-neutral, and that stale belief is
superseded.

## Decision

**1. Home a test by what it proves, not which backend runs it.** A
backend-common test — one written `#[apply(backends)]` — proves the _generic_
contract of `XStore<DB>`, so it lives in the generic home module
`storage/src/<trait>.rs` (per #126), beside the store it exercises. A dialect
file (`storage/src/sqlite/media.rs`) holds one backend's _divergent_ impl; a
dual-backend test in a dialect file is self-contradictory. A **decisively
backend-specific** test — one whose subject is backend-exclusive
syntax/feature/introspection with no generic home — lives with its dialect code,
because that code _is_ what it proves. A dialect file's `#[cfg(test)]` block is
deleted once no decisive-keep test remains in it.

**2. Presume a coverage gap.** A single-backend storage test is _presumed_ a
Postgres coverage gap and is converted to run on both backends via the
fault/seed harness — reach the store handle through `AppState`, inject the fault
via `CloseablePool::close`, and seed data through the agnostic
`CloseablePool::execute` (both in `storage::test_support`). A test is kept
single-backend **only** with a decisive, backend-exclusive reason: syntax or a
feature only one backend supports (e.g. Postgres `CREATE ROLE`/`CREATE DATABASE`
DDL, SQLite `PRAGMA`/`sqlite_master` introspection, a harness type-guard tied to
one pool variant). "Looks single-backend," "error path," "lazy/closed pool," or
"the seed SQL is written in one dialect" are **not** decisive — the behavior is
agnostic; provide per-backend injection SQL and convert. When in doubt, convert.

**3. Classify pure-logic by generalizability.** A helper duplicated across
dialect files with an identical body (e.g. `quote_identifier`, `quote_literal`,
`parse_status`) is deduped into a shared module and tested **once** as a plain
`#[test]`. Truly dialect-specific pure-logic (e.g. PG `CAST`/`OVERRIDING` SQL,
`json_select`, SQLSTATE matching) stays in place, exempt from the backend guard.

**4. Supersede the "no in-file dialect tests for coverage reasons" belief.**
Placement is coverage-neutral under the single workspace-wide PG-live pass, so
in-file dialect tests are permitted; homing is an organizational choice governed
by decisions 1–3, not a coverage one.

### Backup carve-out (deferred to #136)

Backup/restore is a cross-backend **contract** — a backup is a _portable dump_ —
not a per-backend behavior. So it must be tested through the public
backup/restore interface, at the contract level: round-trip fidelity **per
backend**, cross-backend portability **both directions** (both hops already
exist in `server/tests/misc/backup_interop.rs`), and a new **double round-trip
A→B→A** fidelity test. That reconception is **out of scope for #135 and deferred
to #136.** #135 left the backup tests marked `#[apply(sqlite_only)]` as an
interim measure pointing at #136; those markers are _not_ a claim that the tests
are truly backend-exclusive — they are a placeholder until #136 reworks them at
the contract level.

## Consequences

- After #135, no `#[apply(backends)]` test lives in any
  `storage/src/{sqlite,postgres}/*.rs` dialect file; converted tests sit in the
  generic home module beside the `XStore<DB>` they prove, and Postgres parity
  holes closed by the presumption are eliminated (coverage rises, never falls).
- Every `#[tokio::test]` under `storage/src/**` carries a backend template or a
  `// guard:no-backend` marker, machine-verified by the widened
  `test-backend-pattern` guard; a bare dialect `#[tokio::test]` now fails the
  gate.
- Every remaining single-backend test carries a decisive backend-exclusive
  `// reason:`; a reviewer can tell an intentional keep from an accidental gap.
- Deduped pure-logic helpers have one definition and one test each; a future
  edit changes behavior in exactly one place.
- The backup tests remain single-backend on purpose, with their interim markers
  pointing at #136; this ADR does **not** settle backup test placement — #136
  reconceives it at the contract level (per-backend round-trip, both-direction
  portability, and A→B→A double round-trip).

## References

- ADR-0019 — generic storage backend via `Backend` marker + per-trait `Dialect`.
- #126 — root-file dual-backend contract tests and the `#[apply(backends)]`
  home-module pattern.
- #170 / PR #242 — the backend-generic fault-injection harness.
- #136 — reconceive backup testing at the contract level (the backup carve-out
  above).
