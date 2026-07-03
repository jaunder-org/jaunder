# Firefox e2e wall-clock reduction (post-CSR) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure the post-CSR Firefox/Chromium e2e per-test tax, land a
`workers>1` flip (folding in #182) where the data proves it safe, dismantle the
now-vestigial `hydrationHeavy*` timeout inflation, and document the
reducible-vs-irreducible verdict.

**Architecture:** This is a **measurement-gated** cycle. Task 2 produces the
baseline data that gates every later task; Task 3's probes produce a GO/NO-GO on
the worker flip. All measurement reuses the existing #152 tooling
(`scripts/run-e2e-trace-analysis`, `scripts/analyze-otel-traces`, the per-combo
`playwright-report-<backend>.json`). Config lives in `flake.nix`
(`nixPlaywrightConfig`) and `end2end/`; the worker count is threaded through the
existing `warmupEnv` env-passthrough so probing needs no new machinery.

**Tech Stack:** Nix flake `testers.nixosTest` e2e VMs, Playwright
(`@playwright/test`), TypeScript fixtures, OpenTelemetry traces,
Rust/`cargo xtask` gate, ADR markdown.

## ⏸ STATUS: PAUSED — blocked on #210 (2026-07-02)

Tasks 1–4 + the worker-aware timeout fix are **done and committed**; the
workers=4 flip is **verified ready** (3/4 combos 66/66 green at 4c/6GB; Firefox
rock-solid on both backends). It is **deferred**: `sqlite+chromium`
intermittently times out on the pre-existing flaky heavy test
`posts.spec.ts:349`, whose root cause is fixed by **#210** (batch-seed, being
handled by another agent). Rather than mask it with retries/timeout inflation,
the branch keeps `workers=1` (green) until #210 lands.

**Resume after #210 — `/jaunder-develop issue 155` reconstructs the cycle from
these artifacts; the steps below are the remaining work:**

0. **PREREQUISITE (load-bearing): get #210's fix onto this branch first.**
   Confirm #210 has **merged to `main`**, then rebase this branch onto `main`
   (`jaunder-ship/base-moved.md`). Until #210's batch-seed change is present
   here, `posts.spec.ts:349` is still the old slow version and step 2 will flake
   exactly as it does now. Sanity-check that #210 actually made the heavy
   timeline tests fast (its acceptance) before flipping.
1. Flip `nixPlaywrightConfig` default → 4; add `vmMemory=6144; vmCores=4;` to
   `e2eWarmChecks` (values recorded in `docs/observability.md` → "#155 AC3").
2. Re-verify all 4 combos green at workers=4 (`cargo xtask validate`, or the 4
   `cargo xtask e2e <backend> <browser>` builds). With #210 landed,
   `sqlite+chromium` should now be stable.
3. Revisit whether `workerContentionScale` (fixtures.ts) can shrink now that the
   heavy tests are fast — the headroom was sized for the _slow_ tests.
4. Remaining tasks below: rename `hydrationHeavy*` off the false hydration
   premise (Task 6b) + fix the `run-e2e-trace-analysis` out-path parser bug;
   ADR-0039 update (Task 7); resolve #182/#61; ship.

