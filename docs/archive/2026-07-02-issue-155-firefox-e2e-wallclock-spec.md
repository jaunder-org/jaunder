# Spec — Reduce Firefox e2e wall-clock (post-CSR), encompassing the workers>1 flip

- Issue: #155 (`perf(e2e): reduce Firefox e2e wall-clock`)
- Also closes: #182 (`Re-enable parallel e2e (workers>1) once SSR is gone`) —
  folded in (see §Scope)
- Milestone: E2E test suite
- Date: 2026-07-02
- Status: draft (awaiting approval)

## Context

#152 built per-test e2e timing/attribution and root-caused the Firefox slowdown:
Firefox ran the suite **~1.9× slower than Chromium, uniformly** (median per-test
ratio 1.90×, 62/66 tests ≥1.4×), and the cost was **entirely browser-side** —
every server OTEL span identical across browsers; the delta lived in the client
`e2e.test` span. Because Firefox is the long pole of the parallel
`{backend}×{browser}` matrix (#129), that ratio sets e2e CI wall-clock (~12m vs
a ~7m40s Chromium/validate floor).

**The premise has since shifted.** The Milestone-8 leptos-CSR cutover (PR#192,
merged 2026-07-02, closed #180) **removed SSR + hydration entirely**. The client
is now leptos-CSR: it mounts fresh via `mount_to_body()`; there is no
`waitForHydration` reconciliation step to profile. The `body[data-hydrated]`
marker survives but now means "CSR mount done" (`csr/src/lib.rs`,
`web/src/lib.rs`). So the issue's original "profile `waitForHydration` / reduce
hydration cost" acceptance criterion is **obsolete** — there is no hydration
path left.

**The default e2e derivation is now CSR** (verified, not assumed): there is no
`hydrate` crate, the `csr` crate is the mount entry, and the single build
`jaunderPkg ? jaunderBin` (flake.nix:849) is the CSR binary —
`jaunderBinCsr`/`csrSite` referenced at flake.nix:846 exist **only in a stale
comment**, and `csrPlaywrightConfig` (flake.nix:551) is a defined-but-unused
spike leftover. So "measure the default derivation" _is_ "measure CSR." The
SSR-referencing comments in `flake.nix` (~L544-550, L846) are stale cleanup
debt; this cycle retires them as part of the worker-flip work (see §Scope Part
B).

The #152 timing harness (`end2end/tests/` — `otel.ts`, `fixtures.ts`'s auto
`_autoPerfSpan`, `actions.ts`, `helpers.ts`, `hydration.ts`, `perf.ts`) survived
the cutover **intact and functional**: it measures user-observable events
(request / DOM / mount / action), not SSR internals, so it works transparently
on CSR. It is the instrument this issue uses.

