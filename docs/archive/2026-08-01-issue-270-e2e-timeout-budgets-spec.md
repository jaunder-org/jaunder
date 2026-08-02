# Spec — #270: right-size e2e timeout budgets

## The issue's stated method is unsound

#270 says: "Compare each test's real duration to its budget … Lower
over-provisioned budgets." Measured duration is the wrong quantity, and acting
on it alone breaks tests.

A whole-test budget's job is not to bound the happy path. It is to leave room
for the test's **internal deadlines** — timeouts the test passes to polling
helpers, which are raw constants that do **not** scale with browser or worker
count. When the whole-test budget is the smaller of the two it preempts the
internal one: the failure message degrades from a diagnostic
(`feed <url> never contained "X" within 25000ms; last body: …`) to a bare
whole-test timeout, and eventual-consistency lag the test was written to
tolerate now reds the run.

**`feeds.spec.ts:179` proves it.** Its body loops `for (const fmt of FORMATS)`
(`:208`), `FORMATS` has 3 entries (`:16-20`), and each iteration calls
`fetchFeedContaining` twice (`:211`, `:228`), each defaulting to
`timeoutMs = 25_000` (`:31`). 3 × 2 × 25 s = **exactly the 150 s budget**. It is
derived, not arbitrary — and a duration-driven pass would have cut it to 60 s.

Sizing rule: **budget ≥ worst-path sum of intentional waits, plus setup
margin**, with measured duration as a cross-check that the result is not below
observed reality — never as the source of the number.

## Method and its limits

**Intentional wait** = an awaited operation whose author chose a hard literal
deadline in order to tolerate eventual-consistency lag — poll loops and
`waitForTimeout`. Distinguish it from an **assertion/navigation deadline**
(`waitForSelector`, `waitFor`, `expect.poll`, `Promise.race`, `register`'s
first-nav), which resolves the moment the condition holds. Both are charged to
the budget when consumed; only the former is _expected_ to be consumed.
(`fetchFeedContaining` also early-exits, but its 25 s exists precisely to absorb
worker lag, which is what makes it intentional.)

**Durations** (cross-check only) come from existing CI artifacts — no local e2e
run. CI uploads them per combo (`e2e-diagnostics-<backend>-<browser>`, 14-day
retention); `flaky.rs:37` reads the same file. Sample: **19 green CI runs × 4
combos = 76 reports**, 2026-07-29 → 08-01, at `workers=2`. The "Measured" column
below is **the max over those 76 reports**, per browser.

Two populations, kept distinct because they get used for different things:

- **Suite-wide** (all 122 tests, including those with no explicit budget): worst
  chromium **24 228 ms**, worst firefox **34 599 ms** — both
  `visibility.spec.ts :: Private post…`. p99 20 314 / 26 830 ms. This is what
  sizes the _ambient_ default and what the A10 code comment cites.
- **Among the 20 budgeted sites** (the table below): worst chromium 23.2 s
  (`feeds:245`), worst firefox 32.4 s (`visibility:112`). This is what the
  delete decisions are checked against.

Three stated limits:

- **Green runs only**, so the sample excludes the slow tail. It bounds the happy
  path; it cannot bound the worst case. That is why it is a cross-check, not the
  instrument.
- **Ratios are computed at `workers=2`.** See the accepted risk below.
- **Retry cost is not captured.** An assertion that retries 3 s and then passes
  is a success charged to the budget. Tests opening extra contexts via
  `tracedContext()` each pay a cold-WASM first navigation. Neither the
  intentional-sum nor a green-run duration bounds this; the measured column
  absorbs it only to the extent it occurred in the sample.

Scaling at `workers=2`: `slowBrowserTimeoutMs` = `N × max(browserScale, 1.5)`,
where browserScale is 1.0 **only** when `project.name === "chromium"` exactly,
else 2.2. `chromium-admin` / `firefox-admin` match
`/(admin-site|invite)\.spec\.ts/` (`playwright.config.ts:82-83`, `:100-101`), so
**`invite.spec.ts` gets 2.2× on both**. Ambient 30_000 ⇒ effective **45 s**
chromium / **66 s** elsewhere.

## Per-site findings (all 20)

"Covers" = `min(45 s ÷ measured-chromium, 66 s ÷ measured-firefox)` for
zero-intentional rows; for the four non-zero rows both `ambient ÷ intentional`
and that ratio are given.