Full status in `docs/observability.md` → "#155 — flip status". #155 is still
claimed (Status: In Progress in project #1) — a resume continues, not re-claims.

## Global Constraints

- **Faithful measurement surface is the nix e2e derivation**, not a bare host
  `playwright test`: `cargo xtask e2e <backend> <browser>` → artifacts at
  `.xtask/diagnostics/e2e-<backend>-<browser>/`. Heavy/cold runs → Bash tool
  background mode.
- **Single canonical findings location:** `docs/observability.md`. AC1/AC3/AC6
  all record there.
- **Document-and-close is valid** for the per-test tax (AC2) and, with specific
  evidence, for the worker flip (AC4). Any measured net wall-clock reduction
  qualifies — no minimum-% bar.
- **The two worker failure modes have opposite worst-case browsers:** contention
  → SQLite+**Chromium** (fastest = max write-overlap); OOM → **Firefox**
  (memory-heavy). Wall-clock benefit measured on Firefox.
- **Gate per task:** the pre-commit hook runs full `cargo xtask check`; run
  `cargo xtask check` first (`jaunder-commit`). **No `Co-Authored-By` trailer.**
  `.ts` edits bust the coverage cache (~2.3 min/commit) — expected.
- **ADRs** via the `jaunder-adr` always-0000 flow: write
  `docs/adr/0000-<slug>.md`, let `cargo xtask adr renumber` assign the number +
  sync the README table. Never pick a number by hand.
- Do **not** commit without explicit user approval (per repo CLAUDE.md);
  `jaunder-iterate` commits are the gate-enforced per-task commits once the plan
  is approved and work has begun.

## File / artifact map

- `flake.nix` — `nixPlaywrightConfig` (~L494, the workers setting + projects),
  `csrPlaywrightConfig` (~L551, spike leftover to retire),
  `mkE2eCombo`/`e2eCombos` (~L992, probe variants via `warmupEnv`/`nameSuffix`),
  VM builders (`mkE2eSqliteCheck`/`mkE2ePostgresCheck`, ~L747), memory/cpu
  sizing.
- `end2end/playwright.config.ts` — local (non-nix) config; mirrors the
  `hydrationHeavyTimeoutScale` (L6) — keep in sync but nix is the gate.
- `end2end/tests/fixtures.ts` — `hydrationHeavyTimeoutScale` (L107),
  `hydrationHeavyTimeoutMs`/`hydrationHeavyFirstNavigationTimeoutMs` (L172/L182)
  — rename + rescale target.
- `end2end/tests/*.spec.ts`, `helpers.ts` — dozens of `hydrationHeavy*` call
  sites + doc comments to rename.
- `end2end/tests/admin-site.spec.ts` — the global-singleton spec needing serial
  quarantine for workers>1.
- `scripts/analyze-otel-traces`, `scripts/run-e2e-trace-analysis` — measurement
  tooling (reuse; light CSR-naming touch-up in Task 6).
- `docs/observability.md` — findings home + `hydrationHeavy*` usage docs.
- `docs/adr/0039-*.md` — the `workers:1` ADR this cycle amends.

---

### Task 1: Coordinate #182 / #61 (claim + cross-link)

Prevents a concurrent agent from grabbing #182 (its own milestone-8 issue) while
this cycle owns it. No code.

**Files:** none (GitHub only).

- [x] **Step 1: Check whether #182 is a Backlog-project item and claim it if
      so.**

```bash
gh project item-list 1 --owner jaunder-org --limit 200 --format json \
  --jq '.items[] | select(.content.number==182) | {id, status}'
```

If it returns an item, set its Status to **In Progress** (same mechanism as
#155, `jaunder-issues/claim-status.md`): `In Progress` option id `47fc9ee4`,
field-id `PVTSSF_lADOECw7os4BblPPzhWUx4Q`, project-id `PVT_kwDOECw7os4BblPP`. If
it returns nothing, #182 isn't in project #1 — skip the status edit (the comment
below is the coordination signal).

- [x] **Step 2: Cross-link the two issues.**

Comment on #182: folded into #155's cycle (branch
`worktree-issue-155-firefox-e2e-wallclock`); the workers>1 flip lands there;
#182 will be closed by that PR (or its residual re-filed). Comment on #155: #182
folded in per the approved spec.

- [x] **Step 3: No commit** (GitHub-only task). Tick the checkboxes and move on.

---

### Task 2: Capture + analyze the post-CSR baseline (Part A → AC1, AC2)

Establish the current Firefox/Chromium per-test ratio on the CSR build and
localize where the delta lives. This data gates Tasks 3–6.

**Files:**

- Modify: `docs/observability.md` (add a `## #155 — post-CSR Firefox e2e tax`
  findings section).

**Interfaces:**

- Produces: the measured **median per-test Firefox/Chromium ratio** and the
  **per-test floor** (per-browser p50/p90 `e2e.test` durations) that Tasks 3 and
  6 consume.

