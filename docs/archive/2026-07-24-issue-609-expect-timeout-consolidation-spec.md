# Spec — #609: consolidate the e2e assertion (`expect`) timeout, fixing the admin-site base_url flake

**Status:** awaiting approval. **Milestone:** Test infrastructure & E2E (#6).
**Originating issue:** #609 (flaky `admin-site.spec.ts` base_url round-trip on
sqlite/firefox). Scope was **broadened during the design interview**
(maintainer's call) from "patch the one flaky assertion" to "consolidate the
assertion-level timeout," because the flake is a symptom of an un-consolidated
global.

## Problem

`admin-site.spec.ts`'s "site base URL round-trips…" test intermittently times
out on **sqlite/firefox only**: after save→reload,
`expect(base_url_input).toHaveValue(…)` finds the field still empty past the **5
s** `expect` timeout, because this CSR app populates the field from an async
config fetch that, on the slowest combo, can take longer than 5 s. A re-run
passes — so the field _does_ populate; it's slow, not broken.

**The issue's stated root cause (parallel contention) is overstated, not the
primary cause.** `admin-site` runs in the **serial `*-admin` Playwright
project** (`fullyParallel: false`, `dependencies: [<main>]`,
`playwright.config.ts:74-100`), which **limits** contention — but
`fullyParallel: false` only serializes tests _within a file_, so at `workers=2`
`admin-site.spec.ts` can still co-run with the other file in that project
(`invite.spec.ts`). So concurrency is reduced, not eliminated. Either way, the
operative cause is the flat, un-scaled global `expect: { timeout: 5000 }`
(`playwright.config.ts:42`) being too tight for firefox+sqlite async data-load,
and the fix (a generous global) is robust regardless of how much concurrency
remains.

**This is one un-consolidated tier of a two-tier timeout model:**

- **Whole-test + first-navigation timeouts are already consolidated and
  browser-aware** — `fixtures.ts` provides
  `slowBrowserTimeoutMs(testInfo, base)`, `slowBrowserFirstNavigationTimeoutMs`,
  `DEFAULT_TEST_BUDGET_MS = 30_000`, an `auto` fixture that scales every test's
  budget for the slow browser, and a `firstNav` fixture. Firefox automatically
  gets more. This tier is fine.
- **The assertion-level `expect` timeout is NOT consolidated** — it's a flat 5 s
  global, so specs that assert on async CSR data-load bump it ad-hoc to
  `10_000`. `admin-site.spec.ts:64` is the one reload-then-assert-populated site
  that **didn't** bump, and is therefore the flake casualty. Everyone else
  already hand-compensated.

Also outdated in the issue: it claims `retries` is `0`. CI now sets
`JAUNDER_E2E_RETRIES=1` (read at `playwright.config.ts:11-17`, set in
`flake.nix:992`; #624), so the flake is _contained_ (reported `flaky`, exit 0)
unless it double-fails — but it _can_ double- fail (the issue's evidence shows
two hard-failed job runs), so a real fix is still warranted. It also matters for
the **merge queue** just adopted (#627/ADR-0077): a flake that double-fails
ejects a PR from the queue.

## Decisions (interview-resolved)

1. **Raise the global assertion timeout:** `expect: { timeout: 5000 }` →
   **`10000`** in `playwright.config.ts`, with a comment explaining why
   (firefox+sqlite CSR data-load). `10_000` is chosen because it is already the
   **common** bump value the suite reaches for when 5 s is too tight (e.g.
   `password_reset`, `invite`, the `expectFlash` email waits) — making the
   default match established practice rather than inventing a number.
2. **Collapse the now-redundant `expect` overrides equal to `10_000`.** Two
   forms count, both of which become no-ops once the global is 10 s and so are
   stripped:
   - **Direct:** `expect(locator).toX(…, { timeout: 10_000 })` → drop the
     option.
   - **Helper-wrapped:** `expectFlash(page, text, 10_000)` → drop the trailing
     `10_000` arg (the helper defaults to the global when the arg is omitted,
     `helpers.ts:242-249`). The plan **enumerates the exact removal set** by
     reading each candidate site (the syntactic patterns above are the search
     seed, not the authority — a multi-line
     `expect(...\n).toX({ timeout: 10_000 })` must still be caught).
3. **Do NOT touch non-`expect` waits.** `page.waitForSelector(…, { timeout })`,
   `page.waitForURL(…, { timeout })`, and `locator.waitFor({ state, timeout })`
   are **action/navigation** timeouts, not governed by `expect.timeout`. Under
   the deliberate `use: { actionTimeout: 0 }` (`playwright.config.ts:53`) their
   explicit `{ timeout: 10_000 }` is an **intentional fast-fail guard** ("fail
   with a message rather than burn the whole-test budget", per the
   invite/helpers comments), a different purpose. They are out of scope and
   stay.
4. **Keep deliberately-distinct `expect` timeouts.** Overrides whose value is
   **not** `10_000` are intentional and stay: e.g. `admin-site.spec.ts:99`'s
   `{ timeout: 5_000 }` on a fast client-side validation error (tighter on
   purpose), `atompub`'s `15_000` token wait, and any
   `slowBrowserTimeoutMs(testInfo, …)`-scaled assertions.
5. **No per-browser scaling of the global `expect` timeout.** Playwright's
   `expect.timeout` is static config (browser is per-project), so it can't be
   scaled at config load without per-assertion `slowBrowserTimeoutMs` calls
   everywhere — more verbosity, not less. A flat 10 s global that comfortably
   covers firefox is the consolidation. Accepted cost: a genuinely-failing
   assertion that relies on the global now surfaces in 10 s instead of 5 s
   (bounded by the 30 s scaled whole-test budget).

## Acceptance criteria (observable)

1. **The global assertion timeout is 10 s.** `playwright.config.ts` sets
   `expect: { timeout: 10000 }` with an explanatory comment. Verifiable by
   reading the config.
2. **The flaky assertion inherits the raised global.** `admin-site.spec.ts:64`'s
   `toHaveValue` (and the sibling reload-then-assert-value assertions in that
   test) carry **no** sub-5 s override and now get 10 s — verifiable by reading
   the test.
3. **Redundant 10 s `expect` overrides are gone.** Every site in the plan's
   enumerated removal set (both the direct
   `expect(...).toX({ timeout: 10_000 })` and the `expectFlash(…, 10_000)`
   forms) is stripped — verified by reading each listed site, not by a
   single-line grep (which misses multi-line assertions and the helper-wrapped
   form). As a supplementary check, a **multi-line-aware** search (`rg -U` or
   ast-grep) for `{ timeout: 10_000 }` on an `expect`/`toBeVisible` assertion,
   and for `expectFlash(…, 10_000)`, returns none. Deliberately-distinct values
   (5_000, 15_000, `slowBrowserTimeoutMs`-scaled) remain.
4. **Non-`expect` action/nav waits are unchanged.** `waitForSelector` /
   `waitForURL` / `locator.waitFor` `{ timeout: 10_000 }` guards are untouched
   (diff shows no lines removed from those).
5. **The full e2e gate passes**, including `sqlite/firefox` —
   `cargo xtask validate` (all four `{sqlite,postgres}×{chromium,firefox}`
   combos) green.

## Out of scope

- **Consolidating the action/navigation `{ timeout: 10_000 }` fast-fail guards**
  (Decision 3) — a separate, different-purpose cleanup; not folded in.
- **Introducing per-assertion browser-scaled `expect` timeouts** (Decision 5).
- **Changing `retries`, worker counts, or the serial `*-admin` project split** —
  the parallelization (shipped under #61) is not touched.
- **The `admin-site` production code** (`web/src/site/`, `SiteSettingsPage`) —
  the fix is test-timeout only; there is no product bug (the field populates,
  just slowly).

## Verification

- **Static:** read `playwright.config.ts` (global = 10 s) and grep the tests for
  the redundant-override rule (criteria 3/4). `cargo xtask check` green (this
  touches only `end2end/` TS; `tsc`/`prettier` gates apply).
- **Behavioral:** `cargo xtask validate` (full e2e, all combos) green — the
  load- bearing check is that `sqlite/firefox` `admin-site` passes. **Note on
  the flake's intermittency:** it is VM+firefox-only and not reproducible on the
  host (`cargo xtask e2e-local` is chromium/workers=1), and a _single_ green
  `cargo xtask e2e sqlite firefox` can't _prove_ a timing flake gone (it passed
  sometimes before). Confidence rests on the diagnosis (slow load < the raised
  budget), consistency with the suite's existing 10 s convention, and the full
  gate — not on manufacturing a red-then-green. This is stated honestly rather
  than overclaimed.
