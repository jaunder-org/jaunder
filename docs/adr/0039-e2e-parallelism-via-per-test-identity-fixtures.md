# ADR-0039: Per-test identity fixtures for parallel-safe e2e specs

- Status: accepted
- Date: 2026-06-29 (updated 2026-07-03: `workers=2` flip landed, #155)
- Issue: #61; `workers>1` flip landed via #155 (#173 dissolved by the CSR
  cutover #180)

## Context

The Nix-VM Playwright suite pinned `workers: 1`. The original justification was
SQLite read-then-write `SQLITE_BUSY` contention, resolved by `BEGIN IMMEDIATE`
(#18, #51/#52/#53). What remained before the suite could run with `workers > 1`
were **logical races on shared global state**: shared seeded accounts
(`testlogin`/`testoperator`/`testnoemail`), the singleton
`site.title`/`base_url` that `admin-site` mutates and other specs assert, an
exact global-feed count in `posts`, and a global-newest (unfiltered) mail
waiter.

## Decision

Make the specs **parallel-safe at the test level** via per-test identity, so the
suite carries no shared-state races and a future `workers > 1` flip is a
config-only change:

1. **Per-test identity fixtures** (`end2end/tests/fixtures.ts`): `user`
   provisions a uniquely-named account out-of-band (in a throwaway browser
   context, so the test page stays logged out); `mailbox` is a recipient-scoped,
   cursor-tracked mail waiter bound to that account's unique address;
   `verifiedUser` adds the email-verification flow. Lazy and test-scoped, so
   each test that destructures them gets isolated state and the boilerplate
   lives once. The `auth`, `email`, and `password_reset` specs drop the shared
   seeded accounts in favour of these fixtures.

2. **Own-scoped feed assertions.** `posts`' anonymous local-timeline test
   asserts a full first page (`toBeGreaterThanOrEqual`) and that pagination
   grows the set, rather than an exact global count.

3. **The lone global-singleton spec, `admin-site`**, mutates `site.title` /
   `base_url`. The intended parallel design quarantines it in a per-browser
   **serial Playwright project** sequenced via project `dependencies`.
   `admin-site` keeps the seeded `testoperator` account (operator privilege
   can't be self-served via `register()`), so the quarantine is by config, not
   de-seeding.

New specs follow the fixture-first pattern; a spec needs quarantine only if it
mutates a genuine global singleton.

## Status of the `workers > 1` flip — LANDED at `workers=2` (#155)

The flip is **landed**. The original blocker — **concurrent-SSR
reactive-disposal panics** in the Leptos server (leptos #4590, tracked as #173)
— was dissolved by the leptos-CSR cutover (#180): there is no SSR to race, so
the suite hydrates/mounts client-side and runs cleanly in parallel. The
remaining heavy-timeline-test flake was root-caused and fixed in #210
(batch-seed via `test-support` instead of sequential `api.create_post`).

The landed configuration is **`workers=2`**, chosen over `workers=4` after a
per-VM-footprint sweep (see `docs/observability.md` → "#155 — flip landed"):

- **`cores` must be `≥ workers`** or the guest CPU-starves
  (`workers=4`/`cores=3` was measurably _worse_), so `workers=4` forces 4-core
  VMs. Four 4-core VMs can't pack 4-wide on a 16-core host, so the local
  `validate` aggregate must run them 2-at-a-time; four `workers=2` / `cores=2`
  VMs pack 4-wide with no throttling. (CI runs one combo per runner, so it is
  unaffected either way — ADR-0034.)
- On corrected budgets **both configs are green** (see the budget-bug note
  below); the choice is a balance, not `workers=4` being unfixable. With all
  four combos green, `workers=2` / 4-wide ran the local aggregate _faster_ (8.7
  m vs `workers=4` / 2-wide's 10.8 m — all-at-once beats 2-at-a-time), is less
  bursty on a shared host (2 browser instances/combo vs 4), and needs no
  `--max-jobs` throttling.
- VMs are sized **`cores=2` / 3 GB** with Firefox `firefoxUserPrefs`
  process-slimming (Fission off, single content process, trimmed caches —
  transparent to the app-level tests), keeping peak local RAM ≤12 GB.

**Budget-bug note:** the sweep's early `workers=4` runs were tainted by a
worker-scaling bug — `fixtures.ts` re-read `JAUNDER_E2E_WORKERS` with a default
that diverged from the config, so chromium budgets got zero contention headroom
(Firefox was unaffected — its 2.2× browser scale dominates). Fixed by deriving
the scale from `testInfo.config.workers`; a re-test then ran `workers=4` 71/71
green. Details in `docs/observability.md`.

`workers`/`cores`/`mem` are baked into the shared `e2eWarmChecks` derivation, so
CI's per-combo matrix uses the same values; `workers=2` gives up only ~1 min on
the isolated CI combo vs `workers=4` in exchange for the faster, simpler,
gentler local story — still a large cut from the old ~12 min Firefox long pole.
The `admin-site` serial-project split (its own global-singleton isolation)
landed with the flip.

## Consequences

- The suite runs parallel at `workers=2` (`playwright.config.ts` + the
  `admin-site` serial project + small-VM sizing); #173 is dissolved by the CSR
  cutover (#180), so there is no SSR race to gate on.
- The `auth`/`email`/`password_reset` specs no longer depend on shared seeded
  accounts; `verifiedUser` self-provisions, so those tests are heavier and the
  `password_reset` flow carries a wider per-test timeout.
- The local `end2end/playwright.config.ts` is intentionally left diverging;
  deduping the two configs is #153 (since done: ADR-0051 unified the configs).
