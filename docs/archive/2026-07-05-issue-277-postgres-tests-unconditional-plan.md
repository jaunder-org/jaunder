# Issue #277 — Postgres tests unconditional Implementation Plan

> **For agentic workers:** Execute this plan with **jaunder-iterate**
> (delegating via **jaunder-dispatch** if useful). Steps use checkbox (`- [ ]`)
> syntax.

**Goal:** Remove the four `postgres_testing_enabled()` early-return skips and
delete the now-unused helper, so all Postgres integration tests run
unconditionally (PostgreSQL is presumed present).

**Architecture:** Pure deletion. The helper's env check gates nothing real (PG
is always provisioned by the gate; connection URLs come from
`postgres_url()`/`postgres_bootstrap_url()` independently). Removing the skips +
the function + its re-export + two imports leaves the tests running their
assertions unconditionally.

**Tech Stack:** Rust, `cargo xtask` gate.

**Spec:**
`docs/superpowers/specs/2026-07-05-issue-277-postgres-tests-unconditional.md`.

## Review header

- **Goal:** Make the 4 straggler Postgres tests unconditional; delete
  `postgres_testing_enabled()`.
- **Scope — in:** remove 4 `if !postgres_testing_enabled() { return; }` blocks;
  delete the fn (`test_support.rs`) + its re-export (`helpers/mod.rs`) + 2 `use`
  imports.
- **Scope — out:** CONTRIBUTING.md:392 stale-teardown line → **#281**; no
  live-doc changes (CONTRIBUTING/ADR-0033 already correct); `#[apply(...)]`
  backend-case selection untouched.
- **Tasks:** 1 — remove skips + delete helper (one atomic commit).
- **Key risks/decisions:** Deleting the fn requires all callers gone in the same
  commit or the workspace won't compile — hence one task. No red→green: this is
  pure cleanup; the tests already pass (the skip never fired under the gate,
  where PG is present) and continue to. Verification is a green gate +
  `rg -t rust` returning nothing.

## Global Constraints

- **Commit trailer:** No `Co-Authored-By` (jaunder + user preference).
- **Gate:** run `devtool run -- cargo xtask check` green before committing
  (**jaunder-commit**); the pre-commit hook re-runs it.
- **No suppression:** if a diagnostic appears (e.g. unused import), fix it by
  removal, not `#[allow]`.
- **Crate names:** `storage`, `jaunder` (server tests).

---

### Task 1: Remove the `postgres_testing_enabled()` gating

**Files:**

- Modify: `storage/src/test_support.rs:275-279` — delete the fn + its doc
  comment.
- Modify: `server/tests/helpers/mod.rs:15-20` — drop `postgres_testing_enabled`
  from the re-export union.
- Modify: `server/tests/misc/backup_interop.rs` — import (`:25`) + 2
  early-returns (`:144-146`, `:181-183`).
- Modify: `server/tests/misc/pg_teardown.rs` — import (`:1-3`) + 2 early-returns
  (`:46-48`, `:72-74`).

**Interfaces:**

- Consumes: nothing new.
- Produces: `postgres_testing_enabled` no longer exists (removes it from the
  public test-support surface).

- [x] **Step 1: Delete the four early-return skips.**

In each of the four test bodies, delete the block (leaving the preceding
`let _ = backend;` and the following body intact):

```rust
    if !postgres_testing_enabled() {
        return;
    }
```

Sites: `backup_interop.rs` `sqlite_backup_restores_into_postgres` (~:144) and
`postgres_backup_restores_into_sqlite` (~:181); `pg_teardown.rs`
`per_test_database_is_dropped_on_teardown` (~:46) and
`unique_postgres_database_is_dropped_on_guard_drop` (~:72).

- [x] **Step 2: Delete the `postgres_testing_enabled()` definition.**

In `storage/src/test_support.rs`, remove the doc comment + function (lines
275-279):

```rust
/// Whether Postgres-backed tests are enabled (i.e. `JAUNDER_PG_TEST_URL` is set).
#[must_use]
pub fn postgres_testing_enabled() -> bool {
    std::env::var("JAUNDER_PG_TEST_URL").is_ok()
}
```

- [x] **Step 3: Remove the re-export and the two imports.**

- `server/tests/helpers/mod.rs` — remove `postgres_testing_enabled,` from the
  `pub use storage::test_support::{…}` union (currently on the line
  `postgres_only, postgres_test_authority, postgres_testing_enabled, recorded_postgres_url,`).
- `server/tests/misc/backup_interop.rs` — remove `postgres_testing_enabled,`
  from
  `use crate::helpers::{ postgres_only, postgres_testing_enabled, unique_postgres_url, Backend, PostgresDbGuard, };`.
- `server/tests/misc/pg_teardown.rs` — remove `postgres_testing_enabled,` from
  `use crate::helpers::{ postgres_bootstrap_url, postgres_only, postgres_testing_enabled, recorded_postgres_url, unique_postgres_url, Backend, };`.

- [x] **Step 4: Run the gate — verify PASS and no residue.**

Run: `devtool run -- cargo xtask check` Expected: PASS — compiles clean (no
unused-import / dead-code warning under `-D warnings`); the four
now-unconditional Postgres tests execute and pass under the ephemeral cluster;
coverage clean.

Then confirm no Rust-source residue:

Run: `rg -t rust 'postgres_testing_enabled'` Expected: no output (exit 1 /
empty). Archived docs and the spec retain the string — that's fine.

- [x] **Step 5: Commit.**

```bash
git add storage/src/test_support.rs server/tests/helpers/mod.rs server/tests/misc/backup_interop.rs server/tests/misc/pg_teardown.rs
git commit -m "test(pg): run Postgres tests unconditionally; drop postgres_testing_enabled() (#277)"
```

(Pre-commit hook re-runs `cargo xtask check`; must be green from Step 4. **No
`Co-Authored-By` trailer.**)

---

## Self-review

- **Spec coverage:** Decision 1 (remove skips) → Step 1; Decision 2 (delete fn +
  re-export + imports) → Steps 2-3; Decision 3 (no live-doc changes) → nothing
  to do. AC1 (`rg -t rust` empty) → Step 4; AC2 (no env-skip in the 4 tests) →
  Step 1; AC3 (no dead-code/unused-import) → Step 4 gate; AC4 (`validate` green)
  → Step 4 (`check` per-task; `validate` at ship). #281 kept out.
- **Placeholder scan:** none.
- **Type consistency:** single symbol `postgres_testing_enabled` removed
  everywhere; no cross-task signatures.
