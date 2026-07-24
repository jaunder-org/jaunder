# Plan — #609: consolidate the e2e assertion (`expect`) timeout

**Spec:**
`docs/superpowers/specs/2026-07-24-issue-609-expect-timeout-consolidation.md`
(the what/why; this plan is the how). **For agentic workers:** drive with
`jaunder-iterate`.

## Review header

**Goal.** Raise the flat global `expect` timeout `5000 → 10000` in
`end2end/playwright.config.ts` and delete the now-redundant per-assertion 10 s
`expect` overrides — eliminating the `admin-site` base_url flake (spec §Problem)
as a special case of an un-consolidated global.

**Scope — in:** the config bump + the enumerated 11-site override removal
(TypeScript under `end2end/` only). **Scope — out:** the action/nav
`{ timeout: 10_000 }` waits (`waitForSelector`/`waitForURL`/`locator.waitFor` —
different category, intentional fast-fail guards); deliberately-distinct
`expect` values (5 s, 15 s, scaled); `retries`/workers/project-split;
`admin-site` product code. No new separable concerns → no issue-filing task.

**Tasks (one line each):**

1. Raise the global `expect.timeout` to `10000` (with a comment).
2. Strip the 11 enumerated redundant 10 s `expect` overrides.
3. Verify: `cargo xtask check` green; full e2e (`validate`) green incl.
   sqlite/firefox.

**Key risks / decisions:**

- **Order matters:** Task 1 (raise global) precedes Task 2 (strip overrides) so
  the suite is never in a state where a stripped assertion falls back to the old
  5 s.
- **No Rust / no TDD red-green.** The per-commit gate (`check --no-test` +
  `validate --no-e2e`) type-checks the TS (`tsc`) but does **not** run the
  browser e2e; the behavioral proof (sqlite/firefox `admin-site` passes) is the
  **ship-time `cargo xtask validate`** (all four combos) / CI, not a per-commit
  step.
- **Can't prove the flake gone by reproduction** (intermittent, VM+firefox-only;
  host `e2e-local` is chromium/workers=1). Confidence = diagnosis + convention +
  full gate (spec §Verification).

## Global constraints

- **TypeScript-only**, under `end2end/`. Do not touch Rust crates or
  `flake.nix`.
- **Gate = `cargo xtask check`** (includes `tsc` + `prettier`); run it before
  committing (`jaunder-commit`) so the pre-commit hook passes clean. **No
  `Co-Authored-By`.**
- **Keep** every non-`expect` timeout and every non-10 000 `expect` timeout (see
  the "Keep" list in Task 2). Removing an action/nav `{ timeout: 10_000 }` would
  change behavior (`actionTimeout: 0` → it would wait the whole-test budget), so
  don't.

---

## Task 1 — Raise the global `expect` timeout

**File:** `end2end/playwright.config.ts`

**Change** line 42 `expect: { timeout: 5000 },` → `10000`, with a comment:

```ts
// 10s (not the Playwright 2s / our old 5s default): this CSR app populates fields
// from async config/data fetches, which on the slowest combo (firefox+sqlite) can
// exceed 5s — the #609 admin-site base_url flake. 10s is the value assertions across
// the suite already bumped to by hand; making it the default lets those overrides go
// (see the removed set). A real assertion failure now surfaces in 10s, bounded by the
// browser-scaled 30s whole-test budget (slowBrowserTimeoutMs / DEFAULT_TEST_BUDGET_MS).
expect: { timeout: 10000 },
```

**Verify:** read the config — `expect: { timeout: 10000 }`. `cargo xtask check`
green (`tsc`/`prettier`).

**Commit:** `test(e2e): raise the global expect timeout to 10s (#609)`

## Task 2 — Strip the redundant 10 s `expect` overrides

Remove the `{ timeout: 10_000 }` option (or, for `expectFlash`, the trailing
`10_000` arg) at **exactly these 11 sites** — each defaults to the global
`expect.timeout` once the option is gone, so behavior is unchanged (still 10 s):

