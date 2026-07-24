# Plan — #271: drop the redundant DB wipe in the e2e VM `seed_db()`

Spec:
[`2026-07-24-issue-271-e2e-vm-seed-no-wipe.md`](../specs/2026-07-24-issue-271-e2e-vm-seed-no-wipe.md)

**For agentic workers:** execute with `jaunder-iterate` (delegate a task via
`jaunder-dispatch` if useful). Tick checkboxes in real time.

---

## Review header

**Goal.** Remove the redundant pre-seed DB wipe from the two Nix VM e2e
`seed_db()` functions in `flake.nix`, leaving only the `devtool seed-e2e`
seeding call in each. On a fresh single-use VM the DB is already clean + fully
migrated by boot, so the wipe reconstructs an equivalent DB for nothing (see
spec §Equivalence).

**Scope.**

- _In:_ the SQLite `seed_db()` (`flake.nix:764`) and Postgres `seed_db()`
  (`flake.nix:898`) bodies, and their explanatory comments.
- _Out:_ everything else in `flake.nix` (boot `preStart`, `create-pg-db`, the
  Postgres delayed start, `e2eRunAndCapture`), the host `e2e-local` path, the
  Rust test-DB harness. No product code. Per spec AC3, no diff outside the two
  bodies + comments.

**Tasks.**

- [x] 1. SQLite `seed_db()` — drop stop/`rm -rf`/start + the two waits; rewrite
     comment.
- [x] 2. Postgres `seed_db()` — drop the `TRUNCATE DO $$…$$` block; rewrite
     comment.
- [x] 3. Verify — `cargo xtask validate` (all four
     `{sqlite,postgres}×{chromium,firefox}` VM e2e combos, warm cache) green
     locally; CI `e2e-gate` is the cold backstop. **Done:** validate PASSED
     (~13.8m); 108 Playwright tests passed per combo, 0 failures.

**Key risks / decisions.**

- No unit-test surface: this is Nix VM plumbing. The per-commit gate
  (`cargo xtask check`) proves the flake still evaluates/builds and host checks
  pass; the **definitive** proof is task 3's full `validate` (the e2e VM run) —
  `check` does not run the VMs.
- Documented e2e DB-state fragility history: both SQLite and Postgres browser
  runs must pass **end-to-end**, not merely boot (spec §Verification).
- Two commits (one per backend) for clean, revertable history under one issue.
- No separable follow-on concerns surfaced during the interview, so there is
  **no** issue-filing first task.

---

## Global constraints

- Edit **only** the two `seed_db()` bodies + their comments in `flake.nix`. Keep
  `create-pg-db`, the delayed `systemctl start jaunder.service`, and boot
  `preStart` exactly as-is.
- No `Co-Authored-By` trailer on commits.
- Commit via `jaunder-commit`: run `cargo xtask check` first so the pre-commit
  hook (fmt + clippy + Nix coverage/tests) passes clean. `check` builds the
  flake, so a malformed `flake.nix` fails here — but it does **not** run the e2e
  VMs.
- `flake.nix` is tracked, so edits are seen by Nix without `git add`; still
  verify `git status` before committing (a hook may auto-stage).

---

## Task 1 — SQLite `seed_db()`: remove the wipe

**Files:** `flake.nix` (SQLite VM derivation, `seed_db()` at `:764`).

**Change.** Replace the current body (comment `:765-769` + the wipe/restart at
`:770-774`) so the function's only action is the existing `devtool seed-e2e`
call, run against the boot DB while the boot service is still up.

Current:

```python
def seed_db():
  # Wipe the SQLite data dir wholesale and let jaunder's auto-init
  # recreate the schema. Avoids hardcoding a table list (which
  # would silently drift as the schema grows). The host driver
  # (`cargo xtask e2e-local`) gets the same fresh-state guarantee a
  # different way: a per-run temp storage dir + DB (#249).
  machine.succeed("systemctl stop jaunder.service")
  machine.succeed("rm -rf /var/lib/jaunder/data")
  machine.succeed("systemctl start jaunder.service")
  machine.wait_for_unit("jaunder.service", timeout=60)
  machine.wait_for_open_port(3000, timeout=30)
  machine.succeed(
    "JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture devtool seed-e2e"
    + " --db sqlite:/var/lib/jaunder/data/jaunder.db"
    + " --test-support-bin test-support"
    + " --jaunder-bin jaunder"
  )
```

After:

