# Plan — #202: empirical CLS assertion for the authed-owner flash

Spec:
[`2026-07-24-issue-202-cls-flash-assertion.md`](../specs/2026-07-24-issue-202-cls-flash-assertion.md)

**For agentic workers:** execute with `jaunder-iterate`. Tick checkboxes in real
time.

---

## Review header

**Goal.** Add one deterministic Playwright spec,
`end2end/tests/authed-cls.spec.ts`, that empirically asserts the owner's
own-post content does **not** shift when the additive `.j-post-acts` action
column is added at wasm-mount — the bounded, deliberately-unreserved case the
#181/ADR-0044 CSS comment names as the deferred #202 follow-up. No product code.

**Scope.** _In:_ one new e2e spec file (+ a small local sampling helper inside
it). _Out:_ product code (`web`/`csr`/`server`), the #181 structural guards, the
sidebar, any `setTimeout` gating.

**Tasks.**

- [ ] 1. Write the reusable helper `end2end/tests/layout-shift.ts`
     (`expectNoShiftAcrossMount(page, testInfo, probe)`): wasm-hold route +
     `fonts.ready` gate + before/after `boundingBox` sampling +
     `waitForHydration`
     - optional `afterMount` hook + per-call `tolerancePx` threshold. The
       invariant machinery, page-agnostic.
- [ ] 2. Write the thin spec `end2end/tests/authed-cls.spec.ts` — `register()` +
     seed post, author-scoped `.j-post-head`/`.j-post-body` targets,
     `.j-post-acts` `afterMount` assertion, `tolerancePx: 0`. Iterate on the
     host runner until green.
- [ ] 3. Full-matrix verify + finalize threshold — `cargo xtask validate`;
     confirm Playwright `flaky` = 0 on all four combos; pass a documented
     per-axis `tolerancePx` (or a `<1px` compare) only if a browser shows
     sub-pixel noise; if a _material_ shift appears, HALT and report (do not
     tolerance-hide).

**Key risks / decisions.**

- **Determinism is the whole point:** gate only on `page.route` release +
  `body[data-hydrated]` + `document.fonts.ready` — never a timer. Register the
  route **before** `goto`.
- **Threshold:** exact-0 v1; loosen a specific axis/browser to `<1px` only with
  a code comment citing the observed value (spec §Threshold; user decision
  "start exact, loosen on evidence").
- **A material shift is a real finding**, not a tolerance problem — Task 3 halts
  and reports it rather than widening the threshold.
- **No new shared selectors:** `.j-post`, `.j-post-head`, `.j-post-body`,
  `.j-post-acts` are raw CSS locators (as `authed-flash.spec.ts` already uses).
  Note `SEL.postBody` is the _composer textarea_ — do **not** use it for the
  rendered body; measure `.j-post-body`.
- **Author-scope the measured post:** `/` has many `.j-post` articles; scope to
  the owner's own via the `@username` handle (present in the anon paint →
  phase-stable), or the check passes vacuously under `workers>1`. This forces
  `register()` (for the username) over the `registeredPage` fixture.
- No separable follow-on concerns surfaced → no issue-filing first task.

---

## Global constraints

- Touch only `end2end/tests/layout-shift.ts` (Task 1) + `authed-cls.spec.ts`
  (Task 2). Zero product-code diff.
- No `setTimeout`/fixed-delay gating anywhere.
- Match the existing spec conventions (`authed-flash.spec.ts`): `register()` +
  `testInfo`, `createPostViaApi`, `waitForHydration`,
  `slowBrowserTimeoutMs(testInfo, budget)` (from `./fixtures`) for the
  post-mount visibility assertion, raw CSS locators.
- The e2e specs are not covered by `cargo xtask check` (host static/clippy only)
  — they run only in the Nix VM e2e (`validate`) or the host runner
  (`e2e-local`). Per-commit gate is therefore `cargo xtask check` (proves
  nothing broke host-side + fmt/prettier/tsc clean); the **behavioral** gate is
  Task 3's `validate` (with the Task 2 host `e2e-local` iteration as the fast
  inner loop).
- No `Co-Authored-By` trailer.

---

## Task 1 — Reusable helper `end2end/tests/layout-shift.ts`