- [x] **Step 1: Run all four combos warm and analyze.** (Heavy — Bash background
      mode.) _`run-e2e-trace-analysis` helper mis-parsed the nix out-path
      (returned `-user-environment`); bypassed by building the 4 checks
      directly + `analyze-otel-traces`. Helper bug flagged for Task 6 cleanup._

Run: `scripts/run-e2e-trace-analysis --top 25` Expected: builds
`e2e-{sqlite,postgres}-{chromium,firefox}`, then prints the analysis — including
**per-project/browser e2e duration breakdown** and **slowest `e2e.test` spans**.
Artifacts land at `.xtask/diagnostics/e2e-<backend>-<browser>/`
(`playwright-report-<backend>.json`, `otel-traces-<backend>.jsonl/`).

- [x] **Step 2: Compute the per-test Firefox/Chromium ratio from the Playwright
      reports.** _sqlite 1.83× / postgres 1.69× median (vs SSR-era 1.90×);
      61–62/66 ≥1.4×._

Parse the per-combo `playwright-report-<backend>.json` (fields: each test's
`title`, `projectName`, `status`, `duration`). For the same backend, pair each
test's Firefox vs Chromium duration and compute the **median per-test ratio**
and the distribution (how many tests ≥1.4×), mirroring #152's method
(`docs/observability.md` "Per-test timing report"). Use
`ctx_execute(language:"javascript")` reading the JSON by absolute path (host has
no python). Expected output: a single ratio number + a short distribution (e.g.
"median 1.3×, 20/66 ≥1.4×") for the record.

- [x] **Step 3: Localize the delta from the OTEL traces.** _Delta is client-side
      `commit_to_hydration` (CSR mount) 2.01× ff/ch; server request/API times
      browser-invariant._

Run:
`scripts/analyze-otel-traces .xtask/diagnostics/e2e-sqlite-firefox/otel-traces-sqlite.jsonl/otel-traces.jsonl .xtask/diagnostics/e2e-sqlite-chromium/otel-traces-sqlite.jsonl/otel-traces.jsonl`
Expected: confirm **server spans are browser-invariant** (the delta is not
server-side) and read the **action/mount hotspots** to see where the client-side
Firefox delta concentrates (uniform vs a few tests).

- [x] **Step 4: Write the Part A findings + verdict into
      `docs/observability.md`.** _Verdict: per-test tax irreducible (inherent
      engine cost); worker parallelism is the lever._

