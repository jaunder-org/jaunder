# ADR-0039: Per-test identity fixtures for parallel-safe e2e specs

- Status: accepted
- Date: 2026-06-29
- Issue: #61 (parallelism deferred — blocked by #173)

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

## Status of the `workers > 1` flip — DEFERRED

The actual `workers > 1` flip is **not landed**. Turning it on exposed
**concurrent-SSR reactive-disposal panics** in the Leptos server (_"Tried to
access a reactive value that has already been disposed"_, `reactive_graph`
`traits.rs:394` and `actions/action.rs:945`) — a production-relevant,
upstream-rooted race (leptos #4590, NOT_PLANNED) that the available dependency
fixes (`tachys 0.2.16`'s `OwnedView` deferred drop, the `sandboxed-arenas`
feature) only partially mitigate. It is tracked as **#173**, which blocks #61.

What landed here is therefore only the **parallel-safe prep**: the fixtures, the
spec migrations, and the own-scoped feed assertion. The gate config stays
`workers: 1`; the serial-project split and the VM-capacity bump land with #61
once #173 makes concurrent SSR panic-free.

## Consequences

- The suite is parallel-_safe_ but still runs serially; landing `workers > 1`
  later is a `nixPlaywrightConfig` change plus the `admin-site` serial project,
  gated on #173.
- The `auth`/`email`/`password_reset` specs no longer depend on shared seeded
  accounts; `verifiedUser` self-provisions, so those tests are heavier and the
  `password_reset` flow carries a wider per-test timeout.
- The local `end2end/playwright.config.ts` is intentionally left diverging;
  deduping the two configs is #153.
