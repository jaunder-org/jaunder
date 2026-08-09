# Plan — issue #866: decompose the non-wasm half of `commitToMount`

Spec:
[`docs/superpowers/specs/2026-08-09-issue-866-boot-phase-remainder.md`](../specs/2026-08-09-issue-866-boot-phase-remainder.md).
The spec is "what/why"; this is "how". Referenced by section (D0…D9, A1…A10),
not restated.

**For agentic workers:** drive with **`jaunder-iterate`**; delegate a single
task with **`jaunder-dispatch`** where useful. Tick checkboxes in real time.

---

## Review header

**Goal.** Report where the ~95 s/browser of non-wasm `commitToMount` goes, land
a wasm preload if and only if it measurably helps, and file the three larger
levers this analysis surfaced.

**Scope — in:** the write-up; the preload in the two shells; drift +
double-fetch tests; a pre-registered prediction with floor and abort rule; a
two-arm before/after capture; four filed issues. **Scope — out:** the
caching/hashed-URL lever (D9, filed); frame-skew attribution (D3, filed,
ADR-0100); the stylesheet hypothesis (filed); the e2e protocol or worker count
(ADR-0034/0099).

| #   | Task                                                                     | Gate                          |
| --- | ------------------------------------------------------------------------ | ----------------------------- |
| 1   | File frame skew, the caching lever, the stylesheet hypothesis            | issue numbers exist           |
| 2   | Write the decomposition into `docs/observability.md` (A1–A4)             | `cargo xtask check`           |
| 3   | Failing tests: shell drift + preload/`init()` URL agreement              | tests FAIL                    |
| 4   | Test: exactly one `/pkg/jaunder.wasm` request, proven by mutation        | mutation shows 2, then 1      |
| 5   | Implement the preload in `render_head` + `csr/index.html`                | tests PASS, **both** browsers |
| 6   | Register prediction + floor + D8 abort rule, **commit before capturing** | sha recorded                  |
| 7   | Two-arm interleaved capture, 3 runs × 2 browsers × 2 arms                | corpus certified              |
| 8   | Analyse, write predicted-vs-observed, **apply D8**                       | abort rule stated             |
| 9   | If D8 fired: revert the preload, keep the write-up                       | preload absent, finding kept  |
| 10  | Full `cargo xtask validate` (A10)                                        | green, e2e included           |

**Key risks / decisions.**

- **The fix may not survive its own measurement.** D8 reverts it. Task 9 is a
  legitimate ending, not a failed cycle. The spec's A5 now has an explicit
  revert clause.
- **Firefox — the gate's critical path — gains ≤2.6% (D4).** Lead with that
  (A8).
- **Pre-registration vs interleaving, resolved.** The prediction is _relative_ —
  "after-arm boot total is below before-arm by more than 3×SE and at most D4's
  ceiling" — committed before any capture. No after-arm information leaks; the
  **floor** is what gives it falsifying power (a ceiling alone is confirmed by
  every positive outcome).
- **A double fetch is worse than doing nothing (D6)** and invisible without
  counting. Task 4 precedes task 5, and **proves itself by mutation** rather
  than passing green from birth.
- **Do not add a new e2e navigation** — extend `boot-marks.spec.ts`. A new spec
  that navigates works against #867.

## Global constraints

- No `Co-Authored-By`. Conventional-commit subject **≤72 chars**.
- Pre-commit runs the full `cargo xtask check`; run it first
  (**`jaunder-commit`**). **Never commit a red test** — see task 4's commit
  note.
- Worktree:
  `/home/mdorman/src/jaunder/.claude/worktrees/issue-866-boot-phase-remainder`.
  Pin it: `devtool run --cwd <that path> -- …`. JS in `ctx_execute` runs from
  the MAIN repo — absolute paths.
- Means, never medians. Denominator **208 mounted navigations/run** (of 211),
  stated wherever seconds appear — **including in filed issues** (A4).
- `topSlow` is truncated (D2) — never used for share arithmetic.

### Verified facts the tasks depend on

Established during planning; do not re-derive:

- `web/src/app/render.rs:51` already has
  `include_str!("../../../csr/index.html")`, and `flake.nix` keeps that file in
  the source filter — **the drift guard is feasible in `web`**.
- `render_head` (`render.rs:76-87`) emits `meta charset` → `viewport` → two
  stylesheets. It emits **no** prepaint script and **no** boot script;
  `server/src/projector/mod.rs:87` splices `PREPAINT_SCRIPT` ahead of it and
  emits the boot script itself.
