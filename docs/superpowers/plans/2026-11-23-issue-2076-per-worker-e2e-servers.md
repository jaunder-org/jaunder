# Per-worker E2E servers Implementation Outline

> Execute with dev-cycle-iterate. Spec:
> `docs/superpowers/specs/2026-11-23-issue-2076-per-worker-e2e-servers.md`.
> Outline needed because the change crosses the CI lane topology, the Nix VM
> harness, the host-proof driver and the Playwright census/reconciliation
> contract.

## Execution summary

First, prove the core mechanism on the host. `dev e2e-local` starts two
instances, and each Playwright worker binds to its own instance. The existing
site-settings project then runs concurrently with an ordinary spec. Next, prove
the same in one hermetic VM lane per backend, with the existing gates looping
over the instances and instance memory measured.

With the mechanism proven, apply the state rule to the specs while the old
topology still stands. Any failure at that point is a spec problem, not a
topology problem. That includes the `@visual` tests: once they establish their
own state, they need no ordering. Then merge the roles and lanes. Projects keep
no dependency edges, because Playwright 1.61's `--shard` reruns dependency
projects in every shard. With no edges, plain `--shard` splits every test
exactly once.

## Task outline

### Deliverable A — instance mechanism

- [x] Task 1: Workers bind to their own instance; the host driver starts N
      instances
  - Contract:
    - `JAUNDER_E2E_INSTANCES` is a JSON array with one entry per instance index:
      `{ baseUrl, db, storagePath, captureDir }`. It replaces
      `JAUNDER_E2E_BASE_URL`, and the `localhost:3000` default in `helpers.ts`
      goes away.
    - Every Playwright invocation sets this map. Single-server callers pass a
      one-entry map with workers=1. Those callers are:
      - production-baseline qualification
        (`tools/qualification/src/production_baseline.rs:1546`);
      - the perf, wasm-coverage and measurement runs in `nix/checks.nix`.
    - An instance module is the first import of `fixtures.ts` and `helpers.ts`.
      It reads `TEST_PARALLEL_INDEX`, which workers set before loading spec
      files.
      - **Runner process** (spec loading, `--list`, census, reporters): the
        index is absent. The module touches nothing, and `BASE_URL` resolves
        lazily, throwing only if used.
      - **Worker process:** the module selects its entry, sets `BASE_URL`, and
        overwrites `JAUNDER_DB`, `JAUNDER_STORAGE_PATH` and
        `JAUNDER_CAPTURE_DIR`. Child tools (seed, capture, mail) keep inheriting
        those variables unchanged.
    - Invariant: no module reads `JAUNDER_*` at load, other than the instance
      module.
    - `playwright.config.ts` fails at load when the map is present and its
      length ≠ `workers`.
    - Host driver: one `HostServerSession` per worker. Each has its own tempdir
      and capture dir, is seeded with `seed-e2e`, shuts down with an identity
      check, and gets its own panic check.
  - Verification:
    - `dev e2e-local` with `JAUNDER_E2E_WORKERS=2` runs `admin-site.spec.ts`
      (existing site-settings project) together with one ordinary spec. Both are
      green.
    - A run whose map length doesn't match `workers` fails at load.
    - The census and reporter output contain no instance values.
    - `--update-visual-snapshots` leaves the PNGs unchanged.

