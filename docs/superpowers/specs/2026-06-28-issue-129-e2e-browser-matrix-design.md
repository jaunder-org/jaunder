# Spec: Parallelize e2e by fanning out a {backend}×{browser} matrix (issue #129)

- **Issue:** jaunder-org/jaunder#129
- **Date:** 2026-06-28
- **Status:** approved (design)
- **Related:** #130 (lower e2e VM driver timeout — independent), ADR-0033 (this work)

## Problem

e2e is the dominant CI cost — ~17.3 min of a ~24 min `validate` run (measured
2026-06-28, run 28322600832). The two backend VMs (sqlite, postgres) already run
**concurrently** (one `nix build` realizes both via Nix concurrency), so they are
not the bottleneck. The real serialization is **inside each VM**: each runs the
full Playwright suite twice, once per browser, back-to-back — two hardcoded
sequential `seed_db()` + `machine.succeed("playwright test --project <browser>")`
blocks (flake.nix:622-650 sqlite, :766-794 postgres; chromium ~6 min then firefox
~10.6 min). Because both browser runs share one jaunder instance + one DB
(re-seeded between them), they cannot run concurrently in the same VM.

So total e2e wall ≈ max(sqlite 16.9m, postgres 17.3m) ≈ 17.3 min, of which
~16.5 min is the serial double browser run.

## Goals

- Stop running the two browsers serially within a VM.
- Drop CI e2e wall-clock toward the slowest single combo (~10.6 min) — a ~6–7 min
  reduction on a ~24 min run.
- Crisp per-combo failure reporting (which backend×browser failed).
- Preserve today's exact e2e coverage (both browsers on both backends).
- Keep `cargo xtask validate` the full local gate (now parallel-by-browser).

## Non-goals

