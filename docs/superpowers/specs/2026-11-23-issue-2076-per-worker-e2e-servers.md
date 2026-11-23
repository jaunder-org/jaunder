# Issue #2076 — per-worker E2E servers

## Outcome

Every Playwright worker uses its own jaunder server instance and database. A
worker runs one test at a time, so no two tests ever use one server
concurrently. That lets the site-global specs (the site-settings group and
`invite`) run in parallel with everything else. The serial Playwright projects
and the `serial-special` CI lanes go away.

## Approach and tradeoffs

**Why a server per worker.** Several site settings are single global rows that
the server reads live on every request: the site title, `base_url`, registration
policy, default audience, the uploads switch, and the site theme. Restoring a
setting after a test only fixes what the _next_ test sees. It does nothing for a
test running at the same moment.

The simplest alternative was finer-grained serial projects, one per "conflict
domain", with different domains allowed to overlap. We rejected it because it
depends on a conflict map that someone has to maintain by hand and that nothing
enforces. Most conflicts would also hide in tests that only _observe_ a setting:
any page that renders the title, or any test that signs up. Raising the worker
count is ruled out because #828 found no 3-worker configuration that works.

Another option is per-worker servers for the global specs only, with ordinary
tests left on today's single server. We rejected it because it keeps two server
topologies and leaves the serial lanes in place.

**Who starts the instances.** The harness runs N instances, where N is the
Playwright worker count. Each instance has its own port, database, storage path
and capture subdirectory. The two harnesses do this as follows:

| Harness         | How it starts N instances                                                                                                                                                                                                                            |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Nix lane VM     | Templated systemd units `jaunder@<i>`. Each one gets the same environment, preStart, user and stop behaviour as today's `jaunder.service`. Postgres lanes create one database per instance with `create-pg-db`, in the one existing Postgres server. |
| `dev e2e-local` | N `HostServerSession`s. These are already identity-checked and use port 0. SQLite only, as today.                                                                                                                                                    |

The OTel collector stays shared by all instances, because traces correlate by
trace ID.

**How tests find their instance.** Playwright reads an instance map from the
environment: for each index, the base URL, DB, storage path and capture
directory. A worker selects its entry by `parallelIndex`. Everything that is
fixed per process today then becomes per worker:

- the module-level `BASE_URL` constant;
- `JAUNDER_DB`;
- `JAUNDER_STORAGE_PATH`;
- `JAUNDER_CAPTURE_DIR`.

Base URLs use `localhost`, not `127.0.0.1`, because passkeys need it. The config
fails at startup unless the map has exactly `workers` entries.

When Playwright replaces a failed worker, the new worker gets the same
`parallelIndex` and so reuses the same instance. That is acceptable under the
state rule below.

**Existing gates loop over the instances.** Every check that today looks at one
server now does the same thing for each instance:

- the zero-panic gate (ADR-0032), which reads each unit's journal and capture
  subdirectory;
- `verify-seed-trace`, after each instance is seeded;
- capture quiescence, which stops every unit before integrity hashing;
- the unit-environment assertion in `nix/checks.nix`.

Consumers that expect a single `capture/diag.log`, such as server-function
coverage, either merge the per-instance subdirectories or read each one.
ADR-0057's capture-directory contract is amended to match. The measurement,
performance and wasm-coverage VM checks keep their own unit definitions, run as
the same template.

**Costs we accept.**

- Each VM runs N−1 more jaunder processes and N−1 more databases. The browser
  count is unchanged at 2 per VM, and browsers are what ran out of memory in
  #828. We measure and record each instance's peak memory use (RSS).
- The unsplit workers=4 measurement control would run 4 instances. It is a
  measurement artifact, not production, but its memory sizing is rechecked.
- Two harnesses change instead of one.

**State rule.** This replaces restore discipline, across the whole suite:

- A test sets the global state it depends on and asserts nothing about global
  state it did not set. Nobody cleans up.
- Delete the existing restore machinery: config resets in `finally`, `afterAll`
  and `afterEach`; the restore half of `withSiteThemeSelection`; snapshot and
  restore usage, and the snapshot/restore tool if nothing else uses it; and the
  project-role guards.
- Delete assertions that aren't germane to the test. For example, `theme.spec`
  asserts the site theme is Studio while it is testing an author theme.
- Observers outside the old serial group are in scope too. For example,
  `feeds.spec.ts` assumes the seeded `base_url` `https://example.com` and should
  set it itself.
- Account provisioning (`seedUserViaTool`) already bypasses registration policy.
  UI sign-up tests set the policy themselves.
- When an order-dependent failure appears, fix it with the same two moves: the
  test sets its own precondition, or the incidental assertion goes. Never add
  reset infrastructure.

**Topology.**

- The site-settings and invite roles merge into the ordinary role for Chromium,
  Firefox and WebKit (`webkit-theme` merges into `webkit`), in both the unsplit
  and split graphs.
- `@visual` must still run once per backend×browser pair, and first on its
  instances, because its exact-pixel public timeline needs a fresh database. How
  a sharded lane does this (Playwright sharding versus project dependencies) is
  resolved by a feasibility probe in the outline. It must not add a runner.
- Each pair keeps today's 3 runners as ordinary shards, unless measurement shows
  a better balance.
- These all follow the new layout: `end2end/e2e-lanes.json`,
  `scripts/e2e-matrix.jq`, the census and reconciliation, and the documentation.

**Decision record.** A `jaunder-adr` draft records per-worker instances and the
state rule. It supersedes ADR-0039's quarantine rule (point 3), and the
serial-special parts of ADR-0203 and ADR-0253. It amends ADR-0032 (the panic
gate covers every instance) and ADR-0057 (the capture layout).
`docs/ARCHITECTURE.md` and `CONTRIBUTING.md` are updated in the same commit.

## Acceptance

Proof to execute:

- **Full suite:** hermetic `dev e2e` is green for both backends and all
  browsers. Census and reconciliation account for every test exactly once. The
  panic gate, seed-trace gate, capture integrity and server-function coverage
  all pass with the per-instance layout.
- **Order independence:** before landing, the merged suite runs green under at
  least two different orderings (for example, reversed file order). Any test
  that fails only under one ordering is fixed by the state rule.
- **CI timing:** PR CI runs E2E at least 5 times, each with a distinct
  `e2eSalt`, on both backends. The slowest E2E lane's median must come in under
  the #2065 baseline of 14.0 min (postgres-firefox-serial-special). Record the
  per-lane medians and the total E2E runner-minutes. No new class of flaky
  failure may appear in the retry or flaky reporting.
- **Memory:** measure peak RSS per instance and the VM's memory headroom in at
  least one Firefox lane per backend, and record them in the PR.
- **Local runs:** `dev e2e-local <spec>` and `--update-visual-snapshots` work
  with N instances.

Deliverables to commit:

- the instance template and gate loops;
- host-driver support for N sessions;
- the worker-scoped instance access;
- the merged projects and lane catalog;
- the state-rule spec edits;
- the ADR draft and the documentation updates.

## Boundaries

- The worker count and vCPU count stay as they are. Memory may be raised only if
  the measured RSS requires it, and any change is recorded.
- No new restore, reset or snapshot infrastructure.
- No product code changes. A real bug exposed by a spec edit is filed
  separately.
- `dev e2e-local` gains no Postgres support.
- After landing, #2065 is re-scoped or closed against the new lane timings. Its
  remaining work is not done here.
