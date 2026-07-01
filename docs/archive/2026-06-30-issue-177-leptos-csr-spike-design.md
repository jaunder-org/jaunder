# Spike #177 — Confirm leptos-CSR eliminates the concurrent-SSR panic (gate)

**Issue:** jaunder-org/jaunder#177
**Milestone:** 8 — Off concurrent SSR (web re-architecture v1)
**Status:** design approved 2026-06-30

## Goal (the gate)

Empirically confirm that a **leptos-CSR** build does **not** hit the concurrent-SSR
reactive-disposal panic (*"Tried to access a reactive value that has already been
disposed"*) under concurrent load, **before** committing the milestone to the full
migration. Output: a go/no-go on leptos-CSR.

- **GO** — a `workers:4` / `fullyParallel` campaign on postgres+chromium of **~30**
  runs with **zero** `already been disposed` panics → leptos-CSR confirmed for the
  milestone (#178–#182 proceed).
- **NO-GO** — any panic → escalate, re-open the framework decision toward Dioxus, and
  record exactly what reproduced (this would contradict the root-cause analysis, so
  capture it precisely).

### Why this should pass (root-cause prediction)

The #173 panic is **server-side concurrent reactive render** — `reactive_graph`'s
process-global arena gets a node disposed/reused by a sibling request while another
in-flight SSR render still references it. **CSR has no server-side reactive render**:
each browser tab is single-threaded with one reactive runtime, no concurrent render,
no SSR-owner-across-`await`. So CSR is *predicted* safe; this spike **confirms** it.
Full diagnosis: `docs/issue-173-findings-and-pivot-handoff.md` (on the issue-173
branch).

### Confidence math (why ~30)

Baseline SSR panicked ~12% per run (first panic ~run 7). If a 12%-per-run defect were
still present, 30 clean runs would miss it with probability `0.88^30 ≈ 1.7%` — ~98%
confidence. Document the exact run count and result per the acceptance criteria.

## Architecture (target, per inbound-data-handling §4)

The milestone moves to: **server = UI-free** (server-fn endpoints + a non-reactive
projector + static wasm/CSR assets); **web client = leptos-CSR**; **content render = a
shared pure non-reactive fn**. This spike implements only the **CSR client + a static
shell server path** — *not* the projector (#178), *not* render-coincidence/blob-seeding
(#179), *not* SSR removal (#180). It is the confirming gate.

## What lands (reusable, feature-gated)

The default SSR build and CI matrix must remain **untouched** — every change is behind a
`csr` build feature or a parallel nix attr.

1. **`web` crate `csr` feature.** Compiles `App` + pages for client-side render;
   excludes the ssr-only deps (`leptos_axum`, `axum`, `storage`, …). The ~50
   `#[server]` fns compile to **client fetch stubs** that POST to the existing
   `/api/{*fn_name}` endpoint (leptos's standard non-`ssr` behavior). No component
   rewrites — the `App`/pages already run client-side under hydration today, so
   `mount_to_body(App)` exercises the same component tree.

2. **New `csr/` wasm crate** (mirrors `hydrate/`). Entry calls
   `leptos::mount::mount_to_body(App)` — **no hydration**. Drops the SSR/hydration perf
   instrumentation; instead emits a **CSR-ready signal** (a `data-csr-mounted`
   attribute on `<body>`, or equivalent) the e2e harness can wait on.

3. **Server `csr` build feature.** Under `csr`, `create_router` serves a static
   `index.html` **shell** (loads `jaunder.js`, boots the wasm) as the SPA fallback for
   all app routes, keeps `/api/{*fn_name}` server fns and static-asset serving, and
   wires **no** `leptos_routes_with_context`. This makes "no reactive SSR in the request
   path" a **structural** property of the CSR binary, not a runtime claim — exactly what
   the gate must prove. The per-request-arena middleware (`server/src/arena.rs`) is not
   on the CSR path.

4. **Nix + e2e wiring.**
   - A parallel CSR site build: the `csr` wasm bundle (wasm-bindgen `--target web`,
     renamed to `jaunder.js`/`jaunder_bg.wasm` like the hydrate bundle) + the `csr`
     server binary + the static `index.html` shell.
   - An e2e variant that boots the CSR server in the NixOS VM.
   - VM bumped to `virtualisation.cores = 4` + `memorySize = 6144` (1 vCPU →
     *false* hydration-timeouts; 2 GB OOMs at 4 browsers).
   - A CSR Playwright project at `workers:4` + `fullyParallel:true`, postgres+chromium
     (postgres latency widens the concurrency window; sqlite is too fast; chromium is
     the most sensitive surface).

5. **e2e spec adaptation.** The 12 existing specs interact with rendered DOM, which CSR
   produces client-side. Swap hydration-marker waits (`mark_hydrated` / `hydration.ts`)
   for the CSR-ready signal; keep the nav/list/form flows — they generate the concurrent
   server-fn load that drives the (now-absent) reactive path. Only the CSR project runs
   the campaign; the default SSR matrix is left alone.

## What does NOT land

- **The campaign harness is a scratch script** (kept in session scratchpad, matching the
  #173 approach) — not committed. It loops the CSR e2e ~30× with `nix build --rebuild`
  (nix caches a passing e2e), classifies each run PANIC / PASS / OTHER by grepping the
  VM journal for `already been disposed`, and prints a tally.

## What merges

- The feature-gated CSR scaffolding (items 1–5 above).
- A **findings doc** (`docs/issue-177-csr-spike-findings.md`) recording the run count
  and result (GO/NO-GO), and — if NO-GO — exactly what reproduced.
- An **ADR** confirming **leptos-CSR** for the web leg (narrows ADR-0002 from "anything"
  to leptos-CSR; relates to ADR-0039 / #173). Add its row to the ADR table in
  `docs/README.md`. Next number after the current highest.

## Faithfulness checks (the gate is only valid if these hold)

- **No reactive SSR in the CSR server binary** — verify the `csr`-feature build links no
  `leptos_routes_with_context` / `leptos_axum` reactive render (structural, via the
  feature gate).
- **The campaign actually exercises concurrency** — `workers:4`, `fullyParallel`,
  postgres latency: the same recipe that reproduced SSR panics ~12% of runs.
- **CSR-ready, not server-painted** — specs wait on the CSR mount signal; CSR first
  paint is blank-until-boot (the projector that paints initial content is #178).

## Risks / gotchas (from the #173 handoff)

- **wasm-bindgen ↔ leptos version lock.** Stay on the known-good pairing on `main`
  (verify exact leptos / wasm-bindgen versions during planning). **Do not** bump to
  leptos 0.8.20 — it regresses our rendering independent of wasm-bindgen. The Nix
  `wasm-bindgen-cli` must match Cargo's `wasm-bindgen` exactly.
- **VM sizing** — `cores=4` / `memorySize=6144`, per above.
- **Nix e2e cache** — force re-runs with `nix build --rebuild` each campaign iteration.
- **Server fns still run server-side** under CSR (as plain POST endpoints, no reactive
  render tree). Whether that residual server-fn machinery touches `reactive_graph` under
  concurrency is precisely what the empirical campaign settles — do not assume; measure.

## Out of scope (later milestone issues)

Public projector / flash-free render coincidence (#178); CSR-as-default + seeding from
the projector blob (#179); removing leptos_axum reactive render entirely / closing #173
(#180); authenticated-owner flash handling (#181); flipping the production e2e matrix to
`workers:4` and updating ADR-0039 (#182).

## Acceptance

- A multi-run `workers:4` campaign (~30 runs, postgres+chromium) with **zero**
  `already been disposed` panics; run count and result documented in the findings doc.
- CSR scaffolding is feature-gated; the default SSR build and CI matrix are unchanged
  (`cargo xtask check` green on the default build).
- ADR recorded (leptos-CSR confirmed) and listed in `docs/README.md`.
- If panics persist under CSR → escalate to Dioxus, findings doc records the
  reproduction.
