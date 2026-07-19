# ADR-0070: web verticals split host/wasm at the file level — wasm-only `component.rs`

- Status: accepted
- Note: amended 2026-07-18 (#530) — `#[server]` endpoints and wire types move
  from `mod.rs` to `api.rs`; `mod.rs` is module wiring only
- Note: amended 2026-07-18 (#527) — the shared leaf widgets (`avatar`, `icon`,
  `taglist`, `topbar`) are dissolved out of `ui/` into top-level directory
  modules; see the amendment under Decision point 5
- Date: 2026-07-18
- Issue: [#526](https://github.com/jaunder-org/jaunder/issues/526)

## Context

[ADR-0056](0056-web-canonical-colocated-leptos.md) converged `web` on co-located
verticals split **by cargo feature, never `target_arch`**: `#[component]` UI is
ungated and host-compiles as dead-but-exempt code (the coverage gate's syntactic
`#[component]` exemption, ADR-0050, holds it green). That doctrine rested on a
premise verified at the time: components used only dual-target Leptos
primitives, so nothing _forced_ a `target_arch` gate — `web-sys` itself
host-compiles, so even the browser touchpoints could be made "dual-target-clean,
not gated."

[ADR-0069](0069-client-crate-wasm-only-home.md) then chartered the `client`
crate as the wasm-only browser-glue home: a single crate-level
`#![cfg(target_arch = "wasm32")]`, **genuinely empty on host, never a fake
stub**. Milestone 14 (#514–#520) moves `web`'s scattered browser glue into it.

These two decisions cannot both hold once a component consumes a `client`
primitive. An ungated `#[component]` host-compiles; a call into a crate that is
an empty rlib on host does not. Working the milestone surfaced this immediately:
either every consumer stays behind an interim `#[client_only]`/`target_arch`
wrapper layer in `web` (exactly the noise the milestone exists to delete), or
`client` grows fake host stubs (which ADR-0069 forbids, with reason), or
components stop host-compiling.

Two facts tip the choice. First, since #180 **no component ever executes on the
host in production** — the projector renders through the pure, non-reactive
`web::render`, and authed routes are static CSR shells. Host compilation of the
reactive UI is pure dead code, and a real machinery stack exists solely to keep
it green: the `#[component]` exemption, the `#[client_only]` identity macro
(ADR-0062), the A1 guard, and per-site `cov:ignore`. Second, the dead-but-exempt
arrangement is what _prevents_ components from calling browser infrastructure
directly — the direct call is the simpler code we actually want to write.

## Decision

`web` keeps ADR-0056's **co-location** and reverses its **gating**: each
vertical splits host and wasm code at the **file** level, inside the existing
single crate.

1. **Per-vertical layout** (as amended by #530). A vertical (`audiences/`,
   `posts/`, …) is:
   - `mod.rs` — **module wiring only**: the module declarations (`mod api;`, the
     gated `mod server;`/`mod component;`) and the re-exports
     (`pub use api::{…};`,
     `#[cfg(target_arch = "wasm32")] pub use component::{…};`). No items of its
     own — declaring endpoints here would mix the collection-point concern with
     the API surface.
   - `api.rs` — the vertical's API surface: shared wire types and the
     `#[server]` endpoint declarations, dual-compiled, plus at most one grouped
     `#[cfg(feature = "server")]` support-import gate for the bodies. The
     `mod.rs` re-exports keep external call-site and registrar paths
     (`web::<vertical>::<Leaf>`) stable.
   - `server.rs` — host-only support for the `#[server]` bodies, declared
     `#[cfg(feature = "server")] mod server;`.
   - `component.rs` — the `#[component]` UI and all browser-bound code, declared
     `#[cfg(target_arch = "wasm32")] mod component;`. **Zero cfg gates inside
     the file**, and free to call `client::` primitives directly.

   Other support files take the same treatment when needed; the gate always sits
   on the `mod` declaration, never on items inside a file.

2. **Gate axes are fixed.** `feature = "server"` expresses server-ness;
   `#[cfg(target_arch = "wasm32")]` expresses browser-ness and appears **only on
   module-wiring declarations** in `web/src` — a `mod` declaration or its paired
   re-export (`pub use component::{…};`), never on items inside a leaf file. A
   scan-style xtask check (modeled on the existing syn/scan steps; tracked by
   the re-scoped #520) enforces the wiring-only rule.

3. **`#[server]` fns stay ungated and dual-compiled.** The wasm side needs the
   generated client stub; the host side needs the handler and the registrar
   entry (#426 gate). Only their _support_ code is feature-gated.

4. **Components are wasm-only code.** They do not host-compile, are not
   dead-but-exempt, and call browser infrastructure (`client`, `web-sys`)
   directly — the `#[client_only]` wrapper layer in `web` is retired rather than
   relocated.

5. **`pages/` dissolves into the verticals' `component.rs` files** (new vertical
   dirs where none exists: cockpit, home, timeline). The `App` + Router shell
   keeps a wasm gate at whatever home it moves to. Shared widgets (`ui/`) become
   wasm-gated shared component files the same way.

   > **Amended (#527):** the shared leaf widgets are not kept under `ui/`. Each
   > (`avatar`, `icon`, `taglist`, `topbar`) is promoted to a **top-level
   > directory module** — `mod.rs` (wiring) + ungated `markup.rs` (the pure
   > `render()` twin the projector calls, host-tested) + wasm-only
   > `component.rs` — and `ui/` is dissolved. Shared presentation leaves are
   > therefore top-level modules, not a `ui/` sub-tree.

6. **ADR-0055's retained principles carry forward unchanged**: pure,
   host-testable logic (validation, form/signal state of the `Field<T>` kind,
   wire codecs) lives in **ungated, host-tested, coverage-measured** files and
   is extracted _before_ a gate goes on; no fake-value host stub is ever
   substituted for wasm-only code.

7. **Milestone 14's endgame is re-scoped.** "Zero `target_arch` cfgs in `web`"
   becomes "`target_arch` only on module-wiring lines." The final ratchet (#520)
   still drops `js-sys`/`wasm-bindgen` from `web` and retires `#[client_only]`,
   and now also retires the `#[component]` coverage exemption and the A1 guard's
   component arm — dead machinery once no ungated component remains.

## Consequences

- **Supersedes ADR-0056.** The co-location half survives verbatim; the
  "feature-only, dead-but-exempt" half is reversed — a deliberate partial return
  to ADR-0055's module-level gating, at per-vertical file granularity and
  without its stub temptations. ADR-0055's status is unchanged.
- **Re-scopes ADR-0069's framing, strengthening its charter.** "Components stay
  co-located and dual-target" no longer holds; `client` is the domain-free home
  for **cross-vertical** browser primitives, now called directly from
  `component.rs` instead of through `#[client_only]` wrappers. Its dependency
  rule (`web`/`csr` → `client`, never the reverse; no domain types; no pure
  logic) stands.
- **UI type errors surface only on the wasm target.** The `wasm-clippy` step
  (`-p web -p client`, host + Nix mirror) is permanent load-bearing gate
  surface, not the simplifiable residue #330 assumed; the fast iteration loop
  must include it.
- **Coverage:** component lines leave the host denominator entirely
  (not-compiled beats measured-but-exempt). Aggregate percentages will shift;
  that is re-scoping, not regression. What keeps host coverage honest is point
  6's extraction discipline. Exemption machinery retires only at the end, after
  the last vertical is split.
- **Migration** stays the human-directed, per-vertical broad cleanup of
  ADR-0056's consequences — the open vertical issues swap their invariant floor
  (wasm-gated `component.rs` instead of ungated dual-target), and the verticals
  already converged under the old rule (audiences, backup, the relocated shared
  leaves) get a mechanical retrofit.
- **Rules out** SSR of app routes (unchanged since #180 — reintroducing it would
  require re-litigating this ADR) and rules out per-item `target_arch` gates
  inside `web` files.