- [x] Task 2: Hermetic VM lanes run N instances with every gate covering each
  - Contract:
    - `checks.nix` generates `jaunder-<i>` units. The production module
      `nix/nixos.nix` is unchanged.
    - Each unit inherits the module unit's service config and environment, and
      sets its own port, DB, storage path and `capture/instance-<i>/` directory.
      Seeding runs as the service user, so the service account owns each
      instance's publisher gate.
    - The collector's files stay in `capture/`.
    - Postgres: one `create-pg-db` per instance, database `jaunder_<i>`.
    - Per-instance gates: seed, then `verify-seed-trace`; journal dump, then
      `verify-no-panics`; stop before capture quiescence.
    - Readers of server-stream files (`diag.log`, `mail.jsonl`, `websub.jsonl`)
      read every `instance-<i>/` directory: server-fn coverage IO, the trace
      runner, and the mail/capture helpers.
    - The E2E unit-env assertion (`checks.nix:1803`) targets the instance units.
      The production-module assertions (`checks.nix:1791-1800`, `2193`) are
      untouched.
    - The perf check declares one instance. The workers=4 control declares 4.
      The wasm-coverage checks keep the module's own unit, with one shared
      single-server entry.
  - Verification:
    - `dev e2e-producer` is green for one sqlite and one postgres Firefox lane
      on the current topology.
    - The artifacts show per-instance journals and capture directories.
    - An injected panic in one instance fails the panic gate (probe only, not
      committed).
    - Peak RSS per instance (`systemctl show -p MemoryPeak`) and VM memory
      headroom are recorded in session evidence.
  - Risk stop: if a Firefox lane's headroom does not fit the current sizing plus
    a modest bump, return to the user.

### Deliverable B — state rule, then topology

- [x] Task 3: Apply the state rule across the suite (old topology still in
      place)
  - Contract:
    - Remove pure-restore code. Where a `finally` also asserts something, keep
      the assertion.
    - Remove the restore half of `withSiteThemeSelection`, and the project-role
      guards in `passkeys` and `theme-helpers`.
    - Remove assertions that aren't germane to the test.
    - Make observers set what they depend on. Example: `feeds.spec.ts` sets
      `base_url`.
    - Delete the `site-config-snapshot`/`restore` subcommands if no consumer
      remains.
    - The five `@visual` tests stop using `testlogin` and `visualauthor`. Each
      one seeds its own fixed-name user. Each one also sets the global settings
      its screenshot renders, using the seed values.
  - Verification:
    - Each `@visual` test passes against its baseline when it runs after
      site-global specs on the same instance. The cockpit and composer baselines
      were regenerated for plainly named users (user decision): only the avatar
      changes. The other baselines are unchanged.
    - `dev e2e-local` is green across the full suite in two genuinely different
      orders:
      - workers=1, where one instance sees every test in file order;
      - workers=2 with `fullyParallel`, where interleaving differs.
    - The orderings are diagnostics only; nothing about them is committed.

- [x] Task 4: Merge roles and reshard the lanes
  - Contract:
    - Each gated browser has two projects: `<browser>-visual` (`@visual` grep,
      retries 0) and `<browser>` (everything else). Neither has a dependency.
      Ordinary absorbs site-settings and invite. `webkit-theme` folds into
      `webkit`.
    - The unsplit/split distinction collapses if the two graphs no longer
      differ.
    - `projectName` and the role mapping in `e2eProjects.ts` change. The old
      catalog names (`*-special-*`, `*-admin`, `*-admin-site`, `*-ordinary`)
      disappear.
    - Also updated:
      - `e2e-lanes.json`: per pair, 3 lanes selecting both projects with
        `--shard=k/3`, plus new trace seeds;
      - `scripts/e2e-matrix.jq`;
      - census and reconciliation;
      - the required-check join in `ci.yml`.
  - Verification:
    - Hermetic `dev e2e` is green on all lanes.
    - The census counts every test exactly once, including each visual test once
      per pair.
    - The WebKit merge is proven with a host-local `dev e2e-local` run of the
      webkit project.
    - Instance RSS and VM headroom are re-measured on the shipped layout (one
      Firefox lane per backend). The workers=4 control's sizing is rechecked.
      Report anon memory or minimum MemAvailable, not only cgroup MemoryPeak,
      which includes page cache.
    - Afterwards, remove the `report_e2e_memory()` measurement scaffolding. The
      numbers go in the PR body.

### Deliverable C — record and docs