**Files:** `end2end/tests/layout-shift.ts` (new). The page-agnostic
mount-transition machinery, so per-page CLS checks are thin.

```ts
import {
  expect,
  type Page,
  type Locator,
  type TestInfo,
} from "@playwright/test";
import { waitForHydration } from "./hydration";

export interface MountShiftProbe {
  url: string;
  /** Elements to measure, resolved on `page` after goto (present at first paint). */
  targets: (page: Page) => { name: string; locator: Locator }[];
  /** Optional: assert the mount actually decorated the measured content (not a
   *  no-op), e.g. the owner action column appeared. Runs after hydration. */
  afterMount?: (page: Page) => Promise<void>;
  /** Max allowed |Δ| per axis, px. 0 = exact (default). A caller loosens with a
   *  comment citing the observed value + browser ("start exact, loosen on evidence"). */
  tolerancePx?: number;
}

/** Asserts each target's top-left does not move across projector-paint → wasm-mount.
 *  Deterministic: gates only on the wasm-route release + `body[data-hydrated]` +
 *  `document.fonts.ready` — never a timer. */
export async function expectNoShiftAcrossMount(
  page: Page,
  _testInfo: TestInfo,
  probe: MountShiftProbe,
): Promise<void> {
  const tol = probe.tolerancePx ?? 0;

  // Hold the wasm so init() can't complete → projector first paint stays frozen.
  // NOTE: page.route also disables Playwright's HTTP cache for this URL, forcing a
  // fresh, holdable request even if the wasm was already warmed. Do not remove — it
  // is what makes the pre-mount sample deterministic.
  let releaseWasm!: () => void;
  const held = new Promise<void>((r) => (releaseWasm = r));
  await page.route("**/pkg/jaunder*.wasm", async (route) => {
    await held;
    await route.continue();
  });
  try {
    await page.goto(probe.url, { waitUntil: "domcontentloaded" });
    await page.evaluate(async () => {
      await document.fonts.ready;
    }); // settle metrics
    const targets = probe.targets(page);
    const before = await Promise.all(
      targets.map((t) => t.locator.boundingBox()),
    );

    releaseWasm();
    await waitForHydration(page);
    if (probe.afterMount) await probe.afterMount(page);
    const after = await Promise.all(
      targets.map((t) => t.locator.boundingBox()),
    );

    targets.forEach((t, i) => {
      const b = before[i],
        a = after[i];
      expect(b, `${t.name} missing before mount`).not.toBeNull();
      expect(a, `${t.name} missing after mount`).not.toBeNull();
      expect(Math.abs(a!.x - b!.x), `${t.name} x shift`).toBeLessThanOrEqual(
        tol,
      );
      expect(Math.abs(a!.y - b!.y), `${t.name} y shift`).toBeLessThanOrEqual(
        tol,
      );
    });
  } finally {
    await page.unroute("**/pkg/jaunder*.wasm");
  }
}
```

(`toBeLessThanOrEqual(0)` **is** exact equality for the v1 default, so the same
code path serves both exact and a loosened `tolerancePx` — no separate branch.)

**Check (commit gate):** `cargo xtask check` — host static + fmt/prettier/tsc
clean (does NOT run the e2e). The helper has no caller yet; that is fine (a
`.ts` module, not dead Rust) — Task 2 adds the caller in the same branch before
any verify.

**Commit:** `test(e2e): reusable mount-transition layout-shift helper (#202)`
via `jaunder-commit`.

**Done when:** `layout-shift.ts` exports `expectNoShiftAcrossMount` with the
probe seam above (no timers; gates on route release + `data-hydrated` +
`fonts.ready`); `cargo xtask check` green (fmt/tsc clean).

---

## Task 2 — Thin spec `end2end/tests/authed-cls.spec.ts`

**Files:** `end2end/tests/authed-cls.spec.ts` (new). #202's concrete case, built
on the Task 1 helper.

