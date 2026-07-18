# ADR-0069: `client` crate — the wasm-only browser-infrastructure home

- Status: accepted
- Note: re-scoped by ADR-0070 (web verticals split host/wasm at the file level)
  — components are wasm-only consumers of `client`, not dual-target; the charter
  (crate-level gate, no domain types, `web`/`csr` → `client` only) stands
- Date: 2026-07-17
- Issue: [#513](https://github.com/jaunder-org/jaunder/issues/513)

## Context

[ADR-0058](../0058-host-crate-layering.md) chartered a target-scoped
shared-crate trio and named its third member in advance:

- **`common`** — shared code that compiles to _both_ host and wasm.
- **`host`** — shared code that only makes sense on the host.
- **`client`** _(future)_ — the symmetric wasm/browser peer, "**to be created
  when such shared code first appears**."

That moment has arrived. Milestone 14 relocates the raw browser glue currently
scattered through `web` — 11 `#[cfg(target_arch = "wasm32")]` sites and a
handful of `#[client_only]` markers across `web/src` — into a single wasm-only
crate. Each follow-on issue (#514–#519) moves one primitive **into** that crate,
so the crate, its build/gate wiring, and this charter must exist first. This ADR
activates the pre-chartered peer; #513 delivers the empty scaffolding.

Two prior decisions constrain the shape:

- **[ADR-0056](../0056-web-canonical-colocated-leptos.md)** rejected the #303
  three-crate split and made `web` a canonical single crate whose components are
  co-located and dual-target. It did **not** rule out a leaf crate for raw glue
  — point 4 wanted the two real browser touchpoints "**dual-target-clean, not
  gated**," and it explicitly left the door open: "**if a crate boundary is ever
  wanted later**, the canonical single-crate `web` is the correct base for it."
- **[ADR-0055](../0055-web-host-wasm-boundary-module-level.md)** (superseded by
  0056, but its **relocate-pure-logic** and **no-fake-stub** principles are
  retained): pure logic lives in host-compiled, coverage-measured homes and is
  relocated _before_ any gating; no wasm-only code is ever given a divergent
  fake host substitute.

## Decision

Create the **`client`** workspace crate now, as the symmetric wasm peer of
`host` — where `host` knows raw host infrastructure but never our domain
abstractions, `client` knows **raw browser infrastructure** (`web_sys`,
`js_sys`, `wasm_bindgen`, wasm-side leptos plumbing) but **never our domain
types**. `client` is _not_ a revival of the #303 split ADR-0056 rejected:
components stay co-located and dual-target in `web`; `client` holds only the raw
browser glue ADR-0056 point 4 wanted dual-target-clean, realized as the crate
boundary ADR-0056 said could come later.

**Charter (mirrors `host`'s layering, inverted for wasm):**

- **Strictly-client (wasm/browser) code only.** A single crate-level
  `#![cfg(target_arch = "wasm32")]` gate makes the crate **empty on the host
  target** and active only on `wasm32`. Every module relocated into `client`
  inherits the gate, so it carries **no per-item `#[cfg]`** and **no
  `#[client_only]`** marker.
- **Depends on no workspace crate except `common` (+ `macros`).** It may take
  external browser-infrastructure deps the glue genuinely needs (`web-sys`,
  `js-sys`, `wasm-bindgen`, `wasm-bindgen-futures`). `web` and `csr` depend on
  `client`, **never the reverse** — this is what keeps the crate graph acyclic
  when `web`'s components call client primitives.
- **Never our domain types**, and — per ADR-0055's surviving rules — **pure
  logic never moves here.** Pure, host-testable logic stays in `web` (or
  `common` when both targets need it) so it keeps its test/coverage obligation;
  the crate is genuinely empty on host, never a fake host substitute.

**Coverage position.** Because `client` is wasm-only, the host-run instrumented
coverage build sees **zero measured lines** in it, and that is **not** a gate
failure: the Nix coverage source filter auto-admits any new top-level crate, and
a crate with no host-compiled lines simply contributes nothing to measure.
`client` is therefore the official, **e2e-verified** home for browser glue,
replacing the per-site `#[client_only]` / `cov:ignore` coverage exemptions that
`web` uses today.

**Gate wiring.** Since `client`'s code exists only under
`target_arch = "wasm32"`, the **wasm-clippy static-check step is the only place
any `client` _code_ is actually compiled and linted** (the host build compiles
it as an empty rlib with nothing to lint). The existing single `wasm-clippy`
step (`xtask/src/steps/static_checks.rs`) and its mirror `flake.nix` derivation
are extended to lint `-p client` for the `wasm32-unknown-unknown` target
alongside `web`.

## Consequences

- **Commits us** to the `client` crate as the wasm-only glue home for the rest
  of Milestone 14: #514–#519 each relocate one primitive into it (localStorage /
  auth-marker / theme-seed, `server_resource` / `Invalidator`, navigation /
  confirm-dialog, media upload fetch, the `js_sys::Date` datetime helper, and
  the CSR boot); #520 then drops `js-sys` / `wasm-bindgen` from `web` and
  retires the `#[client_only]` macro entirely.
- **Keeps the crate graph acyclic** by construction: `client` sits below
  `web`/`csr`, above only `common`/`macros`.
- **Rules out** putting pure or dual-target logic in `client` (it belongs in
  `web`/`common`), and rules out a fake host stub for anything gated wasm-only.
- **Does not supersede ADR-0058 or ADR-0056** — 0058's trio charter stands and
  is fulfilled; 0056's single-crate `web` stands, with this leaf as the boundary
  it foresaw.
- **No new coverage/CI wiring beyond wasm-clippy**: the source filters
  auto-admit the crate, and nextest/clippy run workspace-wide.