Record: the post-CSR median ratio (vs #152's SSR-era 1.90×), the distribution,
the server-invariance confirmation, where the client delta lives, and the **AC2
verdict** — either (a) a concrete, fixable client hotspot was found (→ note it
becomes an added iterate task), or (b) the residual is uniform inherent
SpiderMonkey-vs-V8 WASM cost → documented irreducible. State the per-browser
per-test floor (p50/p90) for Task 6.

- [x] **Step 5: Commit.** _Committed in `c9c0bc9` (bundled with spec+plan
      scaffolding)._

```bash
git add docs/observability.md
git commit -m "perf(e2e): record post-CSR Firefox/Chromium per-test tax (#155)"
```

Run `cargo xtask check` first (docs-only change — fast). No `Co-Authored-By`.

---

### Task 3: Worker-safety probes (Part B measurement → AC3)

Measure whether `workers>1` is safe on CSR, each failure mode at its worst case.
Ends with an explicit **GO/NO-GO** for the flip.

**Files:**

- Modify: `flake.nix` — make `nixPlaywrightConfig` read the worker count from
  env; add temporary probe combos.

**Interfaces:**

- Consumes: the CSR build (default derivation).
- Produces: the **max safe worker count** `N` (or NO-GO), consumed by Task 4/5.

- [x] **Step 1: Parametrize the worker count in `nixPlaywrightConfig` (default 1
      — no behavior change yet).**

In `flake.nix` `nixPlaywrightConfig`, replace the hard-coded `workers: 1,` with
an env-driven value:

```js
const workers = parseInt(process.env.JAUNDER_E2E_WORKERS || "1", 10);
```

and in the `defineConfig({...})` body:

```js
            workers: workers,
            fullyParallel: workers > 1,
```

Leave the default at 1 so every existing check is byte-for-byte equivalent
(`JAUNDER_E2E_WORKERS` unset → `workers=1`, `fullyParallel:false`).

- [x] **Step 2: Verify the default is unchanged.** (Heavy — background.)
      _`e2e-sqlite-chromium` 66/66 green (6.6m) — parametrization inert at
      default._

Run: `cargo xtask e2e sqlite chromium` Expected: green, 66/66, identical to
before (proves the parametrization is inert at the default).

- [x] **Step 3: Add temporary probe combos** using the existing
      `warmupEnv`/`nameSuffix` passthrough. _Two refinements vs the sketch
      below: (a) attr keys are
      `probe-_`not`e2e-_`, so they stay OUT of the `e2e-checks`    gate aggregate (derivation name still`jaunder-e2e-_-w4`→ excluded from     cachix push); (b) had to add`vmMemory`/`vmCores`params to`mkE2eSqliteCheck`    (it hard-coded 2048MB, no cores) and thread them through`mkE2eCombo`, so the     contention probe runs on a 4-core/6GB VM — on 1 vCPU the workers would     timeshare and under-stress SQLite locking. Both probes: `vmMemory=6144`,     `vmCores=4`.\*

```nix
        # TEMP (#155 Task 3 probes — removed in Task 5):
        e2eProbeChecks = {
          "e2e-sqlite-chromium-w4" = mkE2eCombo {
            backend = "sqlite"; browser = "chromium"; traceDigit = "1";
            nameSuffix = "-w4"; warmupEnv = " JAUNDER_E2E_WARMUP=1 JAUNDER_E2E_WORKERS=4";
          };
          "e2e-postgres-firefox-w4" = mkE2eCombo {
            backend = "postgres"; browser = "firefox"; traceDigit = "4";
            nameSuffix = "-w4"; warmupEnv = " JAUNDER_E2E_WARMUP=1 JAUNDER_E2E_WORKERS=4";
          };
        };
```

`JAUNDER_E2E_WARMUP=1` is **load-bearing for the contention probe**: it matches
the real warm gate (`e2eWarmChecks`) so requests fire at full rate — a cold
probe would slow the request stream, _reduce_ write overlap, and under-stress
the exact "fastest browser = max overlap" worst case. Merge `e2eProbeChecks`
into the `checks` attrset so `nix build` can reach them. (If Firefox needs the
bigger VM to avoid OOM, bump `mkE2ePostgresCheck`'s
`virtualisation.memorySize`/`cores` for the probe — see Step 5.)

- [x] **Step 4: Run the contention probe (SQLite + Chromium, workers=4).**
      (Background.) _Refuted — ZERO `SQLITE_BUSY`/lock errors at 4 workers.
      WAL + busy_timeout absorbs concurrent writes. The failures were timeouts,
      not locks._

Run: `nix build .#checks.<system>.e2e-sqlite-chromium-w4 -L --rebuild` Expected:
read the build log for `SQLITE_BUSY` / `database is locked` / test failures.
WAL + 5s `busy_timeout` (`storage/src/sqlite/mod.rs`) should serialize writes
without error. Record pass/fail. (Also spot-check workers=2 if 4 is
inconclusive.)

- [x] **Step 5: Run the OOM probe (Firefox, workers=4), sizing the VM.**
      (Background.) _Refuted — postgres+firefox 66/66 clean at 4 workers on
      6GB/4-core (3.5m). No OOM. Firefox tolerates workers=4 (its 2.2× timeout
      scale absorbs the CPU contention)._

Run: `nix build .#checks.<system>.e2e-postgres-firefox-w4 -L --rebuild`
Expected: watch for OOM-kills / VM death vs a clean 66/66. Milestone-8 note: 4
Firefox workers on a 4 GB VM OOM'd → if it OOMs, raise the probe VM to ≥6 GB /
≥4 vCPU and rerun; record the smallest VM size that runs N Firefox workers
cleanly.

- [x] **Step 6: Record the AC3 answer + GO/NO-GO in `docs/observability.md`.**
      _GO — uniform `workers=4` (user's choice). Contention + OOM refuted;
      blocker is heavy-test timeout headroom under CPU oversubscription (Part C
      prerequisite). **REORDER:** the timeout headroom (Task 6) now precedes the
      flip (Task 5) — the workers=4 flip isn't green until the heavy chromium
      tests get worker-aware budgets._

Record: contention outcome (SQLite+Chromium at workers 2/4), OOM outcome +
required VM size (Firefox at workers 2/4), and the **chosen max safe `N`**.
Decide **GO** (a safe `N>1` exists) or **NO-GO** (no safe `N`; e.g. contention
failures WAL can't absorb, or OOM even on a viable VM). NO-GO → Task 4/5 become
"document the blocker + keep workers:1 + file per-worker-DB-isolation
follow-up"; GO → proceed to the flip.

- [ ] **Step 7: Commit** (probes + parametrization; probe combos removed in Task
      5).

```bash
git add flake.nix docs/observability.md
git commit -m "test(e2e): probe workers>1 safety on CSR (contention + OOM) (#155)"
```

---

### Task 4: `admin-site` serial-project quarantine (prereq for the flip)

**Only if Task 3 = GO.** Isolate the one global-singleton spec so it never runs
concurrently with readers, per ADR-0039. Landed at `workers:1` so it's
verifiable independently of the flip.

**Files:**

- Modify: `flake.nix` `nixPlaywrightConfig` `projects` array.
- (Mirror into `end2end/playwright.config.ts` for local parity.)

**Interfaces:**

- Produces: a `projects` layout where `admin-site` runs alone after the main
  project, safe under `fullyParallel`.

- [ ] **Step 1: Split `admin-site` into a dependent serial project.** In each
      browser's project entry, exclude admin-site from the parallel project and
      add a serial companion that runs after it:

```js
            projects: [
              {
                name: 'chromium',
                testIgnore: /admin-site\.spec\.ts/,
                use: { ...devices['Desktop Chrome'], launchOptions: { args: ['--no-sandbox','--disable-gpu','--disable-dev-shm-usage'] } },
              },
              {
                name: 'chromium-admin',
                testMatch: /admin-site\.spec\.ts/,
                fullyParallel: false,
                dependencies: ['chromium'],
                use: { ...devices['Desktop Chrome'], launchOptions: { args: ['--no-sandbox','--disable-gpu','--disable-dev-shm-usage'] } },
              },
              { name: 'firefox', testIgnore: /admin-site\.spec\.ts/, use: { ...devices['Desktop Firefox'] } },
              { name: 'firefox-admin', testMatch: /admin-site\.spec\.ts/, fullyParallel: false, dependencies: ['firefox'], use: { ...devices['Desktop Firefox'] } },
            ],
```

The `--project ${browser}` invocation (flake.nix `e2eRunAndCapture`) must now
also run the `<browser>-admin` project: change it to
`--project ${browser} --project ${browser}-admin`.

- [ ] **Step 2: Verify the suite is still green at workers:1 with admin-site
      serialized.** (Background.)

Run: `cargo xtask e2e sqlite chromium` Expected: green, all tests including
`admin-site` run (admin-site after the main project). Confirms the quarantine
doesn't drop or double-run tests.

- [ ] **Step 3: Mirror the project split into `end2end/playwright.config.ts`**
      (local parity; #153 tracks full dedup — touch only as needed here).

- [ ] **Step 4: Commit.**

```bash
git add flake.nix end2end/playwright.config.ts
git commit -m "test(e2e): quarantine admin-site in a serial project for workers>1 (#155)"
```

---

### Task 5: Resolve the worker flip — land it (GO) or document the blocker (NO-GO) (Part B → AC4)

**Un-gated — runs on both branches.** Steps 1–2 are the GO path (flip + VM
sizing); Step 3 is the NO-GO path; Steps 4–7 (probe/dead-code cleanup, gate,
record, commit) run **regardless of outcome**. On NO-GO, Task 4's admin-site
quarantine was skipped, so there's nothing to unwind there.

**Files:**

- Modify: `flake.nix` — worker default + VM sizing (GO only), remove probe
  combos + `csrPlaywrightConfig` + stale SSR comments (both branches);
  `docs/observability.md`.

- [ ] **Step 1 (GO only): Flip the default worker count** to the safe `N` from
      Task 3. In `nixPlaywrightConfig`:

```js
const workers = parseInt(process.env.JAUNDER_E2E_WORKERS || "N", 10); // N = Task 3's safe count
```

(Keep the env override so future tuning stays config-only.)

- [ ] **Step 2 (GO only): Size the e2e VMs** to Task 3's measured requirement
      (`cores`/`memorySize` in `mkE2eSqliteCheck`/`mkE2ePostgresCheck`), so
      Firefox at `N` workers doesn't OOM.

- [ ] **Step 3 (NO-GO only): Keep `workers:1`.** Leave the parametrized default
      at 1, note the specific blocker (contention WAL can't absorb / OOM even on
      a viable VM) for Step 6, and rely on Task 7 Step 2 to file the
      per-worker-DB-isolation / worker follow-up. (Admin-site quarantine, Task
      4, was not applied on this branch.)

- [ ] **Step 4 (both branches): Retire the probe scaffolding + dead code.**
      Remove the temporary `e2eProbeChecks` (Task 3) _regardless of outcome_;
      and — as dead code **independent of the flip** — the now-unused
      `csrPlaywrightConfig` (~L551) and the stale SSR comments (~L544-550, L846,
      incl. the dead `jaunderBinCsr`/`csrSite` reference). The env
      parametrization (default worker count) stays either way.

- [ ] **Step 5 (both branches): Verify all four combos green + measure.**
      (Background.)

Run: `cargo xtask validate` Expected — GO: all 4 `{backend}×{browser}` combos
green at workers=`N`; capture each combo's wall-clock vs the Task 2 baseline.
NO-GO: all 4 green at workers:1 (confirms the scaffolding removal broke
nothing).

- [ ] **Step 6 (both branches): Record the AC4 result in
      `docs/observability.md`.** GO: landed `N`, VM size, per-combo before/after
      wall-clock, net reduction. NO-GO: the specific blocker + evidence,
      `workers:1` retained, reference to the follow-up filed in Task 7.

- [ ] **Step 7 (both branches): Commit.**

```bash
# GO:
git add flake.nix docs/observability.md
git commit -m "perf(e2e): flip nix e2e to workers>1 on CSR, closes #182 (#155)"
# NO-GO (no closes #182 — resolved via Task 7 follow-up instead):
#   git commit -m "perf(e2e): document workers>1 blocker, retire CSR spike scaffolding (#155)"
```

---

### Task 6: Re-measure the floor + dismantle vestigial timeouts (Part C → AC5)

Reconcile the `hydrationHeavy*` inflation to the real post-flip per-test floor,
and rename it off the false "hydration" premise.

**Files:**

- Modify: `end2end/tests/fixtures.ts`, `end2end/playwright.config.ts`, all
  `end2end/tests/*.spec.ts` + `helpers.ts` (rename),
  `scripts/analyze-otel-traces` (CSR column naming), `docs/observability.md`.

- [ ] **Step 1: Re-measure the per-test floor at the landed worker setting.**
      (Background.)

Run: `scripts/run-e2e-trace-analysis --browser firefox --top 25` Expected:
current Firefox `e2e.test` p50/p90 under the new worker count — the evidence
base for the new budgets. (`run-e2e-trace-analysis` takes `--browser`;
`--project` is `analyze-otel-traces`'s flag, not this one.)

- [ ] **Step 2: Rescale or remove the Firefox multiplier.** In
      `end2end/tests/fixtures.ts` (and mirror `playwright.config.ts:6`), set
      `hydrationHeavyTimeoutScale` to what the measured ratio justifies (ratio ≈
      1.0 → drop the multiplier to `1.0`/remove; a residual ratio → set it to
      that + margin). Do **not** set it to its current `2.2` unless the data
      still shows ~2.2×.

- [ ] **Step 3: Rename the misnomer helpers.** Rename
      `hydrationHeavyTimeoutScale` → e.g. `slowBrowserTimeoutScale`,
      `hydrationHeavyTimeoutMs` → `browserBudgetMs`,
      `hydrationHeavyFirstNavigationTimeoutMs` → `firstNavigationBudgetMs` (or
      agreed names), across `fixtures.ts`, `helpers.ts`, every `*.spec.ts` call
      site, and the doc comments (helpers.ts:24-33, fixtures.ts:104). Update
      `docs/observability.md`'s `hydrationHeavy*` usage section (L99-107) to the
      new names + CSR reality (no hydration).

- [ ] **Step 4: Tighten the hard-coded per-test budgets** toward the Task 6.1
      floor + a sane buffer where the measurement shows comfortable margin (the
      10s–150s literals). Keep each conservative enough to stay green.

- [ ] **Step 5: (Light) update `scripts/analyze-otel-traces`** so its
      hydration-labeled columns (`commit -> hydration`, `wasm_init`,
      `leptos_hydrate`, `post_hydrate_effects`) read correctly on CSR — at
      minimum a comment/label noting they now measure CSR mount, if the fields
      still populate.

- [ ] **Step 6: Verify all four combos stay green.** (Background.)

Run: `cargo xtask validate` Expected: all 4 combos green with the tightened
budgets (a tightened budget that flakes is too tight — loosen it, floor+buffer
is the evidence).

- [ ] **Step 7: Commit.**

```bash
git add end2end/ scripts/analyze-otel-traces docs/observability.md
git commit -m "test(e2e): retire vestigial hydrationHeavy timeout inflation post-CSR (#155)"
```

---

### Task 7: ADR-0039 update + resolve #182 / re-point #61 + final gate (AC6, AC7)

**Files:**

- Modify: `docs/adr/0039-*.md` (or new `docs/adr/0000-*.md` if a superseding
  decision is cleaner).

- [ ] **Step 1: Author the ADR update via the always-0000 flow.** If the flip
      landed: amend ADR-0039's "`workers > 1` flip — DEFERRED" section to record
      the landed `workers=N` decision (the safe count, VM sizing, admin-site
      quarantine, #173-is-dead rationale), or write `docs/adr/0000-<slug>.md`
      superseding it if the change is large. If NO-GO: record the documented
      blocker + why it stays deferred. Then:

Run: `cargo xtask adr renumber` Expected: assigns the number (if a new ADR) +
syncs the README table; `adr-format`/`adr-readme-parity` gates pass.

- [ ] **Step 2: Resolve #182 + re-point #61.** Confirm the Task 5 commit
      references `closes #182` (GO) — or, on NO-GO, file the
      per-worker-DB-isolation follow-up (`jaunder-issues`) and comment on #182
      that its residual moved there. Re-point #61's `blocked-by`/dependency from
      #182 onto this cycle's PR so #61 isn't orphaned. (Final issue-close
      happens on merge in `jaunder-ship`.)

- [ ] **Step 3: Full local gate.** (Background.)

Run: `cargo xtask validate` Expected: static + coverage + all 4 e2e combos green
— the "green → you may move on" bar. This is the ship-readiness gate.

- [ ] **Step 4: Commit.**

```bash
git add docs/adr/
git commit -m "docs(adr): record the e2e workers decision, supersede ADR-0039 workers:1 (#155)"
```

---

## Self-review notes

- **Spec coverage:** AC1/AC2 → Task 2; AC3 → Task 3; AC4 → Task 5 (Steps 1–2 GO
  / Step 3 NO-GO, both recorded in Step 6); AC5 → Task 6; AC6 → Task 7 Step 1;
  AC7 → Tasks 1 + 7 Steps 2–3. Separable concern (per-worker DB isolation) is
  filed conditionally in Task 7 Step 2, not folded in silently.
- **Data-gating is explicit:** only **Task 4** (admin-site quarantine) is
  GO-only; **Task 5 is un-gated** — Steps 1–2 land the flip on GO, Step 3
  documents the blocker on NO-GO, and the probe/dead-code cleanup + gate +
  record + commit (Steps 4–7) run on both branches, so nothing is stranded.
- **Naming consistency:** the renamed helpers (Task 6 Step 3) must be applied at
  every call site in the same commit — the rename is atomic across
  `fixtures.ts`/`helpers.ts`/`*.spec.ts`/`observability.md`.