```python
def seed_db():
  # Seed the fresh VM's already-migrated DB. This VM is single-use and
  # jaunder.service's boot preStart (`jaunder init`) has already created and
  # migrated an empty DB (incl. migration 0018 reference data); nothing writes
  # user data before this point, so no wipe is needed (#271). Seeding runs
  # against the running boot service.
  machine.succeed(
    "JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture devtool seed-e2e"
    + " --db sqlite:/var/lib/jaunder/data/jaunder.db"
    + " --test-support-bin test-support"
    + " --jaunder-bin jaunder"
  )
```

**Check:** `cargo xtask check` — flake still evaluates/builds + host checks pass
(expected PASS). Does **not** exercise the VM.

**Commit:** `test(e2e): drop redundant SQLite DB wipe in VM seed_db (#271)` via
`jaunder-commit`.

**Done when:** SQLite `seed_db()` contains only the `devtool seed-e2e` call +
the new comment; no `systemctl`/`rm -rf`/`wait_for_*` remain in it;
`cargo xtask check` green.

---

## Task 2 — Postgres `seed_db()`: remove the TRUNCATE

**Files:** `flake.nix` (Postgres VM derivation, `seed_db()` at `:898`).

**Change.** Replace the current body (comment `:899-906` + the
`TRUNCATE DO $$…$$` block at `:907-914`) so the function's only action is the
existing `devtool seed-e2e` call. Leave `create-pg-db` (`:883-893`) and the
delayed `systemctl start jaunder.service` untouched — they are load-bearing.

Current:

```python
def seed_db():
  # Dynamic TRUNCATE of every public-schema table avoids
  # hardcoding a list that would drift as the schema grows.
  # Postgres can't be stop-wiped the way SQLite can (it's
  # a separate service), so a wipe-via-TRUNCATE is the
  # cheapest equivalent.
  # channels/subscription_statuses/target_kinds carry migration-seeded
  # reference data (migration 0018); the non-restartable Postgres path
  # can't re-seed them, so exclude them from the wipe.
  machine.succeed(
    "sudo -u postgres psql -d jaunder -c \"DO \\$\\$ DECLARE r record;"
    + " BEGIN FOR r IN SELECT tablename FROM pg_tables"
    + " WHERE schemaname = 'public' AND tablename NOT LIKE '\\\\_sqlx%'"
    + " AND tablename NOT IN ('channels', 'subscription_statuses', 'target_kinds') LOOP"
    + " EXECUTE 'TRUNCATE TABLE ' || quote_ident(r.tablename) || ' CASCADE';"
    + " END LOOP; END \\$\\$;\""
  )
  machine.succeed(
    "JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture devtool seed-e2e"
    + " --db postgres://jaunder:testpassword@127.0.0.1/jaunder"
    + " --test-support-bin test-support"
    + " --jaunder-bin jaunder"
  )
```

After:

```python
def seed_db():
  # Seed the fresh VM's already-migrated DB. This VM is single-use; create-pg-db
  # + the delayed jaunder.service boot preStart (`jaunder init`) have already
  # created and migrated an empty DB (incl. migration 0018 reference data), and
  # nothing writes user data before this point, so no TRUNCATE is needed (#271).
  machine.succeed(
    "JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture devtool seed-e2e"
    + " --db postgres://jaunder:testpassword@127.0.0.1/jaunder"
    + " --test-support-bin test-support"
    + " --jaunder-bin jaunder"
  )
```

**Check:** `cargo xtask check` (expected PASS).

**Commit:** `test(e2e): drop redundant Postgres TRUNCATE in VM seed_db (#271)`
via `jaunder-commit`.

**Done when:** Postgres `seed_db()` contains only the `devtool seed-e2e` call +
the new comment; no `psql`/`TRUNCATE` remains in it; `create-pg-db` + delayed
start unchanged; `cargo xtask check` green.

---

## Task 3 — Verify the full VM e2e matrix

**No file change.** Acceptance gate for spec AC5.

**Run:** `cargo xtask validate` (foreground, long — all four
`{sqlite,postgres}×{chromium,firefox}` VM e2e combos, warm cache). Per repo
guidance run slow gates in the foreground with a generous timeout, not
background.

**Done when:** `validate` is green — every combo's browser run passes end-to-end
(not merely boot). If any combo fails, diagnose against the spec's equivalence
argument (did seeding actually run against the migrated boot DB?) before
re-attempting. CI's `e2e-gate` on the PR provides the cold-cache confirmation
(monitored autonomously post-push).

---

## Self-review

- Every spec AC maps to a task: AC1→T1, AC2→T2, AC3→T1+T2 (bodies+comments
  only), AC4→T1+T2 (comment rewrites), AC5→T3.
- No task smuggles out-of-scope work; no separable concern to file.
- Tasks are independently checkable (T1/T2 by `cargo xtask check` + diff; T3 by
  the e2e matrix).