- **`nix build .#site` emits only `pkg/` and `favicon.ico` — no shell** (#239).
  The shell reaches users through the _server binary_. Arm verification must not
  grep `.#site`.
- `web` cannot depend on `server` (the dependency runs the other way), so any
  assertion about the projector's document belongs in
  `server/src/projector/mod.rs`, which already has the pattern at `:402`.

---

## Task 1 — File the deferred levers

**Files:** none (GitHub). Use **`jaunder-issues`** (type via MCP; topic labels;
project #1; priority).

- [x] **Step 1:** File **frame skew** — 61.3 s chromium / 38.4 s firefox per
      suite over 208 mounted navigations, larger on the faster engine. ADR-0100
      forbids decomposing `commitToMountMs`; attribution needs a Node-frame
      instrument. Milestone 2, P2.
- [x] **Step 2:** File the **caching lever** (D9) — no `Cache-Control` on
      `/pkg/*`, ETag only; `server/src/site.rs:186` records **~44% of
      `pkg/jaunder.wasm` requests conditional**. Needs content-hashed URLs
      (`csr_bundle.rs`, both shells, projector, `audit_wasm`, `wasm_budget`,
      server embed). Attach `wasm_fetch` = 33.5 s chromium / 11.8 s firefox
      **per 208 navigations**, and note `topSlow` truncation does not affect
      these (they come from the `navigation_top_json` census). Say plainly it is
      plausibly larger than #866's own fix. Milestone 6, **P2**.
- [x] **Step 3:** File the **stylesheet hypothesis** — two render-blocking
      stylesheets precede the module script; blocking is spec'd but **untested
      here**. Record the FOUC risk and the interaction with the #181 pre-paint
      script. Milestone 2, P3.
- [x] **Step 4:** Replace the spec's Deferred bullets with the three numbers.
      Commit: `docs(spec): file #866's deferred levers (#866)`.

**Filed:** #868 (frame skew, P2), #869 (caching lever, P2), #870 (stylesheets,
P3).

## Task 2 — The decomposition write-up (A1–A4)

**Files:** `docs/observability.md` (modify).

- [ ] **Step 1:** Add `## #866 — where the rest of boot goes` after #836's.
      Six-segment table per engine (s/suite **and** ms/navigation), cold/warm
      split for `document_start → wasm_fetch_start`, the 208-navigation
      denominator, means-not- medians stated.
- [ ] **Step 2:** State both closures — the Rust boot path is ~1 s of a 300–450
      s suite (**#801 confirmed redirected**), and `topSlow` is truncated
      (`resource_top_slow_dropped = 54` over 822 spans) so it carries no shares.
- [ ] **Step 3:** State the ordering **as observed** (pre-paint script →
      stylesheets → inline module script in `<body>` → glue → `init()` → wasm
      fetch); state the stylesheet-blocking claim **as an untested hypothesis**
      with its issue number.
- [ ] **Step 4:** Cross-reference #864 (84.5 s, still the largest single item),
      #867, and task 1's three new issues.
- [ ] Run: `devtool run --cwd <wt> -- cargo xtask check` → PASS.
- [ ] **Step 5:** Commit
      `docs(observability): decompose the non-wasm boot phases (#866)`.

## Task 3 — Failing tests: shell agreement (A5)

**Files:** `web/src/app/render.rs` (in-file `#[cfg(test)]`, beside the existing
`PREPAINT_SCRIPT` guard at ~`:255`); `server/src/projector/mod.rs` (in-file
tests, pattern at `:402`).

- [ ] **Step 1:** Introduce `pub const WASM_URL: &str = "/pkg/jaunder.wasm";`
      and `pub const GLUE_URL: &str = "/pkg/jaunder.js";` in `web::app::render`.
- [ ] **Step 2:** Failing tests in `render.rs`: -
      `render_head_preloads_wasm_and_glue` — output contains `rel="preload"`
      with `href=WASM_URL` and `rel="modulepreload"` with `href=GLUE_URL`. -
      `csr_index_html_preloads_match_render_head` — `SPA_SHELL` contains both. -
      **`csr_index_html_init_target_is_wasm_url`** — `SPA_SHELL` contains
      `init("{WASM_URL}")`. **This is the A5 requirement the first two miss**:
      it ties the preload to the _actual_ `init()` target, not just to another
      copy of the preload. Without it a stale preload silently degrades into a
      double fetch.
- [ ] **Step 3:** Failing test in `projector/mod.rs`:
      `projected_document_emits_each_preload_once` — the rendered document
      contains the preload `href` **exactly once** (it composes `render_head`; a
      second copy here would double-emit), and its boot script's `init(` target
      is `WASM_URL`.
- [ ] **Step 4:** Run `cargo nextest run -p web render` and
      `cargo nextest run -p server projector` → **FAIL**. Do not implement yet,
      and **do not commit** — the pre-commit gate would reject a red tree.

## Task 4 — Exactly one wasm request, proven by mutation (A6, D6)

**Files:** `end2end/tests/boot-marks.spec.ts` (extend; do **not** add a
navigating spec).

- [ ] **Step 1:** Before an existing `goto`, attach `page.on("request", …)`;
      count requests whose **logical path** is `/pkg/jaunder.wasm`, folding
      `.br`/`.gz` variants and `304`s into the same resource (match on path, not
      on status or encoding).
- [ ] **Step 2:** Assert the count is exactly **1** and **name the cache state**
      in the test — a fresh Playwright context starts with an empty HTTP cache,
      so this is the cold path; say so, because it means the `.br`/`304` folding
      branches are not exercised here.
- [ ] **Step 3: prove the test can fail.** Temporarily add a deliberately
      mismatched preload (e.g. `crossorigin` present when the real fetch is
      same-origin), run
      `devtool run --cwd <wt> -- cargo xtask e2e sqlite chromium`, and **confirm
      it reports 2**. Revert the mutation. Record the observed count in the
      commit message. A guard that has never gone red is not a guard.
- [ ] **Step 4:** Also record `site.status` from the server span for the wasm
      request — D6 names it the engine-independent signal (the browsers disagree
      on `transferSize` for revalidated responses, #818), and
      `page.on("request")` alone does not distinguish a 200 from a 304.
- [ ] **Step 5:** Run both `cargo xtask e2e sqlite firefox` and
      `… sqlite chromium` → PASS. Commit **task 4 only** (it is green; task 3 is
      still red): `test(e2e): assert one wasm request per navigation (#866)`.

## Task 5 — Implement the preload (A5)

**Files:** `web/src/app/render.rs`, `csr/index.html`.

- [ ] **Step 1:** In `render_head`, emit immediately **before** the first
      `link rel="stylesheet"` and after `meta name="viewport"` (there is no
      prepaint slot in `render_head` — the projector splices that ahead):
      `link rel="modulepreload" href=(GLUE_URL);`
      `link rel="preload" href=(WASM_URL) as="fetch" type="application/wasm";`
- [ ] **Step 2:** In `csr/index.html`, add the same two links after the `#181`
      pre-paint `<script>` and before
      `<link rel="stylesheet" href="/style/jaunder.css">`. Do not place them
      above `<meta charset>`.
- [ ] **Step 3:** Do **not** touch `server/src/projector/mod.rs`'s markup — it
      composes `render_head` and would double-emit (D5, and task 3 step 3
      asserts it).
- [ ] **Step 4:** `cargo nextest run -p web render` and
      `cargo nextest run -p server projector` → **PASS** (all four from task 3).
- [ ] **Step 5:** Run e2e on **both** browsers —
      `cargo xtask e2e sqlite firefox` **and**
      `cargo xtask e2e sqlite chromium`. A `crossorigin` disagreement is exactly
      the engine divergence D6 anticipates, so one browser is not enough. **If
      either reports 2 requests, fix the attribute — do not relax the test.**
- [ ] **Step 6:** `cargo xtask check` → PASS. Commit tasks 3+5 together (now
      green): `perf(web): preload the wasm off the serial boot chain (#866)`.

## Task 6 — Register the prediction (D7, A7) — **before** any capture

**Files:** `docs/observability.md` (modify).

- [ ] **Step 1:** Pre-register: decisive quantity is
      **`document_start → mount_done`**; expected change is a reduction
      **bounded above** by D4's ceiling (≤161 ms/nav chromium, ≤57 ms firefox),
      measured **relative to this capture's before-arm**; the ceiling will not
      be reached (connection contention; ~44% conditional requests).
- [ ] **Step 2:** Register the **floor**: the improvement must exceed **3 × SE**
      on boot total in at least one engine. Show the MDE arithmetic — firefox
      boot ~727 ms, run SD ~3–9 ms at n=3 → SE ≈ 2–6 ms — and state that suite
      wall-clock cannot resolve this and is context only.
- [ ] **Step 3:** Write **D8's abort rule verbatim**: below the floor, or a
      regression in either engine, or any double fetch → the preload is
      **reverted** and written up as a negative result.
- [ ] **Step 4:** Commit
      `docs(observability): pre-register the #866 preload prediction (#866)`.
      **Record the sha here in the plan.** No capture may start before it.

## Task 7 — The two-arm capture (D7)

**Files:** none committed. Arm variants live **outside** the worktree.

- [ ] **Step 1:** Copy pristine variants to
      `~/measurements/jaunder/issue-866-preload/arms/`: `before` = `render.rs` +
      `csr/index.html` with the preloads removed; `after` = HEAD.
- [ ] **Step 2: verify the arms differ where it counts.** **Do not grep
      `.#site`** — it emits only `pkg/` and `favicon.ico`, so it is
      byte-identical across arms. Instead confirm the two source files differ,
      and confirm the _served_ markup differs by building the e2e/server
      derivation and asserting the shipped shell contains (resp. lacks)
      `rel="preload"`. **Never commit an arm edit.**
- [ ] **Step 3:** Driver script, arms **interleaved**
      (`before-1, after-1, before-2,     after-2, before-3, after-3`),
      single-worker sqlite × both browsers, **distinct `e2eSalt` per run**
      (`flake.nix` ~`:899`), trap restoring HEAD + empty salt on every exit
      path. ~2 × 70 min.
- [ ] **Step 4:** Extract to `~/measurements/jaunder/issue-866-preload/traces/`
      uncompressed (`<arm>-<run>-sqlite-<browser>.jsonl`), plus `reports/`,
      `store-paths.tsv`, `collection.log`.
- [ ] **Step 5: certify before analysing** —
      `cargo xtask traces analyze <files…>`: require `dropped = 0`, full marks
      on 100% of mounted navigations, 0 closure violations.
- [ ] **Step 6: arm integrity in-trace.** #836 could verify arms via
      `wasmDecodedBytes`; here the bytes are identical, so use an
      **independent** signal — the wasm resource's `initiatorType` in
      `e2e.resource_summary_json` should differ between arms (`link`/preload vs
      `fetch`). Do **not** use `wasmFetchStartMs` for this: it is the outcome
      under test, and using it would be circular.
- [ ] **Step 7:** Write the corpus `README.md` on #836's shape, including host
      quiescence and that arm is confounded with position within a round.

## Task 8 — Predicted vs observed, and apply D8 (A7, A8)

**Files:** `docs/observability.md` (modify).

- [ ] **Step 1:** Per engine and warmth: mean boot total per arm, delta, SE over
      three run-means, |d|/SE. Means only.
- [ ] **Step 2:** Predicted vs observed. If observed diverges, **say so and
      leave it unexplained** — no fourth story (#840).
- [ ] **Step 3:** **State explicitly whether the floor was cleared and whether
      D8 fired.** A7 is not satisfied by a table alone.
- [ ] **Step 4:** Lead the fix's headline with **firefox's** number (A8).
- [ ] **Step 5:** Commit `docs(observability): the #866 preload result (#866)`.

## Task 9 — Conditional: revert if D8 fired

**Only if** task 8 shows the floor uncleared, a regression, or a double fetch.

- [ ] **Step 1:** Revert task 5's commit and task 3's now-vacuous preload
      guards. **Keep** task 4's single-request test — it guards the property
      regardless of how the fetch is initiated (spec A6) — and keep all
      write-ups.
- [ ] **Step 2:** File an issue recording the negative result with the measured
      numbers, so the preload is not re-proposed from first principles.
- [ ] **Step 3:** Commit
      `revert(web): drop the wasm preload, measured no gain (#866)`.

## Task 10 — Ship gate (A10)

- [ ] **Step 1:** `devtool run --cwd <wt> -- cargo xtask validate` (with e2e,
      all four combos) → green. This is the only step that satisfies A10;
      `check` and a single `e2e` combo do not.
- [ ] **Step 2:** Hand off to **`jaunder-ship`**.

---

## Self-review

- A1–A4 → task 2 (+ task 1 step 2 restates the denominator). A5 → task 3 (incl.
  the `init()`-target test) + task 5, with the spec's revert clause covering
  task 9. A6 → task 4, proven by mutation. A7 → tasks 6, 8. A8 → task 8 step 4.
  A9 → task 1. **A10 → task 10.**
- The plan can end with the fix **reverted** and still satisfy the spec — that
  is D8's point, and the spec's A5 now says so explicitly.
- No task commits a red tree: task 3's tests are committed with task 5, when
  they pass.
- No task asserts a mechanism the cycle does not test.