**Why #182 is folded in.** #182 ("re-enable parallel e2e, workers>1") targets
the single biggest lever on the exact metric #155 owns (wall-clock), using the
same instrument (the #152 harness tells us whether workers>1 contends or OOMs),
in the same subsystem (the nix e2e config), and its sole blocker (#173) is now
closed by the CSR cutover. There is no principled seam between "measure/reduce
Firefox wall-clock" and "flip workers>1"; keeping them apart was an artifact of
#182 being milestone-8's deferred tail. This cycle does both.

## Resolved decisions (from the design interview)

1. **Adopt the CSR re-scope.** #155 = re-run the #152 harness on the _current
   CSR build_, measure the Firefox/Chromium per-test ratio and localize where
   the residual delta lives, then reduce-or-document. Hydration profiling is
   dropped entirely (no hydration exists).

2. **Fold worker-parallelism (#182) in.** Measurement covers BOTH the per-test
   Firefox CSR tax AND whether the SQLite and Firefox combos tolerate
   `workers>1` on CSR. If a safe worker bump falls out of the data, **land it
   here**; if it needs a large independent effort (e.g. per-worker DB
   isolation), split _that_specific_piece_ to a follow-up decided from the data
   — not assumed now.

3. **Document-and-close is a valid outcome for the per-test tax.** If
   measurement shows the residual Firefox per-test delta is inherent
   SpiderMonkey-vs-V8 WASM-execution cost, a documented conclusion closes that
   half with no per-test code change. (The worker-flip half still lands or is
   documented-as-blocked with specific evidence — not a vague comment.)

4. **The "SQLite write contention" concern is a hypothesis to test, not a
   given.** The SQLite pool already runs WAL + a 5s `busy_timeout`
   (`storage/src/sqlite/mod.rs:97-102`), and the original `SQLITE_BUSY` race was
   resolved app-side via `BEGIN IMMEDIATE` (#18/#51/#52/#53, per ADR-0039). The
   #177 spike proved `workers:4`+`fullyParallel` is panic-free **but only on
   postgres+chromium** (spike evidence: `docs/issue-177-csr-spike-findings.md` /
   `docs/archive/2026-06-30-issue-177-leptos-csr-spike.md`, the ~30-run
   postgres+chromium campaign) — SQLite-at-workers>1 and Firefox-at-workers>1
   are unmeasured. The measurement resolves this empirically.

## Scope

### Part A — Measure the current CSR Firefox per-test tax

- Re-run the #152 harness on the current CSR build for the Firefox and Chromium
  combos and extract per-test durations (`playwright-report.json`) + the
  `e2e.test` span attribution (OTEL traces), the same way #152 did. The
  faithful, CI-representative surface is the nix e2e derivation
  (`cargo xtask e2e <backend> <browser>` → `.xtask/diagnostics/e2e-*`), not a
  bare host `playwright test`.
- Recompute the Firefox/Chromium ratio on CSR and compare to #152's SSR-era
  1.90×. Localize the residual: server spans should remain browser-identical;
  the delta should sit in client action/mount/render time. Confirm the tax is
  still uniform (vs concentrated).
- Decide: is any portion of the per-test delta reducible (a fixable client
  cost), or is the residual inherent engine cost to be documented?

### Part B — Worker parallelism (encompasses #182)

- **Measure** whether `workers>1` on CSR is safe, probing each failure mode with
  its **worst-case combo** — the two modes have _opposite_ worst-case browsers:
  - **Write contention → worst case is the FASTEST browser (Chromium).** The DB
    write path is server-side and browser-invariant (#152: server spans
    identical across browsers); the browser only sets the _rate_ a worker fires
    DB-mutating requests. Chromium (~1.9× faster) compresses each worker's
    timeline and raises its server duty-cycle, maximising write overlap — the
    max-stress test for SQLite's single-writer lock. Probe **SQLite + Chromium**
    at the target worker count for `SQLITE_BUSY` / lock errors / failures under
    WAL + 5s busy_timeout; if that is clean, Firefox (slower, writes more spread
    out) is contention-safe a fortiori.
  - **Memory / OOM → worst case is Firefox.** Firefox workers are the
    memory-heavy long pole; probe **Firefox** at N workers for VM OOM
    (Milestone-8 notes: ≥4 vCPU/6 GB; 4 Firefox workers on a 4 GB VM OOM'd). VM
    sizing to clear this is part of the flip.
  - **Wall-clock benefit** is then measured on **Firefox** (the long pole the
    flip shortens), though the flip applies uniformly to all four
    `{backend}×{browser}` combos.
- **Land the flip where the data supports it.** A `workers>1` flip requires, per
  ADR-0039:
  - the `nixPlaywrightConfig` `workers`/`fullyParallel` change (retire/rework
    the spike-only, defined-but-unused `csrPlaywrightConfig`);
  - the `admin-site` serial-project quarantine (it mutates the
    `site.title`/`base_url` global singleton; sequenced via Playwright project
    `dependencies`);
  - the e2e VM capacity bump to whatever the Firefox measurement requires;
  - retiring the stale SSR-referencing `flake.nix` comments (~L544-550, L846,
    incl. the dead `jaunderBinCsr`/`csrSite` reference) so the config describes
    the CSR-only reality.
- If SQLite at workers>1 genuinely needs per-worker DB isolation as a large
  independent effort, split that piece to a follow-up (decided from data) and
  land the safe subset.

### Part C — dismantle vestigial timeouts (driven by the Part A measurement)

The suite carries a whole timeout-inflation layer sized for the _old_ world (SSR
hydration + a heavy Firefox penalty): `hydrationHeavyTimeoutScale = 2.2` gives
every Firefox project a 2.2× per-test budget (`end2end/playwright.config.ts:6`,
`fixtures.ts:107`), and `hydrationHeavyTimeoutMs` /
`hydrationHeavyFirstNavigationTimeoutMs` hard-code generous per-test budgets
(10s–150s) at dozens of call sites across every spec. Over-long timeouts **mask
regressions and slow failure detection**, so once Part A establishes the real
post-CSR per-test floor:

- **If Firefox is comparable / Chromium sped up**, cut or shrink the `2.2×`
  `hydrationHeavyTimeoutScale` Firefox multiplier toward what the measured
  Firefox/Chromium ratio actually justifies (a 1.0× ratio ⟹ no multiplier), and
  tighten the hard-coded per-test budgets toward the measured floor + a sane
  buffer.
- **Regardless of the ratio**, the `hydrationHeavy*` naming is a misnomer
  post-CSR (there is no hydration) — rename the scaler/helpers to reflect what
  they now size (browser-speed / cold- WASM budget), so the code stops implying
  a hydration cost that no longer exists.
- Any tightening must keep all 4 combos green (a tightened budget that flakes is
  too tight); the measured floor + buffer is the evidence for each new value.

### Documentation & ADR

- Record the CSR Firefox-tax findings (ratio, attribution,
  reducible-vs-irreducible verdict) **and** the workers>1 measurement in
  `docs/observability.md` (the e2e-tracing home) — the single canonical location
  for all #155 findings (AC1/AC3/AC6 all point here).
- The `workers` outcome amends/supersedes **ADR-0039** (which pinned `workers:1`
  and marked the flip DEFERRED): either a new `workers>N` decision or a
  documented "still deferred, here's the specific blocker." Authored via the
  `jaunder-adr` always-0000 flow.

## Acceptance criteria

- **AC1** — A profile of the current CSR Firefox per-test delta
  (Firefox/Chromium ratio, where the delta lives: request vs mount vs render vs
  interaction), produced from the #152 harness on the CSR build, recorded in
  `docs/observability.md`.
- **AC2** — A reasoned verdict on the per-test tax: either a landed reduction OR
  a documented conclusion that the residual is irreducible browser-engine cost.
- **AC3** — A measured answer on `workers>1`, each failure mode probed at its
  worst case: the **contention** probe (SQLite + Chromium, max write-overlap)
  and the **OOM** probe (Firefox, N workers) — each reported with worker count
  and outcome in `docs/observability.md`.
- **AC4** — Either a landed `workers>1` flip (config + `admin-site` quarantine +
  VM sizing, green across all 4 `{backend}×{browser}` combos) yielding a
  measured net reduction in the e2e-gate wall-clock — **any measured net
  reduction qualifies; there is no minimum-percentage bar** (the issue
  explicitly permits document-and-close if no reduction is reducible) — OR a
  documented conclusion with the specific blocker and evidence (not a vague
  comment).
- **AC5** — Timeouts reconciled to the Part A measurement: the
  `hydrationHeavyTimeoutScale` Firefox multiplier cut/shrunk to what the
  measured ratio justifies (or documented why it stands), the `hydrationHeavy*`
  helpers renamed off the now-false "hydration" premise, and per-test budgets
  tightened toward the measured floor + buffer — all 4 combos still green.
- **AC6** — ADR-0039 updated to reflect the resulting `workers` decision;
  findings in `docs/observability.md`.
- **AC7** — #182 resolved by this cycle (closed if the flip lands; or its
  remaining piece re-filed as a data-justified follow-up). #61's dependency on
  #182 re-pointed so it is not orphaned. The full local gate
  (`cargo xtask validate`, all 4 e2e combos) is green.

## Out of scope / separable concerns

- **Per-worker DB isolation** as a large standalone effort — only if the
  SQLite-at-workers>1 measurement shows plain WAL+busy_timeout is insufficient;
  split then, from data.
- **Deduping the local `end2end/playwright.config.ts` vs `nixPlaywrightConfig`**
  — that is #153; touch only as incidentally required by the flip.
- **#61's own remaining umbrella work** beyond what the workers>1 flip unblocks.

## References

- Issues: #155, #182 (folded), #152 (visibility/attribution, predecessor), #129
  (the matrix), #173 (the closed blocker), #61 (downstream of the flip), #153
  (config dedup), #130 (timeout).
- ADRs: ADR-0039 (per-test identity fixtures; `workers:1` stopgap — this cycle
  amends it), ADR-0040 (leptos-CSR rendering), ADR-0034 (e2e matrix /
  `e2e gate`).
- Harness: `end2end/tests/{otel,fixtures,actions,helpers,hydration,perf}.ts`.
- Config: `flake.nix` (`nixPlaywrightConfig` L494, `csrPlaywrightConfig` L551,
  `e2eCombos` L992, VM builders + `e2eRunAndCapture`),
  `end2end/playwright.config.ts`.
