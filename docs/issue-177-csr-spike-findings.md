# Issue #177 — leptos-CSR spike: findings & verdict

**Verdict: GO.** A leptos-CSR build does **not** hit the concurrent-SSR
reactive-disposal panic (#173) under the exact recipe that reproduced it. The
milestone-8 re-architecture (drop reactive SSR from the server request path;
CSR-only web client) is confirmed on empirical grounds.

## The gate

Empirically confirm, before committing the milestone to the migration, that a
CSR build is free of the _"Tried to access a reactive value that has already
been disposed"_ panic under concurrent load — the #173 class that is unfixable
at the app level (upstream leptos #4590 NOT_PLANNED).

## Recipe (the #173 reproduction, applied to CSR)

- **Build:** a feature-gated leptos-CSR variant — `web` `csr` feature + `csr/`
  wasm crate (`mount_to_body`, no hydration) + a `jaunder --features csr` server
  that serves a static SPA shell (no `leptos_routes_with_context`; `/api` server
  fns retained). Nix derivations `csrWasm`/`csrWasmBundle`/`jaunderBinCsr`/
  `csrSite`; e2e check `csr-e2e-postgres-chromium`.
- **Drive:** Playwright **`workers:4` + `fullyParallel:true`**, backend
  **postgres** (latency widens the Suspense/concurrency window; sqlite is too
  fast), browser **chromium** (most sensitive). VM **`cores=4` +
  `memorySize=6144`** (1 vCPU gives false hydration-timeouts; 2 GB OOMs at 4
  browsers).
- **Campaign:** 30 fresh VM executions via `nix build --rebuild` (nix caches a
  passing e2e; `--rebuild` forces re-execution). Classified by **log content**,
  not exit code — `--rebuild` always exits non-zero with a cosmetic "may not be
  deterministic" output diff (VM journal timestamps), so PASS/PANIC is read from
  the log: `already been disposed` → PANIC; `<N> passed (` with no failures →
  PASS.
- Versions (the known-good pairing; **not** leptos 0.8.20, which regresses
  rendering): leptos 0.8.19 (resolved from the `^0.8.2` pin) / reactive_graph
  0.2.14 / tachys 0.2.15 / wasm-bindgen(-cli) 0.2.121.

## Result

| Campaign                          | Runs   | PASS   | PANIC | OTHER | First failure |
| --------------------------------- | ------ | ------ | ----- | ----- | ------------- |
| CSR, workers:4, postgres+chromium | **30** | **30** | **0** | **0** | none          |

Every run reported **66/66 Playwright tests passed**; the shared ADR-0032
zero-panic gate (`journalctl` scan for `panicked at`) was clean on all 30. Grep
of all 30 run logs: **zero** `already been disposed`, **zero** `panicked at`.

Plus 3 additional clean runs during bring-up (one `workers:1`, two `workers:4`),
all 66/66 — **33 clean runs total**, no disposal panic ever.

## Contrast with the SSR baseline (#173 handoff)

| Config (leptos 0.8.19, workers:4, postgres+chromium)                        | Result                     |
| --------------------------------------------------------------------------- | -------------------------- |
| SSR baseline (no fix)                                                       | panic ~run 7, ~12% per run |
| SSR + per-request arena                                                     | reduced — panic ~run 12    |
| SSR + arena + tachys 0.2.16 + read_signal! + untrack (full app-level stack) | still panics ~run 2        |
| **CSR (this spike)**                                                        | **0 panics / 30 runs**     |

Every app-level SSR mitigation still panicked within ~1–12 runs. CSR — which
removes server-side reactive render entirely — is clean across 30.

## Confidence

If a 12%-per-run defect were still present, 30 clean runs would miss it with
probability `0.88^30 ≈ 1.8%` — ~98% confidence the disposal class is eliminated.
This matches the root-cause prediction: the panic is a **server-side concurrent
reactive render** race in `reactive_graph`'s process-global arena; CSR has no
server-side reactive render (each browser tab is single-threaded, one runtime,
no concurrent render, no SSR-owner-across-`await`), so the class has no home.

## Scope note

Stabilization only: leptos **server functions** are retained as the data API;
the server keeps `web/ssr` for the server-fn bodies. Only the reactive **page
render** leaves the request path. `jaunder-core`, the sync engine, and a clean
REST API are later (§5–6) work, not this milestone.

## Consequence

leptos-CSR is confirmed. The Dioxus fallback is **not** needed. Proceed with the
milestone: public projector (#178), CSR-as-default client (#179), server UI-free
/ close #173 (#180), authed-owner flash (#181), re-enable parallel e2e (#182).
See ADR-0040.