| Site                     | Budget |                Intentional | Measured ch/ff (max) | Ambient covers?                  | Action                    |
| ------------------------ | -----: | -------------------------: | -------------------- | -------------------------------- | ------------------------- |
| `feeds.spec.ts:179`      |  150 s |       **150 s** (3×2×25 s) | 7.6 / 11.5 s         | **no** — 0.30× intentional       | **keep, derive + margin** |
| `feeds.spec.ts:245`      |   90 s |         **82 s** (40+2+40) | 23.2 / 25.9 s        | **no** — 0.55× intentional       | **keep, name constants**  |
| `visibility.spec.ts:290` |   90 s | 25 s (`Date.now()+25_000`) | 11.5 / 16.9 s        | 1.8× intentional, 3.9× measured  | delete                    |
| `invite.spec.ts:35`      |   45 s |    5 s (`waitForNewEmail`) | 14.8 / 27.5 s        | 13.2× intentional, 2.4× measured | delete                    |
| `visibility.spec.ts:112` |   60 s |                          0 | 21.1 / 32.4 s        | **2.04×** — tightest             | delete                    |
| `visibility.spec.ts:161` |   60 s |                          0 | 17.2 / 27.7 s        | 2.4×                             | delete                    |
| `visibility.spec.ts:217` |   90 s |                          0 | 16.7 / 26.3 s        | 2.5×                             | delete                    |
| `audiences.spec.ts:30`   |  120 s |                          0 | 17.0 / 23.4 s        | 2.6×                             | delete                    |
| `audiences.spec.ts:218`  |  120 s |                          0 | 12.2 / 17.6 s        | 3.7×                             | delete                    |
| `audiences.spec.ts:258`  |  120 s |                          0 | 6.5 / 10.1 s         | 6.5×                             | delete                    |
| `audiences.spec.ts:281`  |  120 s |                          0 | 12.1 / 17.1 s        | 3.7×                             | delete                    |
| `audiences.spec.ts:321`  |   60 s |                          0 | 6.3 / 10.3 s         | 6.4×                             | delete                    |
| `atompub.spec.ts:48`     |   60 s |                          0 | 8.7 / 18.8 s         | 3.5×                             | delete                    |
| `atompub.spec.ts:82`     |   60 s |                          0 | 8.8 / 17.7 s         | 3.7×                             | delete                    |
| `atompub.spec.ts:105`    |   90 s |                          0 | 7.5 / 10.3 s         | 6.0×                             | delete                    |
| `feeds.spec.ts:56`       |   60 s |                          0 | 7.1 / 11.4 s         | 5.8×                             | delete                    |
| `feeds.spec.ts:119`      |   60 s |                          0 | 6.6 / 9.9 s          | 6.7×                             | delete                    |
| `feeds.spec.ts:292`      |   60 s |                          0 | 2.8 / 4.6 s          | 14.3×                            | delete                    |
| `feeds.spec.ts:320`      |   60 s |                          0 | 3.0 / 4.1 s          | 15.1×                            | delete                    |
| `posts.spec.ts:800`      |   60 s |                          0 | 12.2 / 18.5 s        | 3.6×                             | delete                    |

**16 of 20 have no intentional wait at all** — those budgets were never covering
anything. `visibility.spec.ts:112` is the tightest cut (2.04×) and the row a
reviewer should overrule first if they want one kept.

## Accepted risk: `workers=1`

Every ratio above is at `workers=2`. At `workers=1` the contention scale is 1.0
(`fixtures.ts:118`), so chromium's ambient is **30 s, not 45 s** — and
`cargo xtask e2e-local` defaults to 1 worker (`e2e_local.rs:174`) against the
**debug** wasm build, slower than anything measured here. The cold combos
(`flake.nix:964-975`) also run at 1 worker.

**Neither is a gate.** `nix eval .#checks` lists only the four warm combos plus
`e2e`/`e2e-elisp-integration`; the cold variants are _packages_, built on
demand. So the exposure is a slower dev loop and a diagnostic tool, not CI.

We accept it deliberately: the heaviest three or four tests (`visibility:112`,
`:161`, `:217`, `audiences:30`) may sit near 30 s under `e2e-local`. If that
surfaces, the signal is a whole-test timeout on those specs alone, and the
remedy is to re-add a deliberate budget or run `JAUNDER_E2E_WORKERS=2`. A code
comment records this so the next reader does not have to re-derive it (A10).

