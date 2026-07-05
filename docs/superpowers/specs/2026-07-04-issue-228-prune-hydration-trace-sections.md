# Spec — issue #228: remove CSR-OBE hydration sections from `xtask traces analyze`

- Issue: [#228](https://github.com/jaunder-org/jaunder/issues/228) (milestone
  "Devtool migration"; label `tooling`)
- Follow-on from #32 (which ported all twelve OTel trace report sections
  faithfully; archived spec/plan at
  `docs/archive/2026-07-04-issue-32-traces-analyze-xtask-{spec,plan}.md`)
- Governing decision: **ADR-0040** (leptos-CSR, drop concurrent reactive SSR) —
  the web client is `mount_to_body`, **no hydration**; the SSR/hydration layer
  is deleted.

## Problem

`xtask traces analyze` reports twelve sections ported verbatim from the Node
tooling. Four are **hydration-framed** and, after the CSR re-architecture
(ADR-0040), are overcome-by-events: the SSR hydration instrumentation that fed
them was deleted, so the data is now either always-null or a redundant reslice
of a signal a surviving section already reports. They must be audited and
removed, along with the now-dead span attributes that only exist to feed them.

### Findings that fix the exact cut (CSR context in hand)

- `globalThis.__jaunder_perf` is **set nowhere** in the repo (`web/` search
  empty). It is only _read_ by `end2end/tests/fixtures.ts`. Every metric derived
  from it is therefore null post-CSR:
  - `e2e.hydration_runtime_json` → all fields null (feeds **section 10**).
  - navigation `wasmInitMs`, `leptosHydrateMs`, `postHydrateEffectsMs` → null
    (feed **section 9** and the like-named `NAV_PHASES` phases in **section
    4**).
- `navigation.commit_to_hydration_ms` is **still live** — computed as
  `hydratedMs - committedMs` (`fixtures.ts`), where `hydratedMs` comes from the
  `body[data-hydrated]` mount-readiness marker set by `csr/src/lib.rs`. It now
  measures **commit → CSR mount**, not hydration.
- Surviving **section 4** (`NAV_PHASES`, `analyze.rs:339`) **already reports**
  `commit_to_hydration` as a navigation phase. So sections **5**
  (commit→hydration by cache warmth) and **8** (hydration budget vs API budget)
  add no _unique_ signal — they only reslice a phase section 4 already prints
  (by a hardcoded `cacheWarmth` flag, `nav.id===1 ? 'cold':'warm'`, and vs API
  request time).
- `NAV_PHASES` itself lists four dead-or-redundant hydration phases:
  `wasm_init`, `leptos_hydrate`, `post_hydrate_effects` are **always-null** (fed
  by the never-set `__jaunder_perf`); `load_to_hydration` is **live but
  redundant** — computed as `hydratedMs - loadMs` (both live), it is a real
  page-load→mount interval, but recoverable by arithmetic from the surviving
  phases
  (`commit_to_mount − commit_to_domcontentloaded − domcontentloaded_to_load`)
  and, like the other three, hydration-framed (OBE naming per ADR-0040).

## Decisions (from the design interview)

1. **Cut all four candidate sections** — 5, 8, 9 (a–c), 10 (a–b). The only live
   signal among them (`commit_to_hydration`) survives in section 4, so nothing
   real is lost. Twelve sections → eight.
2. **Blast radius = consumer + dead emitters.** Remove both the report sections
   in `xtask/src/traces/` **and** the now-dead attribute emission in
   `end2end/tests/fixtures.ts`, including the `__jaunder_perf` plumbing that
   only fed them. No orphaned data path left behind.
3. **Prune section 4's four dead-or-redundant hydration phases**
   (`load_to_hydration` — live but arithmetically redundant; `wasm_init`,
   `leptos_hydrate`, `post_hydrate_effects` — always-null) from `NAV_PHASES` and
   their emitter attributes.
4. **Rename the surviving live phase** `commit_to_hydration` → `commit_to_mount`
   across emitter (`fixtures.ts`), parser (`NAV_PHASES` + the JSON field the
   navigation summary carries), section 4 label, tests, and docs — dropping the
   misleading "hydration" word per ADR-0040.

### Explicitly out of scope (separable concerns — filed as follow-ons)

These are broader renames touching many unrelated files; folding them in would
smuggle unauthorized churn into an OBE-removal cycle. The plan's first task
files them as issues:

- Rename the `hydrationHeavy*` timeout helpers (`hydrationHeavyTimeoutMs`,
  `hydrationHeavyFirstNavigationTimeoutMs`, and the scale constants) to
  mount-oriented names. `docs/observability.md:169` already flags this naming as
  a post-CSR misnomer. Used across many e2e spec files.
- Rename the `body[data-hydrated]` mount-readiness marker / `waitForHydration` /
  `end2end/tests/hydration.ts` to mount-oriented names (`csr/src/lib.rs`,
  `hydration.ts`, `posts.spec.ts`, docs).

