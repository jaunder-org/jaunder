# 0040. Web rendering: leptos-CSR (drop concurrent reactive SSR)

- Status: accepted
- Date: 2026-07-01
- Issue: #177 (milestone 8 — "Off concurrent SSR"); narrows ADR-0002; relates
  #173, ADR-0016, ADR-0032, ADR-0039

## Context

The Leptos stack (ADR-0002: SSR + hydration) is **unstable under concurrent
load**. Turning on parallel e2e surfaced the concurrent-SSR reactive-disposal
class (#173): _"Tried to access a reactive value that has already been
disposed"_ — `reactive_graph`'s process-global arena has a node disposed/reused
by a sibling request while another in-flight SSR render still references it. It
is **not fixable at the app level** (per-request `sandboxed-arenas` + `tachys`
deferred `OwnedView` drop + untracked reads all only _reduce_ it; ~41% of
renders detach from the middleware owner because `leptos_axum` spawns
streaming-render fragments outside it), and upstream marks the exact panic
**NOT_PLANNED** (leptos #4590). The same render-under-concurrency entanglement
produced the ADR-0016 SSR owner/context panics (#124, #138).

The root cause is precise: the instability is the **means** by which we got
graceful degradation — **isomorphic / universal rendering**, i.e. running the
SPA's own reactive components in the concurrent server request path. Take the
reactive runtime out of the server and the class is gone. And the assumption
that forced isomorphic SSR — that the crawlable page-set and the app-rich
page-set are the same set — is false for Jaunder: the public reading surface
(profile/permalink/feed) is anonymous, crawlable, read-only; the owner's cockpit
is authenticated and may require JS. Degradation is a requirement of the public
surface **only**.

## Decision

Narrow ADR-0002 from "Leptos SSR + hydration" to **leptos client-side rendering
(CSR)** for the web leg, and move UI rendering off the server:

- **Server = UI-free:** server-fn endpoints (retained as the data API) + a thin
  **non-reactive** HTML projector for public routes + static wasm/CSR assets. No
  `leptos_axum` reactive page render in the request path.
- **Web client = leptos-CSR:** `mount_to_body`, no hydration. This **reuses the
  existing Leptos components** — the SSR/hydration layer is deleted, the UI is
  not rewritten.
- **Content render = a shared pure, non-reactive fn** used by both the projector
  and the CSR client, so the two renders **coincide** (flash-free first paint)
  without a reactive graph on the server (render coincidence is client-side, one
  tab — unrelated to the server-concurrency bug). Sharing a _reactive component_
  rendered to string on the server is the trap door back to isomorphic SSR and
  is prohibited.

**Dioxus was rejected:** a near-total rewrite with no compensating edge now that
mobile is native-over-FFI (SwiftUI/Compose via uniffi) and desktop is
low-priority. CSR-only sidesteps the entire #173 class (single-threaded per tab,
no concurrent render, no SSR-owner-across-`await`) while keeping the component
code.

The decision was made **contingent on a spike**, now **confirmed**: a CSR build
ran the #173 reproduction recipe (workers:4, fullyParallel, postgres+chromium,
4-vCPU/6 GB VM) **30 times with zero `already been disposed` panics** (SSR
baseline panicked ~12% per run, first panic ~run 7; every app-level SSR
mitigation still panicked within ~1–12 runs). ~98% confidence the class is
eliminated. Evidence: `docs/issue-177-csr-spike-findings.md`.

## Scope

**Stabilization only.** Leptos **server functions** are retained as the data API
(the server keeps `web/ssr` for the server-fn bodies); only the reactive **page
render** leaves the request path. `jaunder-core`, the sync engine, and a clean
REST API are later (§5–6) work, not this milestone.

## Consequences

- The #173 disposal class is structurally eliminated once the server-side
  reactive render is removed (**closes #173 at #180**); no app-level patching
  needed.
- Delivered incrementally over milestone 8: public projector (#178),
  CSR-as-default client seeding from the projector blob (#179), server UI-free /
  cutover (#180), authed-owner flash handling (#181), and re-enabling parallel
  e2e (#182 — unblocks #61 and lifts ADR-0039's `workers:1` stopgap).
- The public first paint is flash-free via render coincidence; the authenticated
  owner stays flash-free via enhance-don't-replace + pre-paint auth detection,
  cockpit on its own route (#181).
- Strengthens the "one Rust core" thesis: server, projector, and web client all
  sit on the shared model + pure render fn. Cost: the public web SPA must be
  Rust (to share the render fn by construction) — a Flutter/TS SPA would
  re-implement content rendering and hand-maintain markup parity; rejected.