| #   | File                     | Line(s) | Form                                                            |
| --- | ------------------------ | ------- | --------------------------------------------------------------- |
| 1   | `password_reset.spec.ts` | 24-26   | `expect(...).toContainText(/…/, { timeout: 10_000 })`           |
| 2   | `password_reset.spec.ts` | 41      | `expect(...).toBeVisible({ timeout: 10_000 })`                  |
| 3   | `invite.spec.ts`         | 103-105 | `expect(...).toBeVisible({ timeout: 10_000 })`                  |
| 4   | `invite.spec.ts`         | 125-127 | `expect(...).toBeVisible({ timeout: 10_000 })`                  |
| 5   | `posts.spec.ts`          | 384-389 | `expect(...).toHaveCount(N, { timeout: 10_000 })`               |
| 6   | `posts.spec.ts`          | 390-392 | `expect(...).toContainText("…", { timeout: 10_000 })`           |
| 7   | `posts.spec.ts`          | 431-435 | `expect.poll(…, { timeout: 10_000 }).toBeGreaterThanOrEqual(…)` |
| 8   | `posts.spec.ts`          | 441-445 | `expect.poll(…, { timeout: 10_000 }).toBeGreaterThan(…)`        |
| 9   | `posts.spec.ts`          | 483-488 | `expect(...).toHaveCount(N, { timeout: 10_000 })`               |
| 10  | `email.spec.ts`          | 18      | `expectFlash(page, "Check your email", 10_000)` → drop the arg  |
| 11  | `fixtures.ts`            | 340     | `expectFlash(page, "Check your email", 10_000)` → drop the arg  |

Notes:

- **`expect.poll` (7, 8):** its `timeout` **defaults to the global
  `expect.timeout`** (Playwright), so dropping the option keeps 10 s. Preserve
  the surrounding concurrency-rationale comments — only the redundant
  `{ timeout: 10_000 }` goes.
- **`expectFlash` (10, 11):** `helpers.ts:242-249` — omitting the arg makes it
  pass `{}`, i.e. the global. `expectFlash(page, "verified")` calls (no arg)
  already do this; these two just match them.
- Line numbers will shift as edits land — match on the assertion text, not the
  number.

**Keep (do NOT touch) — same `10_000`, different category, or different value:**

- Action/nav waits:
  `waitForSelector`/`waitForURL`/`locator.waitFor({ timeout: 10_000 })` (some
  spelled `10000`) — `auth.spec.ts` (waitForURL ×3), `feeds.spec.ts:192`,
  `media.spec.ts:85,105` (`locator.waitFor`, `10000`), `invite.spec.ts:74,77`,
  `helpers.ts:183,185`, `password_reset.spec.ts:47`.
- Different `expect` values: `admin-site.spec.ts:99` (`5_000`, deliberate fast
  client-side validation), `atompub` (`15_000`), any
  `slowBrowserTimeoutMs(...)`-scaled.

**Verify:**

- Read each of the 11 sites — the redundant option/arg is gone; the assertion is
  otherwise unchanged.
- Multi-line-aware sweep returns none: search `end2end/tests/` with `rg -U` (or
  ast-grep) for a `{ timeout: 10_000 }` attached to an `expect(`/`expect.poll(`
  assertion, and for `expectFlash(…, 10_000)`. (A plain single-line
  `rg 'timeout: 10_000'` will still hit the KEPT action/nav waits — that's
  expected; filter to `expect`/`expectFlash` context.)
- `cargo xtask check` green.

**Commit:**
`test(e2e): drop redundant 10s expect overrides now covered by the global (#609)`

## Task 3 — Verify behaviorally

**Steps:**

- **AC2 explicit read-check:** read `admin-site.spec.ts` around the base_url
  round-trip (the `toHaveValue` assertions, ~lines 63-76) — confirm they carry
  **no** sub-10 s `expect` override, so they now inherit the raised 10 s global.
  (This is the assertion the whole change exists to fix; verify it directly, not
  only transitively.)
- `cargo xtask check` green (final; `tsc`/`prettier`/`leptosfmt` clean, tree not
  left dirty by a formatter — `git status --porcelain`).
- **Host sanity (chromium):** `cargo xtask e2e-local admin-site` — confirms the
  admin-site spec still passes end-to-end with the new timeout
  (chromium/workers=1; it can't exercise the firefox flake but proves no
  breakage).
- **All-combos proof is the ship gate:** `cargo xtask validate` (run once at
  ship — spec Acceptance 5) exercises `{sqlite,postgres}×{chromium,firefox}`;
  the load-bearing check is `sqlite/firefox admin-site` green. CI re-runs it
  regardless.

**No commit** unless a formatter auto-fixed (then fold into the owning task's
commit).