```ts
import { test } from "./fixtures";
import { register, slowBrowserTimeoutMs } from "./fixtures"; // CONFIRM both exported here
import { expect } from "@playwright/test";
import { createPostViaApi } from "./posts";
import { expectNoShiftAcrossMount } from "./layout-shift";

test("authed owner: own-post action column is additive (no content shift)", async ({
  page,
}, testInfo) => {
  // register() (NOT the registeredPage fixture) so we get the username to scope to
  // the owner's own post among many on `/`. CONFIRM register's signature + return.
  const user = await register(page, "/");
  await createPostViaApi(page, { body: "cls probe" }); // short → no wrap/reflow

  // Owner's own post, scoped by author handle (@username) — present in the
  // anonymous projector paint, so STABLE across both phases and safe under
  // workers>1 (a concurrent test's post can't match this username).
  const ownPost = (p: typeof page) =>
    p.locator(".j-post", {
      has: p.locator(".j-post-handle", { hasText: `@${user.username}` }),
    });

  await expectNoShiftAcrossMount(page, testInfo, {
    url: "/",
    targets: (p) => [
      { name: "post-head", locator: ownPost(p).locator(".j-post-head") },
      // rendered body div (NOT SEL.postBody = composer textarea):
      { name: "post-body", locator: ownPost(p).locator(".j-post-body") },
    ],
    afterMount: async (p) => {
      await expect(ownPost(p).locator(".j-post-acts")).toBeVisible({
        timeout: slowBrowserTimeoutMs(testInfo, 10_000),
      });
    },
    tolerancePx: 0, // exact; loosen per-axis only on documented evidence (Task 3)
  });
});
```

Confirm before finalizing (read the harness): `register`'s signature + return
shape (does it return `{ username }` — `authed-flash.spec.ts` uses it this way);
that `register`/`slowBrowserTimeoutMs` are exported from `./fixtures`; the
`createPostViaApi` short-body arg; and that `.j-post-handle` text is exactly
`@${username}` (`render.rs:208`).

**Iterate (host runner, fast):** `cargo xtask e2e-local authed-cls` (~3 min,
auto-seeds testoperator; chromium host path). Fix until green — confirms the
helper + spec end-to-end on chromium.

**Check (commit gate):** `cargo xtask check` (fmt/prettier/tsc clean).

**Commit:**
`test(e2e): empirical CLS assertion for authed-owner post action column (#202)`
via `jaunder-commit`.

**Done when:** the spec exists and calls `expectNoShiftAcrossMount` with the
author-scoped head/body targets + `.j-post-acts` `afterMount` assertion +
`tolerancePx: 0`; `cargo xtask e2e-local authed-cls` green on the host.

---

## Task 3 — Full-matrix verify + finalize threshold

**No file change unless a documented loosening is required.** Ship/behavioral
gate.

**Run:** `cargo xtask validate` (foreground/background per length — all four
`{sqlite,postgres}×{chromium,firefox}` VM e2e combos).

**Evaluate:**

- Inspect the Playwright output for `authed-cls.spec.ts` on each combo. "Green"
  = passed **and** `flaky` count 0 (the gate runs `JAUNDER_E2E_RETRIES=1`, so a
  retry-pass is reported `flaky` with exit 0 — a retry-pass means
  non-deterministic and is NOT acceptable here).
- If a combo shows a **sub-pixel** diff (< 1px) on an axis, pass a documented
  `tolerancePx` (e.g. `1`) in the spec's probe with a comment citing the
  browser + observed value; fixup into Task 2's commit; re-run. (Per-call, so it
  doesn't slacken other pages that reuse the helper.)
- If a combo shows a **material** shift (≥ 1px, a real reflow), **HALT and
  report** — the own-post action column is not purely additive for that case; a
  finding about the flash-free guarantee, not a threshold to widen.

**Done when:** `cargo xtask validate` is green with the spec on all four combos,
Playwright `flaky` = 0, and any loosening is applied + documented (or a material
shift is reported to the user rather than hidden).

---

## Self-review

- Every spec AC maps to a task: AC1 → T1 (helper machinery) + T2 (the spec that
  uses it); AC2/AC3 → T2 (targets + `afterMount` assertion); AC4 → T1/T2 scope
  (test-only); AC5 → T3 (matrix + flaky=0 + documented `tolerancePx`).
- The helper (T1) is shaped by this one use + the {url, targets, auth,
  afterMount} variation axis — a genuine seam for future pages, not a
  speculative framework.
- No task touches product code or the sidebar. No separable concern to file.
- T2 is host-verifiable (`e2e-local`); T3 is the definitive cross-browser gate.
