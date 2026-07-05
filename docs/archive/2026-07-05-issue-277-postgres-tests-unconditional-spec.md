# Spec — issue #277: make Postgres tests unconditional; remove `postgres_testing_enabled()`

**Issue:** jaunder-org/jaunder#277 (milestone 1 "Verify-gate hardening";
`tooling`, `test-infra`).

## Premise

PostgreSQL is **presumed present** in every test environment. The integration
suite is only ever run through the gate, which provisions an ephemeral cluster;
there is no supported no-PG run. This is already the documented contract —
`CONTRIBUTING.md` (lines 366–368): _"a bare `cargo nextest run` **requires a
reachable PostgreSQL**: the postgres cases connect to `JAUNDER_PG_TEST_URL` …
and fail if nothing is listening."_ Prior work moved the bulk of the suite to
this model — #54 and #127 converted ~152 tests to unconditional
`#[apply(backends)]`, explicitly _"drop the old `postgres_testing_enabled()`
env-branch; no fallback/skip logic."_

## Problem

Four `postgres_testing_enabled()` early-return skips remain — stragglers
#54/#127 did not touch (they are `postgres_only` teardown/interop tests, not
`backends` behaviour tests). They frame Postgres as optional, contradicting the
premise:

- `server/tests/misc/backup_interop.rs:144, :181` —
  `if !postgres_testing_enabled() { return; }`
- `server/tests/misc/pg_teardown.rs:46, :72` — same, in
  `per_test_database_is_dropped_on_teardown` and
  `unique_postgres_database_is_dropped_on_guard_drop` (the latter added by #43).

The earlier framing of #277 had the fix backward — it proposed making
`commands.rs` (which correctly runs its Postgres cases unconditionally)
_conditional_. The correct direction is the reverse: make everything else match
`commands.rs`.

`postgres_testing_enabled()` (`storage/src/test_support.rs:277`,
`std::env::var("JAUNDER_PG_TEST_URL").is_ok()`) has **no other callers**
(audited `rg 'postgres_testing_enabled'`: the 4 sites above, its definition, and
the re-export in `server/tests/helpers/mod.rs:17`). It is not read by xtask, the
harness, or any URL-resolution path — the connection URL comes from
`postgres_url()` / `postgres_bootstrap_url()` independently.

## Resolved design

1. **Remove the four early-return skips.** Delete each
   `if !postgres_testing_enabled() { return; }` block. The tests then run their
   assertions unconditionally; the surrounding `let _ = backend;` and bodies are
   otherwise unchanged.
2. **Delete `postgres_testing_enabled()`** — its definition
   (`storage/src/test_support.rs:277`) and its re-export
   (`server/tests/helpers/mod.rs:17`) — since it is now unused. Remove
   `postgres_testing_enabled` from the two test files' `use crate::helpers::{…}`
   imports.
3. **No live-doc changes.** `CONTRIBUTING.md` and `docs/adr/0033` already
   describe Postgres as required/available, not conditionally skipped — they are
   consistent with this change.

**Behavioural effect:** if PostgreSQL is genuinely absent, these tests now fail
at connection (a hard error) instead of silently passing via early return —
which is the correct enforcement of the "PG is always present" premise and
matches the existing `#[apply(backends)]` tests' behaviour.

## Out of scope (recorded)

`CONTRIBUTING.md:392` ("Per-test databases are not dropped after each run …") is
now stale after #28/#43 gave per-test databases teardown guards. It is a
consequence of #43, unrelated to the `postgres_testing_enabled()` gating, and is
**not** changed here — filed separately as **#281**.

## Acceptance criteria (observable)

1. No `postgres_testing_enabled` reference remains in Rust source
   (`rg -t rust 'postgres_testing_enabled'` returns nothing): the function
   definition, its re-export, the two `use` imports, and all four call sites are
   gone. (Archived docs under `docs/archive/**` and this spec retain the string
   for history — not in scope.)
2. `backup_interop.rs`'s two tests and `pg_teardown.rs`'s two tests contain no
   `if !… { return; }` env-skip; they execute their assertions unconditionally.
3. No unused-import or dead-code warning results (the two imports are cleaned;
   `cargo xtask check` clippy step is clean under `-D warnings`).
4. `cargo xtask validate` is green — under the ephemeral cluster the
   now-unconditional Postgres assertions actually execute and pass.
