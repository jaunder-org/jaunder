# Prune CSR-OBE hydration trace sections — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Remove the four CSR-obsolete hydration-framed sections from
`xtask traces analyze` and the now-dead span attributes that only feed them,
prune section 4's four always-null hydration phases, and rename the surviving
live phase `commit_to_hydration` → `commit_to_mount`.

**Architecture:** Pure deletion + rename across four surfaces — the xtask
consumer (`analyze.rs`, `render.rs`), its hand-crafted test fixture
(`testdata/otel-traces-sample.jsonl`), the live e2e emitter
(`end2end/tests/fixtures.ts`), and `docs/observability.md`. No new behavior.

**Tech Stack:** Rust (xtask crate, `cargo nextest`), TypeScript (Playwright
e2e).

**Spec:**
`docs/superpowers/specs/2026-07-04-issue-228-prune-hydration-trace-sections.md`
— the "how" is here; the "what/why", the CSR-context findings, and the
acceptance criteria (AC1–AC8) live in the spec. Reference by AC number.

## Global Constraints

- Governing decision: **ADR-0040** (leptos-CSR, no hydration). The removed
  sections/attributes are OBE because their SSR instrumentation was deleted.
- The `commit_to_hydration → commit_to_mount` rename is **load-bearing on the
  camelCase field** `commitToHydrationMs → commitToMountMs`, which must change
  in lockstep on **both** the emitter side (`fixtures.ts` + the xtask test
  fixture JSONL) and the parser side (`NAV_PHASES` second tuple element). Rename
  on one side only → section 4's phase silently reads null (spec AC4).
- Twelve sections → **eight** (former 1, 2, 3, 4, 6, 7, 11, 12 survive).
- Coverage gate is stateless / CRAP-threshold (ADR-0050); removing code _and_
  its tests together keeps it green. `cargo xtask check` runs it.
- Commit messages: **no `Co-Authored-By` trailer**. Pre-commit hook runs the
  full `cargo xtask check` — run it first so it passes clean
  (**jaunder-commit**).

---

## Review header

**Scope — in:**

- Remove report sections #5 (cache-warmth), #8 (hydration-vs-API), #9 (nav phase
  components), #10 (hydration runtime) from the xtask consumer + their tests.
- Prune section 4's four null `NAV_PHASES` phases; rename `commit_to_hydration`
  → `commit_to_mount` across parser, xtask test fixture, and the live emitter.
- Remove the dead attribute emission + `__jaunder_perf` plumbing from
  `fixtures.ts`.
- Update `docs/observability.md`.

**Scope — out (filed as follow-on issues in Task 1, not folded in):**

- Renaming the `hydrationHeavy*` timeout helpers.
- Renaming the `body[data-hydrated]` mount-readiness marker / `waitForHydration`
  / `end2end/tests/hydration.ts`.

**Tasks:**

1. File the two out-of-scope renames as follow-on issues (jaunder-issues).
2. xtask: remove the four sections (structs, fns, constants, `Analysis` fields,
   render dispatch, Display structs, and the four section tests + order test).
3. xtask: prune the four null `NAV_PHASES` phases + rename `commit_to_hydration`
   → `commit_to_mount` (parser + section-4 test + xtask fixture JSONL). Runs
   **after** Task 2 so no still-present section test reads the renamed key.
4. e2e emitter: strip dead attributes + `__jaunder_perf` plumbing from
   `fixtures.ts`; rename `commit_to_hydration_ms` → `commit_to_mount_ms`.
5. Docs: update `docs/observability.md`.
6. Full gate: `cargo xtask validate` green (incl. e2e — proves the emitter
   changes).

**Key risks/decisions:**

- The rename's load-bearing side is the camelCase field, not the OTLP attribute
  string (see Global Constraints). Task 2 and Task 4 each call it out.