- Reducing the coverage stage's 156s instrumented compile (separate, compile-side).
- Fixing the underlying boot/infra flake or the 60-min driver timeout (that is #130).
- Splitting `validate --no-e2e` further (static+clippy from coverage) — measured
  marginal (~1 min, eaten by a second runner's setup); coverage is the long pole.
- Trimming the matrix below full 2×2 (coverage parity is required).

## Design

### 1. Flake: parameterize the e2e checks by browser

Parameterize `mkE2eSqliteCheck` / `mkE2ePostgresCheck` to take a `browser` (and
its distinct `traceId`/`TRACEPARENT`) as a uniform argument, so each produces a VM
that runs **one** `playwright test --project <browser>`. Drop the second
`seed_db()` + `machine.succeed` block; each derivation seeds once and runs one
browser.

Instantiate the warm gate combos (replacing today's `checks.e2e-sqlite` /
`checks.e2e-postgres`, which pass `JAUNDER_E2E_WARMUP=1`):

- `e2e-sqlite-chromium`, `e2e-sqlite-firefox`
- `e2e-postgres-chromium`, `e2e-postgres-firefox`

The `e2e` aggregate (`e2e-checks` `symlinkJoin`, flake.nix:824-829) gains the 4
entries so a single `nix build` of the aggregate (what local `cargo xtask
validate` builds) realizes all 4 concurrently up to host `max-jobs`.

Shared, already-cached inputs are unchanged and not duplicated in build cost:
`jaunderBin` (one backend-agnostic Rust build), the e2e npm bundle, and the
generated Playwright config (which already defines both `chromium` and `firefox`
projects; we only change which `--project` flag each derivation passes). The
per-VM OTel sidecar and zero-panic gate (ADR-0032) are duplicated per combo
exactly as they are per backend today.

### 2. Cold diagnostic packages: per-browser, full parity

The `-cold` packages (`packages.e2e-{sqlite,postgres}-cold`, flake.nix:811-817)
are built by `scripts/run-e2e-trace-analysis --cold` to capture cold-cache
(no-warmup) OTel navigation traces — a manual observability tool, not part of the
gate (CONTRIBUTING.md:225-236, docs/observability.md:65-75).

Since the parameterization is uniform, the cold variants also become per-browser:
`e2e-{sqlite,postgres}-{chromium,firefox}-cold` (4). This keeps **one** code path
in `mkE2e*Check` (browser is just an arg; warm/cold differ only by `warmupEnv`).

- Add explanatory comments on the cold package definitions stating *why* they
  exist (cold-cache trace diagnostics via `run-e2e-trace-analysis`), so future
  readers don't mistake them for dead duplicates.
- Update `scripts/run-e2e-trace-analysis`: add a `--browser <chromium|firefox>`
  selector (default: both → build the relevant cold combos) replacing the two
  hardcoded attrs at lines 120-124.
- Update the package lists in `CONTRIBUTING.md` and `docs/observability.md`.

### 3. CI workflow restructure (.github/workflows/ci.yml)

Split the single `validate` job into:

- **`validate-no-e2e`** — runs `cargo xtask validate --no-e2e` (static + clippy +
  coverage + coverage-gate). Behavior unchanged; kept as one job (coverage is the
  long pole, splitting saves ~0 net).
- **`e2e`** — `strategy.matrix` over `backend ∈ {sqlite, postgres} ×
  browser ∈ {chromium, firefox}` (4 jobs), each running
  `nix build .#checks.x86_64-linux.e2e-<backend>-<browser>` with the existing Nix
  + `cachix/cachix-action` setup. Each job pulls `jaunderBin` + the e2e bundle
  warm from Cachix (no recompile); the e2e derivation stays `pushFilter`-excluded
  (`jaunder-coverage|jaunder-e2e`) so it always re-runs.
- **`e2e-gate`** — a tiny aggregator job that `needs:` all 4 matrix jobs and
  succeeds iff all passed. Gives branch protection one stable required-check name
  immune to matrix-value churn.

All jobs keep cachix configured (pull warm; deps-closure push as today).

### 4. Required-checks wiring (operational, at landing)

Today branch protection requires the single `validate` check; after the split
that name disappears. Update the ruleset/branch protection to require
**`validate-no-e2e`** + **`e2e-gate`**. This is a repo-admin settings change
applied (or approved) by the maintainer at landing — a deliberate halt point, not
silently skippable (skipping it would leave PRs ungated).

### 5. ADR + CLAUDE.md

- Write **ADR-0033** (next after 0032): *CI distributes e2e across a GitHub
  Actions matrix; `cargo xtask validate` remains the full local gate, but CI no
  longer runs e2e via a single `validate` command — CI faithfulness becomes "the
  same Nix check derivations, distributed across runners," with `e2e-gate`
  aggregating.* Add its row to the ADR table in `docs/README.md`.
- Update the `# xtask` section of `CLAUDE.md`, which currently states CI runs
  `nix develop .#ci -c cargo xtask validate` as one command.

## Verification

- **Local:** `cargo xtask validate` builds all 4 e2e combos green (parallel via
  Nix); confirm browsers no longer run serially within a VM.
- **CI:** the new workflow runs; the 4 matrix jobs pass; `e2e-gate` aggregates
  green; `validate-no-e2e` unchanged. Measure the new e2e wall-clock — expect it
  to drop toward ~the slowest single combo (~10.6 min) from ~17.3 min. Confirm
  Cachix still serves `jaunderBin`/e2e-bundle warm (no workspace recompile in the
  e2e jobs).
- **Diagnostics:** `scripts/run-e2e-trace-analysis --cold` (and with `--browser`)
  still builds and analyzes traces.

## Risks / tradeoffs

- **Per-combo VM-boot/backend-bringup duplication:** each of the 4 combos boots
  its own VM + one `seed_db()` (~boot 12s + app/DB ready 20–26s), where today two
  browsers amortize one bring-up per VM. Negligible (~30s/combo) and absorbed by
  running on separate runners.
- **Billing-minutes:** 4 e2e runners instead of 1 job. Accepted — parallel, so no
  wall-clock penalty; the win is wall-clock + reporting.
- **CI-model shift:** CI no longer == one `cargo xtask validate` command. Recorded
  in ADR-0033 and CLAUDE.md; local `validate` remains the single full gate.
- **Required-checks gap:** if the ruleset isn't updated at landing, PRs go
  ungated. Mitigated by the explicit landing halt point (§4) and the stable
  `e2e-gate` name.

## Out of scope (separable, tracked elsewhere)

- #130 — lower the 60-min e2e driver timeout.
- Clippy caching (route validate's clippy to the cached Nix `clippy` crane check)
  — tracked as #10 (the migrated `jaunder-b2i1` bead); adjacent, not required here.
