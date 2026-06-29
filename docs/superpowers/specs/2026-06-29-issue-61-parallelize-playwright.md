# Spec — parallelize the Playwright e2e suite (#61)

**Issue:** [#61](https://github.com/jaunder-org/jaunder/issues/61) — *e2e: parallelize the Playwright suite (workers>1) — serial-project split + per-test identity isolation* (milestone: E2E test suite, P1).

## Problem

The Nix-VM Playwright config pins `workers: 1` (`flake.nix`, `nixPlaywrightConfig`). The infrastructure precondition — SQLite read-then-write `SQLITE_BUSY` contention — is now resolved (#18 + #51/#52/#53 all closed, `BEGIN IMMEDIATE`). What still forces serial execution is a set of **logical races on shared global state**:

- **Shared seeded accounts** `testlogin` / `testoperator` / `testnoemail` (15+ refs across `auth`, `email`, `password_reset`, `admin-site`).
- **`admin-site.spec`** mutates the singleton `site.title` / `base_url` (`admin-site.spec.ts:25-26`), which `example.spec.ts:9` and `posts.spec.ts:380` assert equals the seeded `"jaunder.local"`.
- **`posts.spec`** asserts the anonymous global feed has **exactly** 50 items (`posts.spec.ts:381-382`) — any concurrent publisher breaks the count.
- **Unfiltered mail capture** — `waitForNewEmail()` (`mail.ts:61`) returns the globally-newest line, so parallel tests grab each other's mail.
- **Ordering dependency** — `password_reset` needs `testlogin`'s email verified, which only `email.spec` does; alphabetical file order makes this true serially, parallel removes the guarantee.

Six specs are already isolated (atompub, feeds, media, visibility, static-assets, unicode-slug — all mint unique users via `register()`). The slowest file, **`posts.spec` (~5.4 min, 24 tests), is unsafe** — so it is the highest-value target.

## Goal

Run the suite with `workers > 1` in the Nix VM, made parallel-safe by a **fixture-first per-test-identity** approach. The one genuinely-global spec (`admin-site`) runs in a serial Playwright project. A healthy parallel run is faster and stable; no shared-account or global-mutation races remain except the deliberately-quarantined `admin-site`.

## Design

### 1. Fixture foundation — `end2end/tests/fixtures.ts`

A shared module exporting an extended `test`/`expect`. Playwright fixtures are lazy (only minted for tests that destructure them) and test-scoped (fresh per test), so the isolation boilerplate lives once here:

- **`user`** → registers a unique account via the existing `register()` helper; yields `{ username, password, email }`.
- **`mailbox`** → a recipient-scoped mail waiter bound to `user.email`. Reads `mail.jsonl`, filters by recipient (`to`), and tracks a per-mailbox cursor so each `waitForNewEmail()` returns *this user's* next unseen message. This is the parallel-safe replacement for the global-newest `waitForNewEmail()`.
- **`verifiedUser`** → `user` plus the email-verification flow driven through `mailbox`. Self-provisions a verified account per test, dissolving both the seeded `testlogin` dependency and the `email → password_reset` ordering coupling.

All specs import `test`/`expect` from `fixtures.ts` instead of `@playwright/test`.

WebSub pings already have a filtered helper (`waitForPingMatching`); specs that wait on pings use it (no global-newest race there).

### 2. Spec migrations

| Spec | Change |
|---|---|
| atompub, feeds, media, visibility, static-assets, unicode-slug | Import swap to `fixtures.ts`; inline `register()` → the `user` fixture. Mechanical, no behavior change. |
| auth | Replace `testlogin`/`testpassword123` with `user` (register/login-form tests) or `verifiedUser` (authed flows). |
| email | Use `user` + `mailbox`; verify its own freshly-registered user. |
| password_reset | Use `verifiedUser` + `mailbox`; self-provision a verified user, request reset, read *its own* reset mail. Removes the shared-password mutation and the ordering dependency. |
| posts | Make the anonymous-feed assertion **own-scoped**: assert *this test's* uniquely-titled/tagged posts appear (locate by its unique slug / `toBeGreaterThanOrEqual`) rather than `toHaveCount(50)` on the global feed. Users → `user` fixture. |
| example | Import swap only. Its `site.title === "jaunder.local"` assertion stays valid because `admin-site` cannot run concurrently (§3). |

### 3. The lone serial exception — `admin-site`

`admin-site` mutates the **singleton** `site.title` / `base_url` that other specs read, so it must not overlap them. It runs in a dedicated **serial Playwright project** sequenced via project **`dependencies`** so it executes in its own phase (after the parallel project completes) — no overlap with the title-asserting specs.

Because the Nix VM runs **one browser per VM** (`--project ${browser}`, per the #129 matrix), the VM invocation passes *both* the browser's parallel project and its serial counterpart; the dependency phases them. The parallel project sets `fullyParallel: true`; the serial project runs `admin-site` only.

This is the single, deliberate serial item — the parallel/serial boundary is one thin line. Reaching full isolation later (if ever) is solely de-globalizing `admin-site` (moving the title/base_url assertions out of `example`/`posts`, or restore-after); everything else is already isolated.

### 4. VM capacity + the gate config

The e2e VM currently has **1 vCPU** (`virtualisation.cores` unset → default 1) and 2 GB RAM, so `workers > 1` would barely help. Changes (both `mkE2eSqliteCheck` and `mkE2ePostgresCheck` `nodes.machine`):

- `virtualisation.cores = 4;`
- `virtualisation.memorySize = 4096;` (4 browser contexts pressure 2 GB).

And in `nixPlaywrightConfig`:

- `fullyParallel: true` (on the parallel project),
- `workers: 4`.

This is the gate-relevant config — CI's matrix uses `nixPlaywrightConfig`, not the local `end2end/playwright.config.ts`.

### 5. Validation — parallel-safety is flaky-shaped

A single green run does not prove parallel-safety. Strategy:

1. **Postgres-VM first** — immune to SQLite write contention — to validate logical isolation in isolation from lock concerns.
2. **Then SQLite** — confirms the #51/#52/#53 `BEGIN IMMEDIATE` fixes hold under real multi-worker writes.
3. **Repeat runs** — loop the parallel suite on the fastest combo several times locally to shake out residual races before trusting it (a one-shot pass is not sufficient evidence).
4. **Final gate** — full `cargo xtask validate` (all four `{sqlite,postgres}×{chromium,firefox}` combos), plus the PR's CI matrix as backstop.

## Scope boundary / follow-ups

- **Not** deduping the two diverging Playwright configs (`nixPlaywrightConfig` vs `end2end/playwright.config.ts`) — that is **#153** (P3). The local config already permits parallelism; this cycle changes only the gate-relevant `nixPlaywrightConfig` and the specs.
- Update the `flake.nix` `workers: 1` comment, which currently frames the pin as purely SQLite-locking — the suite also had these logical races.

## ADR

New ADR: **e2e parallelism via per-test identity fixtures + a serial project for global-singleton specs** — records the fixture-first isolation pattern and the single-serial-exception boundary so future specs follow it. Number = next after the current highest; add its row to the `docs/README.md` ADR table.

## Acceptance

- [ ] `nixPlaywrightConfig` runs `workers: 4` / `fullyParallel: true`; the e2e VM has 4 cores / 4 GB.
- [ ] `fixtures.ts` provides `user` / `mailbox` (recipient-scoped) / `verifiedUser`; all specs import from it.
- [ ] No spec depends on a shared seeded account or on another spec's ordering; `posts`' feed assertion is own-scoped.
- [ ] `admin-site` runs in a serial project that does not overlap the parallel specs.
- [ ] Full `cargo xtask validate` green, validated Postgres-first then SQLite, with repeated parallel runs showing stability.
- [ ] ADR written + `docs/README.md` row added.
