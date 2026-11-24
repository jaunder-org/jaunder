# ADR-DRAFT: Every Playwright worker owns its own E2E server instance

- Status: proposed
- Date: 2026-11-24
- Issue: [#2076](https://github.com/jaunder-org/jaunder/issues/2076)

## Context

Every E2E lane ran one Jaunder server and one database, shared by both
Playwright workers. Several site settings are single global rows that the server
reads live on every request: the site title and tagline, `base_url`, the
Registration Policy, the Site Default Audience, the Media Upload Capability and
the Site Theme. A test that changed one of them could break a test running at
the same moment on the other worker.

[ADR-0039](../0039-e2e-parallelism-via-per-test-identity-fixtures.md) point 3
handled this by quarantine. By #2076 the quarantined specs were a site-settings
group (`admin-site`, `backup`, `smtp`, the Theme specs, `media` and others that
changed or restored site configuration) plus `invite`. They ran in serial
Playwright projects ordered by project `dependencies`.
[ADR-0203](../0203-distribute-ci-validation-and-firefox-e2e-lanes.md) and
[ADR-0253](../0253-split-chromium-e2e-lanes.md) then gave each backend/browser
pair a serial-special lane: visual, global-configuration and invite projects in
dependency order, beside two ordinary shards. That lane ran at concurrency 1 and
became the E2E long pole. #2065 measured PostgreSQL/Firefox serial-special at a
14.0-minute median and moved the Theme specs into a fourth, serial-theme lane
per pair, sizing the ordinary shards by measured duration rather than test count
([ADR-0262](../0262-serial-theme-e2e-lane-and-weighted-shards.md)).

The quarantine also leaked. Restoring a setting after a test fixed only what the
_next_ test saw, never a test running at the same moment. Tests outside the
serial group still observed global state: any page that renders the title, any
UI sign-up that depends on the Registration Policy, and feeds that assume the
seeded `base_url`.

Playwright 1.61's `--shard` runs every dependency project in full in every
shard. A sharded lane can therefore have no dependency edges among the projects
it selects, or the dependency runs once per shard.

## Decision

**One server instance and database per Playwright worker.** Each harness starts
N Jaunder server instances, where N is the Playwright worker count. Each
instance has its own port, database, storage root and capture subdirectory. A
worker runs one test at a time, so no two tests ever use one server
concurrently.

- **Nix lane VMs.** `nix/checks.nix` generates units `jaunder-<i>`. Each one
  inherits the NixOS module's evaluated `jaunder` service config and
  environment, and overrides only its own bind address, `JAUNDER_DB`,
  `JAUNDER_STORAGE_PATH` and `JAUNDER_CAPTURE_DIR`. The module's own
  single-server unit is masked. PostgreSQL lanes create one role and database
  per instance (`jaunder_<i>`) with `jaunder create-pg-db` in the lane's one
  PostgreSQL server. The production module `nix/nixos.nix` is unchanged.
- **`dev e2e-local`.** The host driver starts one `HostServerSession` per worker
  (`JAUNDER_E2E_WORKERS`, default 1). Each session has its own temporary
  storage, SQLite database and ephemeral port, and is seeded, identity-checked
  on shutdown and panic-gated on its own.
- **The OTel collector stays shared.** Traces correlate by trace id, so one
  collector per lane or local run serves every instance.

**Workers bind by parallel index; the runner never binds.** Every Playwright
invocation sets `JAUNDER_E2E_INSTANCES`, a JSON array with one
`{ baseUrl, db, storagePath, captureDir }` entry per instance. It replaces the
former single base-URL variable. Single-server callers pass a one-entry map with
one worker. `end2end/tests/instance.ts` is the only module that reads instance
variables at load:

- In a worker process (`TEST_PARALLEL_INDEX` set), it selects the entry at the
  worker's parallel index, sets `BASE_URL`, and overwrites `JAUNDER_DB`,
  `JAUNDER_STORAGE_PATH` and `JAUNDER_CAPTURE_DIR`. Child tools (seeding,
  capture paths, mail) inherit them unchanged.
- In the runner process (spec loading, `--list`, census, reporters), it binds
  nothing and writes no environment. Any use of `BASE_URL` there throws.
- `playwright.config.ts` fails at load when the map's length differs from
  `workers`.

Base URLs use `localhost`, because passkeys need it. A replacement worker reuses
its predecessor's parallel index and therefore its instance. The state rule
below makes that safe.

**The state rule.** It replaces restore discipline across the whole suite:

- A test establishes the global state it depends on and asserts nothing about
  global state it did not set.
- Nobody cleans up. There is no reset, restore or snapshot infrastructure, and
  none may be added.
- An order-dependent failure is fixed by one of two moves: the test sets its own
  precondition, or the incidental assertion goes.
- `@visual` tests follow the same rule. Each seeds its own fixed-name user,
  because usernames render in the pixels. Each sets the seeded appearance its
  screenshot renders (title, tagline, Registration Policy, Site Default
  Audience, Media Upload Capability and no Site Theme selection) before it
  renders. Visual tests therefore need no fresh database and no ordering.
- A retried visual test may land on the instance where its earlier attempt
  already seeded the fixed-name user. It then signs in to that user instead of
  seeding it, and seeds none of the user's other state again, so the screenshot
  is unchanged.

**One project per gated browser, with no dependency edges.** Each gated browser
has exactly one project, `<browser>`, running every test, `@visual` included,
under the lane's retry policy. It has no dependency. A `snapshotPathTemplate`
keeps the reviewed baselines' `<name>-<browser>-visual-<platform>.png` names.
Each backend/browser pair runs four lanes, `--shard=1/4` to `--shard=4/4`, each
selecting that project, so plain sharding runs every test exactly once per pair.
The shards keep ADR-0262's weighting: the catalog's per-browser `shardWeights`,
exported as Playwright's `PWTEST_SHARD_WEIGHTS`, size the contiguous slices by
measured duration. Four shards and their weights are provisional, derived from
the three-shard lanes' reports, until a shard-count comparison on CI reports
settles them. `end2end/e2e-lanes.json` and `scripts/e2e-matrix.jq` enforce that
shape. The census's exactly-once check, run under the lanes' weights, catches
any dependency edge added later, because an edge would rerun its dependency in
every shard.

**Gates cover every instance.** The zero-panic gate dumps each instance unit's
journal and verifies it together with that instance's capture subdirectory. Any
failing instance fails the lane. Seed-trace verification runs after each
instance is seeded. Capture quiescence stops every instance before integrity
hashing.

**Capture layout.** Each instance writes its server streams (`diag.log`,
`mail.jsonl`, `websub.jsonl`) under `capture/instance-<i>/`. The collector's
streams (`otel-traces.jsonl`, `otel-logs.jsonl`) stay at the capture root. The
instance subdirectory name is defined once, by
`host::capture::instance_directory` and its inverse `instance_index`. Readers of
server streams, such as server-function coverage and the trace runner, read
every instance directory.

This decision narrowly supersedes and amends five earlier decisions. The rest of
each stays current.

- It supersedes
  [ADR-0039](../0039-e2e-parallelism-via-per-test-identity-fixtures.md) point 3
  and its quarantine rule. No spec is quarantined. The per-test identity
  fixtures (points 1 and 2) and `workers=2` remain.
- It supersedes the serial-special parts of
  [ADR-0203](../0203-distribute-ci-validation-and-firefox-e2e-lanes.md) and
  [ADR-0253](../0253-split-chromium-e2e-lanes.md): two ordinary shards plus one
  serial-special lane, ordered visual, global-configuration and invite projects,
  and their project dependencies. Their catalog-derived lanes, independent
  census, `always()` reconciliation, shared cap and trace seeds remain.
- It supersedes the serial Theme role and lane of
  [ADR-0262](../0262-serial-theme-e2e-lane-and-weighted-shards.md): the Theme
  specs are ordinary tests under the state rule. Its duration-weighted shards
  remain and now size every lane.
- It amends [ADR-0032](../0032-e2e-zero-panic-gate.md): the gate reads every
  instance unit's journal, not one `jaunder.service` journal.
- It amends [ADR-0057](../0057-e2e-capture-dir-contract.md): one capture
  directory per lane still holds every stream, but server streams move into
  per-instance subdirectories.

### Alternatives rejected

- **Finer serial projects, one per conflict domain**, with different domains
  allowed to overlap. This needs a conflict map that someone maintains by hand
  and nothing enforces. Most conflicts also hide in tests that only observe a
  setting.
- **Restore-scoped fixtures.** A restore runs after the test. It cannot protect
  a test observing the setting at the same moment.
- **More workers.** #828 found no admissible three-worker configuration.
- **Per-worker servers for the global specs only.** This keeps two server
  topologies and the serial lanes.
- **Server lifecycle owned by a Playwright fixture.** It would re-implement the
  systemd unit's environment, the seed and panic gates and capture quiescence
  that the harnesses already own.
- **Explicit partitions or ordering for visual tests.** The state rule makes
  them unnecessary.

## Consequences

- Good: the site-global specs run in parallel with everything else, and no lane
  carries a concurrency-1 tail.
- Good: each pair's four lanes are identical apart from the shard index, so the
  catalog, census and reconciliation need no special lane or project.
- Good: global-state races are removed by construction. They no longer depend on
  quarantining the right specs.
- Cost: each VM runs one more Jaunder process and one more database. Measured
  anonymous memory per instance stayed at about 110 MiB or less. The Firefox
  lanes' minimum `MemAvailable` was 126–241 MiB on their 4 GiB guests. Guest
  sizes are unchanged and that headroom is watched. The browser count per VM is
  unchanged at two.
- Cost: two harnesses run N instances instead of one.
- Constraint: VM seeding runs as the service user (`runuser -u jaunder`), so the
  service account owns each instance's publisher gate. Created by root, the gate
  would lock the server out of publishing.
- Constraint: tests that formerly ran in the Chromium site-settings or visual
  projects lost those projects' implicit 2.2× slow-browser timeout scale. Only
  the `chromium` project keeps the unscaled Chromium budget. One
  package-lifecycle test now carries an explicit whole-test budget instead.
- Constraint: a visual test's fixed-name user is seeded once per instance and
  reused by a retry there. `--repeat-each` on a visual test is unsupported.
- Constraint: the shard weights describe the suite's current order and
  durations. Re-derive them from lane reports when the shards drift apart.
- Constraint: any new gated project, and any dependency edge, must keep the
  exactly-once shard invariant. The census, lanes and reconciliation remain one
  contract.
