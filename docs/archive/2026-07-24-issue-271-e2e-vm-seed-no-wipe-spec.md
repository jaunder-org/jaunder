# Spec — #271: drop the redundant DB wipe in the e2e VM `seed_db()`

- Issue: [#271](https://github.com/jaunder-org/jaunder/issues/271)
- Milestone: Test infrastructure & E2E
- Date: 2026-07-24

## Problem

The Nix VM e2e checks in `flake.nix` each run in their own **fresh, single-use
NixOS VM** — one per `{backend × browser}` derivation, never reused. On such a
VM the application database is already in a **known-clean, fully-migrated**
state by the time the test script's `seed_db()` runs:

- **SQLite** — `jaunder.service` starts at boot; its `preStart` runs
  `jaunder init --db "$JAUNDER_DB" --skip-if-exists` (`flake.nix:121`), creating
  `/var/lib/jaunder/data/jaunder.db` and running all migrations, including
  migration 0018's reference-data seed (`channels` / `subscription_statuses` /
  `target_kinds`).
- **Postgres** — `jaunder.service` is delayed at boot
  (`wantedBy = lib.mkForce [ ]`, `flake.nix:869`); the test script runs
  `create-pg-db` (role + database), then `systemctl start jaunder.service`,
  whose `preStart` migrates the same clean schema + reference data.

No **test step** writes user data between that migration and `seed_db()` — the
only intervening script action is `cp -r ${e2ePackage}` (the Playwright bundle),
which never touches the server. (The jaunder server is running live in this
window — it must be, so the browser can reach it — but `jaunder serve` seeds no
user rows at startup; AC5's end-to-end pass is the empirical backstop for that
assumption.) Yet each `seed_db()` performs a **redundant wipe** before seeding:

- **SQLite** (`flake.nix:770-772`): `systemctl stop jaunder.service` →
  `rm -rf /var/lib/jaunder/data` → `systemctl start jaunder.service`, which
  re-runs `preStart init` and rebuilds a **functionally equivalent** clean DB —
  then waits for the unit + port again.
- **Postgres** (`flake.nix:907-914`): a dynamic `TRUNCATE` of every
  `public`-schema table (excluding `_sqlx*` and the three migration-seeded
  reference tables) against an **already-empty** freshly-migrated DB — a no-op
  in effect.

The wipe buys nothing for correctness on a fresh VM: it destroys a clean DB only
to reconstruct an identical clean DB. It costs one service restart (SQLite) and
one no-op TRUNCATE (Postgres) per job, and the Postgres path carries a fragile
hand-maintained reference-table exclusion list that must track the schema.

## Equivalence argument (why removal is safe)

Both current paths converge on the **same** end state, and removal preserves it:

|                                                   | User-data tables | Reference tables (0018)          | `_sqlx*` bookkeeping |
| ------------------------------------------------- | ---------------- | -------------------------------- | -------------------- |
| SQLite, current (wipe → re-migrate → seed)        | seeded rows only | present                          | present              |
| SQLite, after (seed only)                         | seeded rows only | present (from boot)              | present              |
| Postgres, current (TRUNCATE-excluding-ref → seed) | seeded rows only | present (excluded from truncate) | present              |
| Postgres, after (seed only)                       | seeded rows only | present (from boot)              | present              |

The boot/`create-pg-db` migration already guarantees the exact clean slate the
wipe was reconstructing.

One asymmetry is worth naming: for SQLite the live server is already running
before seeding in **both** the current and the after states, so any hypothetical
server-startup write appears in both and removal is strictly neutral. For
Postgres the current `TRUNCATE` runs immediately before seeding, so it would
also erase any such startup write, whereas removal keeps it. Since
`jaunder serve` seeds no user rows at startup, this is a distinction without a
difference — and AC5's end-to-end pass (Postgres included) is what confirms it
empirically.

Additionally, the SQLite VM service boots with the **same** `captureEnv` + OTEL
environment the in-`seed_db()` restart used (`flake.nix:757`), so dropping the
restart loses no trace/capture configuration — seeding and the browser run
execute against the boot instance, which is identically configured.

## Scope

**In scope:** the two `seed_db()` functions inside the `flake.nix` NixOS-VM e2e
derivations (SQLite `seed_db`, `flake.nix:764`; Postgres `seed_db`,
`flake.nix:898`).

**Explicitly out of scope / must not change:**

- The **host reused-server e2e path** — `scripts/e2e-local.sh`,
  `xtask/src/steps/e2e_local.rs`, `cargo xtask e2e-local`. It runs against a
  long-lived `:3000` dev server with a genuinely dirty DB reused across runs, so
  its reset is load-bearing.
- **Rust unit/integration test DBs** (`make_*_app_state`, `TestEnv`, per-test
  temp SQLite, the Postgres test template).
- Production behavior — the boot `preStart` `jaunder init --skip-if-exists` is
  untouched; only the redundant _second_ wipe in the test script is removed.
- **Postgres `create-pg-db` + delayed service start** (`flake.nix:869`,
  `883-893`) — genuinely required (role/DB must exist before jaunder inits
  against Postgres).
- **The seeding itself** — `devtool seed-e2e` (users ×3, site-config ×2, mailbox
  reset) in both paths.

## Acceptance criteria

1. **SQLite `seed_db()` no longer wipes.** The `systemctl stop jaunder.service`,
   `rm -rf /var/lib/jaunder/data`, `systemctl start jaunder.service`, and the
   two subsequent `wait_for_unit` / `wait_for_open_port` calls are removed from
   the SQLite `seed_db()`. The function's only remaining action is the
   `devtool seed-e2e --db sqlite:/var/lib/jaunder/data/jaunder.db …` call, run
   against the boot DB while the boot service is still running.
2. **Postgres `seed_db()` no longer truncates.** The dynamic
   `DO $$ … TRUNCATE … $$` block (and its now-moot reference-table-exclusion
   comment) is removed from the Postgres `seed_db()`. The function's only
   remaining action is the `devtool seed-e2e --db postgres://… …` call.
3. **Load-bearing setup is untouched.** Postgres `create-pg-db`, the delayed
   `systemctl start jaunder.service`, and the boot `preStart` are unchanged; the
   host e2e-local path and Rust test-DB harness are unchanged (no diff outside
   the two `seed_db()` bodies and their comments).
4. **Comments reflect the new behavior.** The SQLite wipe-justifying comment
   (`flake.nix:765-769`) and the Postgres TRUNCATE-justifying comment
   (`flake.nix:899-906`) are removed or rewritten to describe seeding against
   the already-migrated boot DB — no comment left claiming a wipe/truncate
   happens.
5. **The full VM e2e matrix still passes.** `cargo xtask validate` is green
   locally — all four `{sqlite,postgres} × {chromium,firefox}` VM e2e combos —
   after the change, and CI's `e2e-gate` matrix (fresh runners = cold cache) is
   green on the PR before merge.

## Verification

Per the decision at spec time: run the full local gate `cargo xtask validate`
(all four VM e2e combos, warm cache) as the local proof, then rely on CI's
fresh-runner `e2e-gate` matrix for the cold-cache confirmation on the PR. Given
the documented e2e DB-state fragility history, both the SQLite and Postgres
browser runs must pass end-to-end (not merely boot), confirming the seeded state
is exactly what the specs expect.
