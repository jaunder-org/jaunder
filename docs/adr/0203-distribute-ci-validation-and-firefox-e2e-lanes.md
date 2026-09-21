# ADR-0203: Distribute CI validation and Firefox E2E through retained lane catalogs

- Status: accepted
- Date: 2026-09-20
- Issue: [#1574](https://github.com/jaunder-org/jaunder/issues/1574)

## Context

CI's serialized non-e2e validation became the required-check critical path. A
first split into core and coverage lanes helped, but still serialized
independent host, hermetic, wasm/doctest/Elisp, and source-closure work. Once
that path was shortened, the unsplit Firefox VM became the long pole.

The required contexts must remain stable: `Validate (no e2e)` and `e2e gate`.
Local `cargo xtask validate` must remain the full equivalent of distributed CI.
Final coverage and E2E verdicts are per-ref and cannot come from Cachix. E2E
fan-out must preserve backend parity, visual ownership, retries, panic checks,
duration budgets, boot traces, and failure diagnostics, with isolated mutable
state per lane.

Issue #1574 measured cache-state-matched warm and controlled source-invalidating
validation cohorts. The retained four-job validation topology reduced warm p95
by 17.3% (19.8% median) and broad-cold p95 by 38.2%, with lower runner cost than
the faster five-job alternative. The owner explicitly accepted the warm-p95
close miss after reviewing all six green observations and the workload mix.

The Firefox experiment compared workers=2 unsplit, two ordinary shards plus one
serial-special lane, and workers=4 unsplit for both backends. Across three
successful fresh-realization repetitions, split p95 improved 46.4% for SQLite
and 43.2% for PostgreSQL versus workers=2. Runner-time proxies rose 28.4% and
39.6%, and retained artifact volume stayed about 1.00× and 1.02× control.
Independent censuses and lane-qualified reports, manifests, phases, retries, and
traces reconciled exactly. Workers=4 remained unsuitable: two PostgreSQL runs
failed both retries of global theme-management flows and another passed only
after a retry.

## Decision

CI distributes non-e2e validation across four independently scheduled lanes, all
selected from the same ordered validation-surface catalog used by local
`validate --no-e2e`:

1. `host` owns the verify-only host/static gate and auxiliary host tests.
2. `hermetic` owns Nix static checks and the wasm budget.
3. `test-checks` owns wasm tests, doctests, and Elisp coverage, then runs the
   source-closure probe under `always()` on that already Nix-populated runner.
   Test-check and probe verdicts remain independently visible and either fails
   the job.
4. `coverage` owns Rust coverage and its gate.

The result-only `Validate (no e2e)` job requires all four. Producer/consumer
pairs remain within one job, every lane retains its clean-tree authority, and no
final verdict is accepted from Cachix.

Production E2E uses one shared lane catalog. Chromium remains one unsplit lane
per backend. Firefox uses three isolated lanes per backend:

- ordinary shard 1 of 2;
- ordinary shard 2 of 2; and
- one serial-special lane owning visual tests exactly once plus the ordered
  global-configuration and invite projects.

Each lane has its own NixOS VM, database, storage, trace identity, report,
manifest, phase sidecar, capture, journals, and uploaded artifact. Every Firefox
lane runs an independent unsharded preflight census. A host-only reconciliation
job runs under `always()` after all producers, requires the exact enabled lane
set, proves the report/manifest union equals that census without omissions or
duplicates, and independently validates duration, retry, panic, and trace
evidence. Failed producers are still awaited and their recoverable evidence is
included before aggregate failure. The stable `e2e gate` requires Chromium, all
Firefox producers, and both backend reconciliation jobs.

Local full `cargo xtask validate` realizes the same eight enabled lanes and
performs the same Firefox reconciliation after all builds have settled. Unsplit
Firefox workers=2 and workers=4 remain measurement-only package outputs;
workers=4 is not a production policy.

This decision narrowly supersedes:

- [ADR-0034](0034-ci-e2e-matrix-distribution.md) where it specifies a four-job
  `{backend}×{browser}` matrix and one non-e2e job; its same-derivations,
  distributed-CI model, local full gate, host-only xtask boundary, and stable
  aggregate contexts remain current;
- [ADR-0039](0039-e2e-parallelism-via-per-test-identity-fixtures.md) where it
  treats one workers=2 Firefox process graph per backend as the production
  parallelism boundary; workers=2 remains the intra-lane policy, while
  global-state projects are additionally isolated in the serial-special VM; and
- [ADR-0192](0192-split-ci-non-e2e-validation-lanes.md) where it specifies two
  non-e2e jobs and an unchanged four-combination E2E matrix; its per-ref
  verdict, shared-catalog, producer/consumer, and stable-context constraints
  remain current.

## Consequences

- Good: validation and Firefox latency are reduced without deleting or caching a
  final verdict.
- Good: independent lane identities and host reconciliation make omission,
  duplication, evidence collision, and partial failed-lane reporting explicit
  failures.
- Good: global Firefox state is isolated by VM rather than relying only on
  per-test identities inside one mutable site.
- Good: local full validation and CI consume one authoritative retained lane
  catalog.
- Neutral: branch protection still requires only `Validate (no e2e)` and
  `e2e gate`; internal job and matrix names may evolve.
- Cost: validation repeats runner setup, and split Firefox raises aggregate
  Firefox runner time by roughly 28–40% in the accepted cohort.
- Cost: CI runs eight E2E producer VMs instead of four, plus two lightweight
  host reconciliation jobs. Lane evidence is retained once; reconciliation
  uploads only its result sidecar rather than duplicating producer artifacts.
- Constraint: Firefox lane filters, project dependencies, independent census,
  and reconciliation are one contract. A topology change is incomplete unless
  all four move together and are remeasured.
- Constraint: workers=4 remains measurement-only until new evidence overturns
  its reproduced PostgreSQL global-state instability.