The `body[data-hydrated]` marker itself stays as-is this cycle — it is the live
mount-readiness signal that feeds `commit_to_mount`; only its _name_ is
deferred.

## Acceptance criteria (observable)

**Consumer — `xtask/src/traces/{analyze,render}.rs`:**

- **AC1** `cargo xtask traces analyze <fixture>` output contains **none** of:
  the commit→hydration cache-warmth table, the "hydration budget vs API budget"
  table, the navigation phase-component sections (samples/targets/by-project),
  or the hydration-runtime-component sections. Eight sections remain (the former
  1, 2, 3, 4, 6, 7, 11, 12).
- **AC2** Section 4 (navigation phase hotspots) output no longer lists the
  phases `load_to_hydration`, `wasm_init`, `leptos_hydrate`,
  `post_hydrate_effects`, and the former `commit_to_hydration` phase is labeled
  **`commit_to_mount`**.
- **AC3** `rg` over `xtask/src/traces/` finds no residual `commit_to_hydration`,
  `hydration_runtime`, `leptos_hydrate`, `post_hydrate_effects`, `wasm_init`,
  `load_to_hydration`, cache-warmth, or hydration-vs-API identifiers. The dead
  constants `NAV_PHASE_COMPONENTS` and `HYDRATION_RUNTIME_COMPONENTS`, the cut
  sections' `Row`/`Display` structs and analyze fns, and any parse
  structs/fields that only those sections used are gone. No dead code (clippy
  clean).

**Emitter — `end2end/tests/fixtures.ts`:**

- **AC4** `fixtures.ts` emits neither `e2e.hydration_runtime_json` nor
  `navigation.{wasm_init,leptos_hydrate,post_hydrate_effects,load_to_hydration}_ms`.
  The rename has **two independent sites that must both change**:
  - **The load-bearing parser contract:** the camelCase field
    `commitToHydrationMs → commitToMountMs` on the navigation-summary object
    (`fixtures.ts` type at :94, computed `hydratedMs - committedMs` at :703,
    written into the object at :722; serialized into `e2e.navigation_top_json`
    at :773) **and** the matching second tuple element in `NAV_PHASES`
    (`analyze.rs:346`, `"commitToHydrationMs" → "commitToMountMs"`). This pair
    is what makes the parser resolve the surviving phase — if the field is
    renamed on one side only, section 4's `commit_to_mount` phase silently reads
    null.
  - **The cosmetic OTLP span attribute:**
    `"navigation.commit_to_hydration_ms" → "navigation.commit_to_mount_ms"` on
    the `navigation.lifecycle` span (`fixtures.ts:829`) and the `NAV_PHASES`
    first tuple element
    (`"navigation.commit_to_hydration" → "navigation.commit_to_mount"`). Nothing
    in xtask reads this attribute; renamed only for post-CSR naming honesty.

  The `__jaunder_perf` `hydrationRuntime` computation/reads and now-unused
  TypeScript types/fields that only fed the removed attributes are deleted.
  TypeScript type-checks and lints clean.

**Tests & docs:**

- **AC5** The four section-specific xtask tests
  (`cache_warmth_by_warmth_and_project`, `hydration_vs_api_budget`,
  `navigation_phase_components`, `hydration_runtime_components`) are removed;
  the section-4 test asserts the phase under its new `commit_to_mount` name; the
  `testdata/otel-traces-sample.jsonl` fixture has its `commitToHydrationMs` JSON
  keys renamed to `commitToMountMs` (and any rows that exist only to populate
  the four removed sections trimmed) so the surviving-section assertions pass;
  the xtask test suite is green. (The parser-resolution guarantee for
  `commit_to_mount` lives in _this_ unit test, not in AC7's e2e run — e2e never
  invokes `xtask traces analyze`.)
- **AC6** `docs/observability.md` no longer describes the four removed sections
  as present, and its navigation-summary / phase references use
  `commit_to_mount` (not `commit → hydration`). No dangling reference implies a
  removed section still prints.

**Gate:**

- **AC7** `cargo xtask validate` is green (static + clippy + coverage + e2e
  across all `{sqlite,postgres}×{chromium,firefox}` combos) — the e2e run proves
  the `fixtures.ts` capture changes don't break OTel capture or any test. (It
  does _not_ exercise the analyze parser; that guarantee is AC5's.)
- **AC8** Both separable renames are tracked as issues before the removal work
  lands: the `hydrationHeavy*` rename (**#224**, pre-existing) and the
  `data-hydrated`-marker rename (**#251**, filed this cycle).

## Non-goals

- No new sections or metrics; no behavior change to the surviving eight beyond
  the phase pruning/rename above.
- No change to the `body[data-hydrated]` marker mechanism (only its name is
  deferred to a follow-on).
- No change to OTel collection/export outside the specific dead attributes.
