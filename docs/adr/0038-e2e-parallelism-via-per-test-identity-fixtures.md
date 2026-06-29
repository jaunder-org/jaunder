# 0038. E2E parallelism via per-test identity fixtures + a serial project for global-singleton specs

- Status: accepted
- Date: 2026-06-29
- Issue: #61

## Context

The Nix-VM Playwright suite pinned `workers: 1`. The original justification was
SQLite read-then-write `SQLITE_BUSY` contention, resolved by `BEGIN IMMEDIATE`
(#18, #51/#52/#53). What remained were **logical races on shared global state**:
shared seeded accounts (`testlogin`/`testoperator`/`testnoemail`), the singleton
`site.title`/`base_url` that `admin-site` mutates and other specs assert, an
exact global-feed count in `posts`, and a global-newest (unfiltered) mail waiter.

## Decision

Parallel-safety is achieved per test, not per database:

1. **Per-test identity fixtures** (`end2end/tests/fixtures.ts`): `user`
   provisions a uniquely-named account out-of-band (in a throwaway browser
   context, so the test page stays logged out); `mailbox` is a recipient-scoped,
   cursor-tracked mail waiter bound to that account's unique address;
   `verifiedUser` adds the email-verification flow. Lazy and test-scoped, so each
   test that destructures them gets isolated state and the boilerplate lives once.

2. **One serial exception by config, not code.** `admin-site` mutates the
   singleton site identity, so each browser has a paired **serial** Playwright
   project (`${browser}-serial`, `testMatch` admin-site, `fullyParallel: false`)
   that `dependencies` on the parallel project and therefore runs in a
   non-overlapping phase. The VM invocation passes both `--project ${browser}`
   and `--project ${browser}-serial`. The title-asserting specs (`example`,
   `posts`) keep their `"jaunder.local"` assertions because the mutator can never
   overlap them. `admin-site` keeps the seeded `testoperator` account — operator
   privilege cannot be self-served via `register()` — so the quarantine is by
   config, not by de-seeding.

3. **Own-scoped feed assertions.** `posts`' anonymous local-timeline test asserts
   a full first page (`toBeGreaterThanOrEqual`) and that pagination grows the
   set, rather than an exact global count.

VM capacity is raised to 4 cores / 4 GB so the 4 workers each get a vCPU.

## Consequences

- New specs follow the fixture-first pattern; a spec is added to the parallel
  project by default. Quarantine in a `-serial` project is reserved for genuine
  global-singleton mutation.
- Reaching _full_ isolation later means only de-globalizing `admin-site` (moving
  the title/base_url assertions out of `example`/`posts`, or restore-after) —
  every other spec is already isolated.
- The local `end2end/playwright.config.ts` is intentionally left diverging;
  deduping the two configs is #153.
- Only `nixPlaywrightConfig` (the gate-relevant config) and the spec/fixture
  files change; the change set is small because most specs already minted unique
  users via `register()`.