- [x] Task 5: ADR draft and documentation projection
  - Contract:
    - A numberless `jaunder-adr` draft records per-worker instances and the
      state rule, including visual. It also records the no-dependency-edges
      rule, because `--shard` expands dependencies.
    - It supersedes ADR-0039 point 3 and the serial-special parts of ADR-0203
      and ADR-0253, and amends ADR-0032 and ADR-0057.
    - Updated together in the same change:
      - `docs/ARCHITECTURE.md`, the E2E topology section (~4205-4441);
      - `CONTRIBUTING.md`, the E2E sections, lane counts and visual workflow;
      - the `playwright.config.ts` project comment block;
      - the header comment of `scripts/e2e-matrix.jq`.
  - Verification:
    - `dev check-no-test` is green.
    - A `jaunder-adr-projection` audit of the touched ARCHITECTURE sections.

### Deliverable D — integrate #2065 and choose the shard layout

- [x] Task 6: Rebase onto `main` (#2065 landed); one project per browser;
      provisional 4 weighted shards
  - Contract:
    - Rebase onto `origin/main`, resolving by intent. This topology supersedes
      #2065's serial Theme role and lane. ADR-0262's weighting mechanism is kept
      (`ordinaryShardWeights` → `PWTEST_SHARD_WEIGHTS`).
    - Drop `<browser>-visual`: each gated browser has one project, `<browser>`.
      Visual tests take the lane's retry policy. A retried visual test reuses
      its fixed-name user (`createSessionViaTool` when it already exists). A
      `snapshotPathTemplate` keeps the existing `…-<browser>-visual-linux.png`
      names, so no PNGs change.
    - Catalog: 4 lanes per pair, `--shard=k/4`, with provisional weights.
    - The ADR draft also supersedes ADR-0262's serial Theme lane and keeps its
      weighting. Docs follow.
  - Verification:
    - The census selects every test exactly once per pair.
    - Focused `dev e2e-local` runs cover a visual spec and a former Theme spec.
    - `tsc`, host-proof/dev tests and `dev check-no-test` are green.
    - One hermetic lane per backend is green.

- [x] Task 7: Shard shoot-out
  - Contract:
    - Open the PR, a ship action, at the Task 6 configuration.
    - From that CI run's Playwright reports, take per-test durations for each
      backend×browser pair. Simulate Playwright's weighted shard assignment for
      3, 4 and 5 shards. Estimate each lane's wall-clock as two-worker time plus
      the fixed VM boot and seed cost.
    - Report a table: the slowest lane, the spread, the runner-minutes and the
      best weights. The user picks the count.
    - Apply the chosen count and weights to the catalog and docs.
  - Verification:
    - The simulation is validated against that CI run's actual lane times for
      the 4-shard configuration.

### Ship-boundary proof (owned by jaunder-ship, listed for coverage)

- Run PR CI E2E 5 times on the chosen configuration, each with its own salt.
  - Pass if the slowest lane's median is below `main`'s current post-#2065
    slowest-lane median.
  - Record the per-lane medians and runner-minutes against `main`, and confirm
    no new class of flake.
- Put the RSS evidence from Task 4 in the PR body.
- After landing, re-scope or close #2065.

## Risk checks

- Nothing in the runner process may resolve an instance. Census, `--list` and
  reporters must not bind to instance 0, and nothing the runner forks may
  inherit an instance's env.
- A restarted worker reuses its `parallelIndex`. That's expected, because no
  test depends on a fresh instance. A retried visual test reuses its fixed-name
  user. `--repeat-each` on a visual test is unsupported.
- Hard-coded `:3000` in `bootBudgetAccounting.spec.ts` and `websub.spec.ts` is
  string data. Confirm that, and leave it unchanged.
- After the merge, the duration-pressure, boot-decomposition and trace-capture
  gates must still see every executed project.
- Adding a dependency edge to any gated project would silently rerun its
  dependency in every shard. The census's exactly-once check catches that.