## Decisions

1. **Delete 18 `setTestBudget` sites; keep 2** (`feeds.spec.ts:179`, `:245`).
2. **Derive `feeds:179`'s budget in code, with margin.** Extract the poll
   default to a named constant and compute from it and `FORMATS.length` — but
   **not** as the bare worst-path sum: the body also does 2× `register()` (cold
   WASM), 3× `createPostViaApi`, a logout and a `waitForURL`, and at `workers=1`
   chromium the scale is 1.0, so a budget of exactly 150 s would leave nothing
   for them. Apply a stated margin factor.
3. **Name `feeds:245`'s constants** (two 40 s ping waits, 2 s settle) and
   comment the 82 s sum and the margin over it.
4. **`DEFAULT_TEST_BUDGET_MS` stays `30_000`.** Raising it is
   counter-productive: `test.slow()` **triples the current timeout** (verified
   in Playwright 1.58.2, `timeoutManager.js:99-104`), and it is used at 11 sites
   (`posts.spec.ts` ×9, `unicode-slug.spec.ts` ×2), which today sit at 135 s /
   198 s. A higher ambient inflates the suite's largest budgets while this cycle
   cuts modest ones.
5. **`setTestBudget` survives** (2 callers).
6. **`workerContentionScale` keeps all four rungs.** They are unreachable in
   current configurations, but `JAUNDER_E2E_WORKERS` is a documented override,
   and #155's own history is a wrong contention scale causing silent failures —
   removing the 3/4+ rungs would hand anyone using the override 1.5× instead of
   2.0–2.5×, silently. What actually rotted is the _commentary_:
   `flake.nix:792-794` and `:960-963` describe a "workers=4 gate default" that
   `flake.nix:930` already contradicts ("The warm gate runs at workers=2"). Fix
   the comments, keep the capability.
7. **The `2.2` browser scale stays.** Measured firefox/chromium ratio across 122
   tests: median 1.54×, p95 1.86×, max 2.40× — a real ~p98 safety factor.
8. **No ADR.** #260/#261's ambient-budget design is untouched. But ADR-0012 and
   two docs teach the convention and must be corrected (A7).

## Acceptance criteria (observable)

- A1. Exactly **2** `setTestBudget(` call sites remain under `end2end/tests/`:
  `feeds.spec.ts:179` and `:245`. No spec file retains an unused `setTestBudget`
  import. (Checked by `rg`, not by tooling — `end2end/tsconfig.json` does not
  set `noUnusedLocals`, and `setTestBudget` still exists, so a stale import
  would type-check clean.)
- A2. `feeds.spec.ts`'s poll default is a named module constant, and the `:179`
  budget is **computed** from it, `FORMATS.length`, and an explicit named margin
  — not a literal, and not equal to the bare product. The margin may be additive
  (a named setup allowance) rather than multiplicative; additive is preferred
  here because the margin covers a fixed set of setup steps (two cold-WASM
  registrations, three writes, a logout) whose cost does not scale with the
  number of feed formats. Changing `FORMATS` to 4 entries changes the budget
  without editing the budget expression.
- A3. `feeds.spec.ts:245`'s two ping waits and settle are named constants, with
  a comment stating the 82 s worst-path sum and the margin over it.
- A4. For every deleted site, the ambient effective budget at `workers=2` (45 s
  chromium / 66 s elsewhere; 66 s for `invite.spec.ts` on both projects) is **≥
  1.5× that test's intentional-wait sum**. This half is statically checkable by
  reading the test. The measured column is **committed evidence in this spec**
  (archived with it at ship) and is explicitly _not_ reproducible after the
  14-day artifact retention — it is corroboration, not a gate.
- A5. `DEFAULT_TEST_BUDGET_MS` unchanged at `30_000`; the `2.2` / `2.6` browser
  scales unchanged; `slowBrowserTimeoutMs`,
  `slowBrowserFirstNavigationTimeoutMs` and `_autoTestTimeout` keep their
  behaviour.
- A6. `workerContentionScale` is **unchanged** (all four rungs).
  `flake.nix:792-794` and `:960-963` no longer describe a workers=4 gate
  default, and no longer contradict `:930`.