- Task 3's deletions must not orphan a _shared_ helper. `NAV_PHASES` (section
  4), `field_f64`, `Agg`, `e2e_test_name` (section 2), `parse_json_attr` are
  shared — keep them. Only `NAV_PHASE_COMPONENTS` and
  `HYDRATION_RUNTIME_COMPONENTS` are exclusive to cut sections. clippy
  `dead_code` is the backstop.
- No e2e test reads the removed attributes (verified: only `fixtures.ts` does),
  so Task 4 cannot break a spec test — but it touches the harness, so Task 6's
  e2e run is the gate.

---

## Task 1: File the two deferred renames as follow-on issues

**Files:** none (GitHub issues via **jaunder-issues**).

**Interfaces:**

- Produces: two issue numbers, referenced nowhere in code — captured so the
  separable concerns aren't lost (spec AC8).

- [x] **Step 1: File issue A — `hydrationHeavy*` timeout-helper rename.** →
      filed as **#250** (type Task, label `tooling`, milestone "Devtool
      migration", in Backlog project #1).

- [x] **Step 2: File issue B — `data-hydrated` marker rename.** → filed as
      **#251** (type Task, label `tooling`, milestone "Devtool migration", in
      Backlog project #1).

- [x] **Step 3: Record the two issue numbers** (#250, #251) — to be referenced
      in the PR description at ship time.

---

## Task 2: Remove the four CSR-OBE sections from the xtask consumer

**Ordering:** runs before Task 3 — while the four section tests still exist they
read the hardcoded key `"commitToHydrationMs"`, so this task must delete them
_before_ Task 3 renames that key (else Task 3's commit gate fails).

**Files:**

- Modify: `xtask/src/traces/analyze.rs` — `Analysis` fields (:118-131), the cut
  structs (`CacheWarmthRow` :219-, `HydrationVsApiRow` :231-, `PhaseSampleRow`/
  `PhaseTargetRow`/`PhaseProjectRow`, `RuntimeSampleRow`/`RuntimeProjectRow`),
  the cut fns (`cache_warmth_rows` :521, `hydration_vs_api_rows` :562,
  `nav_phase_component_sections` :616, `hydration_runtime_sections` :706), the
  dead constants (`NAV_PHASE_COMPONENTS` :608, `HYDRATION_RUNTIME_COMPONENTS`
  :697), the build calls + `Analysis {}` fields (:851-880), and the four section
  tests (`cache_warmth_by_warmth_and_project` :1043, `hydration_vs_api_budget`
  :1061, `navigation_phase_components` :1078, `hydration_runtime_components`
  :1102)
- Modify: `xtask/src/traces/render.rs` — the cut Display structs + `impl From`
  (`CacheWarmthDisplay` :230, `HydrationVsApiDisplay` :251, `PhaseSampleDisplay`
  :277, `PhaseTargetDisplay` :298, `PhaseProjectDisplay` :321,
  `RuntimeSampleDisplay` :342, `RuntimeProjectDisplay` :363), the `use` imports
  (:12-14), the four `section::<…>(…)` dispatch blocks (:419-473), and the
  canonical-order test markers (:545-561)
- Modify: `xtask/src/traces/testdata/otel-traces-sample.jsonl` — drop the
  `e2e.hydration_runtime_json` attribute and the `cacheWarmth` key inside each
  `e2e.navigation_top_json` payload (both now have no consumer). Leave
  `commitToHydrationMs` (renamed in Task 3) and the four phase-component
  camelCase keys (still read by `NAV_PHASES` until Task 3) in place.

**Interfaces:**

- Consumes: `NAV_PHASES` (unchanged here — still 9 entries incl.
  `commit_to_hydration`; Task 3 prunes/renames it).
- Produces: `Analysis` with the 7 removed field groups gone (8 section groups
  survive); `render` emitting the 8 surviving sections (11 render tables). No
  public signature changes to `analyze` / `analyze_spans` / `render`.

- [x] **Step 1: Remove the four section tests + trim the order test (compile
      breakage is expected mid-task).** In `analyze.rs` delete the four
      `#[test]` fns `cache_warmth_by_warmth_and_project`,
      `hydration_vs_api_budget`, `navigation_phase_components`,
      `hydration_runtime_components`. In `render.rs`
      `render_emits_sections_in_canonical_order` (:545-561), delete the four
      order markers: `"commit->hydration by cache warmth"`,
      `"hydration budget vs API     budget"`,
      `"navigation phase component samples"`,
      `"hydration runtime     component samples"` (leaving 11 markers).

- [x] **Step 2: Delete the production code for the four sections.** -
      `analyze.rs`: remove the **7** `Analysis` fields (`cache_warmth`,
      `hydration_vs_api`, `nav_phase_component_samples/_targets/_by_project`,
      `hydration_runtime_samples/_by_project`, `:118-131`); the `Row` structs
      listed above; the constants `NAV_PHASE_COMPONENTS`,
      `HYDRATION_RUNTIME_COMPONENTS`; the fns `cache_warmth_rows`,
      `hydration_vs_api_rows`, `nav_phase_component_sections`,
      `hydration_runtime_sections`; and their `let … =` build calls + the
      corresponding lines in the `Analysis {}` literal (:851-880). Keep the
      shared helpers (`NAV_PHASES`, `field_f64`, `Agg`, `e2e_test_name` — still
      used by `slowest_e2e_tests`/section 2 at :783 — `parse_json_attr`,
      `sort_desc_by`, `entry`, `project_label`). - `render.rs`: remove the 7
      `*Display` structs + their `impl From`, the four `section::<…>(…)`
      dispatch blocks (:419-423 cache-warmth, :444-448 hydration-vs-API,
      :449-463 phase components, :464-473 runtime), and the now-unused names
      from the `use super::analyze::{…}` import (:12-14): `CacheWarmthRow`,
      `HydrationVsApiRow`, `PhaseProjectRow`, `PhaseSampleRow`,
      `PhaseTargetRow`, `RuntimeProjectRow`, `RuntimeSampleRow`. -
      `testdata/otel-traces-sample.jsonl`: remove the
      `e2e.hydration_runtime_json` attribute from each span that carries it, and
      the `cacheWarmth` key from each `e2e.navigation_top_json` entry.

- [x] **Step 3: Build + clippy, verify no dead code and it compiles.** Run:
      `cargo clippy -p xtask --all-targets` Expected: PASS, no
      `dead_code`/unused-import warnings (proves no orphaned shared helper and
      all cut symbols are gone).

- [x] **Step 4: Run the xtask trace tests, verify green.** Run:
      `cargo nextest run -p xtask traces` Expected: PASS — the surviving section
      tests and `render_emits_sections_in_canonical_order` (now 11 markers)
      pass; the four deleted tests are gone (spec AC1, AC5). Section 4 still
      asserts the old `commit_to_hydration` name here (renamed in Task 3).

- [x] **Step 5: Eyeball the analyze output for the surviving-section shape (spec
      AC1).** Run:
      `cargo run -p xtask -- traces analyze xtask/src/traces/testdata/otel-traces-sample.jsonl`
      Expected: output shows none of the four removed section titles (cache
      warmth, hydration-vs-API, phase components, hydration runtime).

- [x] **Step 6: Residue grep over the consumer (spec AC3).** Run:
      `rg -n "cache_warmth|CacheWarmth|hydration_vs_api|HydrationVsApi|hydration_runtime|HydrationRuntime|NAV_PHASE_COMPONENTS|HYDRATION_RUNTIME_COMPONENTS|PhaseSample|PhaseTarget|PhaseProject|RuntimeSample|RuntimeProject" xtask/src/traces/`
      Expected: no matches (all cut-section identifiers gone).

- [x] **Step 7: Commit** — committed `7dc68a60`, full gate green.

```bash
git add xtask/src/traces/analyze.rs xtask/src/traces/render.rs xtask/src/traces/testdata/otel-traces-sample.jsonl
git commit -m "refactor(xtask/traces): remove CSR-OBE hydration report sections (#228)"
```

---

## Task 3: Prune null `NAV_PHASES` phases + rename to `commit_to_mount`

**Files:**

- Modify: `xtask/src/traces/analyze.rs:339-355` (`NAV_PHASES`), `:990-995`
  (section-4 test `navigation_phase_and_targets`)
- Modify: `xtask/src/traces/testdata/otel-traces-sample.jsonl` (the
  `e2e.navigation_top_json` payload's `commitToHydrationMs` keys + the now-dead
  phase-component keys)

**Interfaces:**

- Consumes: the section set left by Task 2 (the four sections and their tests
  are already gone, so nothing but `NAV_PHASES` reads `commitToHydrationMs`).
- Produces: `NAV_PHASES` reduced from 9 to 5 entries — `navigation.total`,
  `navigation.request`, `navigation.commit_to_domcontentloaded`,
  `navigation.commit_to_mount` (renamed), `navigation.domcontentloaded_to_load`.
  The renamed tuple is `("navigation.commit_to_mount", "commitToMountMs")`. Task
  4 produces the matching emitter key.

- [x] **Step 1: Update the section-4 test to the new phase name (red).** In
      `analyze.rs` `navigation_phase_and_targets` (:990-995), change the lookup
      `r.name == "navigation.commit_to_hydration"` →
      `"navigation.commit_to_mount"` and the
      `.expect("commit_to_hydration present")` message →
      `"commit_to_mount present"`. Leave the `max_ms == 400.0` assertion.

- [x] **Step 2: Run the test, verify it fails.** Run:
      `cargo nextest run -p xtask navigation_phase_and_targets` Expected: FAIL —
      no phase named `navigation.commit_to_mount` yet (fixture still carries
      `commitToHydrationMs`, `NAV_PHASES` still says `commit_to_hydration`).

- [x] **Step 3: Prune + rename `NAV_PHASES`.** Edit the array (`analyze.rs:339`)
      to the 5 entries above: delete the `load_to_hydration`, `wasm_init`,
      `leptos_hydrate`, `post_hydrate_effects` tuples; rename the
      `("navigation.commit_to_hydration", "commitToHydrationMs")` tuple to
      `("navigation.commit_to_mount", "commitToMountMs")`. Update the array
      length annotation `[(&str, &str); 9]` → `; 5]`.

- [x] **Step 4: Rename + trim the fixture JSONL.** In
      `testdata/otel-traces-sample.jsonl`, within every
      `e2e.navigation_top_json` value: rename the field `commitToHydrationMs` →
      `commitToMountMs` (leave the value — the firefox nav's 400 is what the
      test asserts), and delete the now-fully-dead keys `loadToHydrationMs`,
      `wasmInitMs`, `leptosHydrateMs`, `postHydrateEffectsMs` (no longer read by
      the reduced `NAV_PHASES`).

- [x] **Step 5: Run the test, verify it passes.** Run:
      `cargo nextest run -p xtask navigation_phase_and_targets` Expected: PASS.

- [x] **Step 6: Residue grep + full output check (spec AC2/AC3).** Run:
      `rg -n "commit_to_hydration|commitToHydration|load_to_hydration|loadToHydration|leptos_hydrate|post_hydrate|wasm_init" xtask/src/traces/`
      Expected: no matches. Then
      `cargo run -p xtask -- traces analyze xtask/src/traces/testdata/otel-traces-sample.jsonl`
      — section 4 lists phase `commit_to_mount` and none of the four pruned
      phases.

- [x] **Step 7: Commit** — committed `83954835`, full gate green.

```bash
git add xtask/src/traces/analyze.rs xtask/src/traces/testdata/otel-traces-sample.jsonl
git commit -m "refactor(xtask/traces): prune null hydration phases, rename commit_to_hydration->commit_to_mount"
```

---

## Task 4: Strip dead attributes + `__jaunder_perf` plumbing from the emitter

**Files:**

- Modify: `end2end/tests/fixtures.ts` — the `PagePerfSummary.hydrationRuntime`
  field (:50-58), the `HydrationPerfPayload` type (:61-69), the
  `NavigationRecord` hydration-perf fields, the `__jaunder_perf` reads that
  populate `hydrationRuntime` (~:580-668) and the per-navigation `wasmInitMs` /
  `leptosHydrateMs` / `postHydrateEffectsMs` reads (~:354-357), the navigation-
  summary object (:703-728), and the attribute list (:765-848)

**Interfaces:**

- Produces: `e2e.navigation_top_json` no longer carrying `loadToHydrationMs` /
  `wasmInitMs` / `leptosHydrateMs` / `postHydrateEffectsMs`, carrying
  `commitToMountMs` (was `commitToHydrationMs`); no `e2e.hydration_runtime_json`
  attribute; the `navigation.lifecycle` span attribute renamed
  `navigation.commit_to_hydration_ms` → `navigation.commit_to_mount_ms` and the
  four null
  `navigation.{load_to_hydration,wasm_init,leptos_hydrate, post_hydrate_effects}_ms`
  attributes removed.

_No unit test exercises the emitter; its contract is TypeScript type-checking
(`tsc --noEmit`) + the Task 6 e2e run (lint/format runs via
`cargo xtask check`). `end2end/package.json` has no `lint`/`typecheck` scripts,
so invoke `tsc` directly. Keep the change mechanical and type-driven._

- [x] **Step 1: Rename the load-bearing camelCase field.** In `fixtures.ts`,
      rename `commitToHydrationMs` → `commitToMountMs` at the
      `NavigationSummary` **type** field (:94), its `const` declaration
      (:703-706), and the returned navigation-summary object (:722). This is the
      field the xtask parser reads via `NAV_PHASES` (Task 3) — must match
      `commitToMountMs` exactly.

- [x] **Step 2: Rename the OTLP span attribute (cosmetic).** Change
      `otlpAttribute("navigation.commit_to_hydration_ms", navigation.commitToMountMs)`
      (:828-831) → `"navigation.commit_to_mount_ms"`, updating the field
      reference to `commitToMountMs`.

- [x] **Step 3: Remove the dead attribute emissions.** Delete from the
      attribute/event lists: the `e2e.hydration_runtime_json` attribute
      (:765-768); and the `navigation.load_to_hydration_ms` (:836-839),
      `navigation.wasm_init_ms` (:840), `navigation.leptos_hydrate_ms`
      (:841-844), `navigation.post_hydrate_effects_ms` (:845-848) span
      attributes.

- [x] **Step 4: Remove the dead plumbing + types.** Delete, in `fixtures.ts`: -
      the summary object's `loadToHydrationMs` (:711-714 compute, :724 field)
      and `wasmInitMs` / `leptosHydrateMs` / `postHydrateEffectsMs` fields
      (:726-728), and their `NavigationSummary` **type** fields (:96,
      :98-100); - the `NavigationRecord` hydration-perf fields (`wasmInitMs`,
      `leptosHydrateMs`, `postHydrateEffectsMs`) + the `~:354-357` `perf.*`
      reads that set them (keep the `navigation.hydratedMs = nowMs` assignment
      at :352); - **every remaining `__jaunder_perf` site** so Step 6's grep is
      clean: the `globalScope` type declaration (:371), the `notifyHydration`
      polling reads (:390, :404-406), and the `~:580-668` block that computes
      `hydrationRuntime`; - the `PagePerfSummary.hydrationRuntime` field
      (:50-58) and the `HydrationPerfPayload` type (:61-69).

      **Keep** `hydratedMs` and the `data-hydrated` marker handler —
      `commitToMountMs` depends on `hydratedMs` (spec: marker stays, only its name
      is deferred to the Task 1 follow-on).

- [x] **Step 5: Type-check, verify clean (spec AC4).** Run:
      `cd end2end && npx tsc --noEmit` Expected: PASS — no unused-symbol / type
      errors; no residual reference to the removed fields. (Formatting/lint is
      enforced by `cargo xtask check` in Task 6.)

- [x] **Step 6: Grep for residue.** Run:
      `rg -n "hydration_runtime|commitToHydration|commit_to_hydration|load_to_hydration|loadToHydration|wasmInitMs|leptosHydrateMs|postHydrateEffectsMs|__jaunder_perf|HydrationPerfPayload|hydrationRuntime" end2end/tests/fixtures.ts`
      Expected: no matches.

- [x] **Step 7: Commit** — committed `b60ad1dd`, full gate green.

```bash
git add end2end/tests/fixtures.ts
git commit -m "refactor(e2e): drop dead hydration span attributes, rename commit_to_mount (#228)"
```

---

## Task 5: Update `docs/observability.md`

**Files:**

- Modify: `docs/observability.md` — the navigation-summary description (:22-25),
  the section list (:91-95), and any `commit -> hydration` phrasing describing
  the surviving section 4 phase (:153)

**Interfaces:** none (docs only).

- [x] **Step 1: Remove the four removed sections from the section list.** Delete
      the bullets describing: "navigation `commit -> hydration` split by
      `cacheWarmth`" (:91), the per-navigation hydration component hotspots
      (`wasm_init`/`leptos_hydrate`/`post_hydrate_effects`/`commit_to_hydration`)
      (:93-94), and the hydration runtime component hotspots from
      `e2e.hydration_runtime_json` (:95). Also remove any "hydration budget vs
      API budget" line if present.

- [x] **Step 2: Rename surviving `commit -> hydration` references to
      `commit -> mount`.** Update the navigation-summary line (:22-25, "includes
      `commit -> hydration` timing…") and the phase reference at :153 to
      `commit -> mount` / `commit_to_mount`, consistent with ADR-0040. Leave the
      `hydrationHeavy*` (:168-170, :296-300) and `data-hydrated` (:319)
      references untouched — those renames are the Task 1 follow-ons.

- [x] **Step 3: Re-read the edited regions** to confirm no dangling reference
      implies a removed section still prints (spec AC6).

- [x] **Step 4: Commit** — committed `76162edd` (prettier reflowed; re-staged).

```bash
git add docs/observability.md
git commit -m "docs(observability): drop CSR-OBE hydration trace sections, commit_to_mount (#228)"
```

---

## Task 6: Full gate

**Files:** none.

**Interfaces:** none — integration gate proving the whole cut (spec AC7).

- [ ] **Step 1: Run the full local gate.** Run: `cargo xtask validate` (Bash
      background mode — long; static + clippy + coverage + e2e across
      `{sqlite,postgres}×{chromium,firefox}`). Expected: green. The e2e run
      exercises `fixtures.ts` end-to-end, proving the emitter changes don't
      break OTel capture or any test. Confirm completion via the
      `xtask-done: … ok=true` sentinel.

- [ ] **Step 2: If green, the branch is ready for jaunder-ship.** No commit —
      `validate` is verify-only.

---

## Self-review

- **Spec coverage:** AC1 → Task 2 (steps 4-6); AC2 → Task 3 (steps 5-6); AC3 →
  Task 2 (step 3 clippy + step 6 grep) + Task 3 (step 6 grep); AC4 → Task 4
  (steps 1-7); AC5 → Task 2 (steps 1,4) + Task 3 (steps 1,5); AC6 → Task 5; AC7
  → Task 6; AC8 → Task 1. All covered.
- **Type consistency:** the rename target is `commitToMountMs` (camelCase field)
  everywhere — Task 3 (`NAV_PHASES` tuple + xtask fixture), Task 4
  (`fixtures.ts` type + const + object) — and `navigation.commit_to_mount` /
  `_ms` for the labels/attributes. No mismatched spellings across tasks.
- **Ordering:** Task 2 (remove sections + their tests) precedes Task 3 (rename
  the `commitToHydrationMs` key), so no still-present test reads the renamed key
  at a commit gate.
- **No placeholders:** every step names exact files/lines, exact `cargo`/`rg`
  commands, and expected pass/fail.