- A7. Docs no longer teach the removed per-test-budget workflow:
  `helpers.ts:23-27` and `fixtures.ts:10`, `:214`, `:324` describe the two
  surviving budgets and why they exist; `CONTRIBUTING.md:285-289`,
  `docs/observability.md:434` and
  `docs/adr/0012-environment-aware-timeouts.md:30` are corrected.
  **`helpers.ts`'s rule "Do not combine with `test.slow()` — the scaled budget
  already covers Firefox" is preserved verbatim.**
- A8. `devtool run --cwd <worktree> -- cargo xtask check` is green.
- A9. All four e2e combos pass **and** show no timeout regression: in each
  combo's `playwright-report-<backend>.json`, zero
  **`suites[].specs[].tests[].results[].status == "timedOut"`** (note
  `tests[].status` is `expected|unexpected|flaky|skipped` and is never
  `timedOut` — grepping that level gives a vacuous pass), and the sidecar's
  `flaky-scan` step reports `0 flaky test(s)` in its `detail` (the step is
  always `ok`, so its status proves nothing — `flaky.rs:42`).
- A10. A comment at `DEFAULT_TEST_BUDGET_MS` records that the deletions were
  validated at `workers≥2`, that `workers=1` halves chromium's ambient, and what
  to do if `e2e-local` starts timing out.

## Out of scope

- The 11 `test.slow()` sites (135 s / 198 s today) — the suite's largest
  budgets. They are a different mechanism with a different rationale;
  right-sizing them is separate work, not silently folded in here.
- `expect` timeout, `workers`, `retries`; `playwright.config.ts:41`'s `timeout`
  (unchanged because the ambient is unchanged, so its comment stays true).
- `flake.nix`'s `e2ePlaywrightTimeout` / `e2eGlobalTimeout` — settled
  separately.
- A drift gate that fails when a duration approaches its budget — file as a
  follow-up.
- **Making the suite faster — file as a follow-up, with the evidence below.**
  Budgets are ceilings that bite only on failure; nothing in this cycle makes a
  passing test faster, and it would be wrong to imply otherwise. But the traces
  needed to answer "why is the suite slow" already ship in every CI artifact
  (`capture-<backend>.tar.gz` → `capture/otel-traces.jsonl`) and already link
  test → app → backend: `e2e.test` ×121, `action.timed` ×1233, `request` ×4370,
  plus `storage.*` and `crypto.*`. Measured on run 30714621799
  (sqlite/chromium): total `e2e.test` **583 980 ms** against total server
  `request` **42 039 ms** — the server is **7.2%** of wall-clock, median request
  1 ms, `crypto.password.hash` 7.8 s total (1.3%),
  `storage.session.authenticate` 15.5 s (2.7%). So the cost is client-side (WASM
  load/hydration per context, render, selector waiting, explicit sleeps), and
  backend optimisation is not the lever. Recorded here so the finding is
  reviewed rather than living only in a chat log; ADR-0028 deferred
  `analyze-otel-traces` (#32) and `run-e2e-trace-analysis` (#33), which may
  already be its home.
- The interaction between `feeds:179`'s raised budget and the VM-level cap: at
  `workers=2` firefox its effective budget becomes 396 s, and
  `JAUNDER_E2E_RETRIES=1` (`flake.nix:947`) could double that to 792 s against
  `e2ePlaywrightTimeout = 1020` (`flake.nix:587`). Latent only — the test
  measures 11.5 s — and a green run cannot surface it, so it is noted rather
  than addressed.
- `verifiedUser`'s in-fixture `mailbox.waitForNewEmail()` (unscaled 5 s
  intentional wait at `fixtures.ts:382`, `:415`). **No deleted site uses it**,
  but it is invisible to a method that reads only test bodies — recorded here so
  a future ambient-budget test that destructures `verifiedUser` is not silently
  under-counted.

## Verification ladder

- **Static:** `rg -n 'setTestBudget' end2end/tests/` shows exactly 2 (A1); read
  the two kept tests for A2/A3; read the diff for A5–A7, A10.
- **Machine gate:** `devtool run --cwd <worktree> -- cargo xtask check` (A8).
- **e2e:** per-combo `cargo xtask e2e <backend> <browser>` — preferred locally,
  since four concurrent VMs on one box distort durations (observed: a contended
  run blew the 15 min Playwright cap while the same combo passed alone). Then
  parse each report for `results[].status == "timedOut"` and read `flaky-scan`'s
  detail (A9).
