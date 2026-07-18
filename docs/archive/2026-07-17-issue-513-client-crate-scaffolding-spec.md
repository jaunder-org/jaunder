# Spec — #513: `client` crate — ADR + scaffolding (wasm-only browser infrastructure)

- Issue: [#513](https://github.com/jaunder-org/jaunder/issues/513)
- Milestone: client crate: browser glue out of web (M14)
- Governing ADR: [ADR-0058](../../adr/0058-host-crate-layering.md) (pre-charters
  the crate); reconciles
  [ADR-0056](../../adr/0056-web-canonical-colocated-leptos.md) and
  [ADR-0055](../../adr/0055-web-host-wasm-boundary-module-level.md). This issue
  authors a **new ADR** activating the client peer.
- Follow-ons (this milestone, each relocates one primitive into the crate this
  issue scaffolds): #514 (localStorage + auth-marker/theme/seed), #515
  (`server_resource` + `Invalidator`), #516 (navigation + confirm-dialog), #517
  (media upload fetch), #518 (`js_sys::Date` datetime helper), #519 (CSR boot),
  #520 (endgame ratchet — drop web's browser deps, retire `#[client_only]`).
- Date: 2026-07-17

## Problem

ADR-0058 chartered a target-scoped shared-crate trio — `common` (both targets),
`host` (host-only), and `client` (wasm-only) — but deferred creating `client`
"to be created when such shared code first appears." Milestone 14 is that
moment: it will relocate the raw browser glue currently scattered through `web`
(11 `#[cfg(target_arch = "wasm32")]` sites and ~8 `#[client_only]` markers
across `web/src`) into a single wasm-only crate that is the official,
e2e-verified home for browser infrastructure.

Every other issue in the milestone relocates a specific primitive **into** that
crate, so the crate — plus its build/gate wiring and a charter ADR — must exist
first. This issue delivers exactly that scaffolding and nothing else: **no code
is relocated here** (the first inhabitant lands in #514), and `web` is left
untouched (the `web → client` dependency edge forms when the first symbol
moves).

## Decision

Create the `client` workspace crate as the symmetric wasm peer of `host`
(inverting host's host-only stance to wasm-only), ship it **empty**, wire it
into the build and the wasm-clippy gate, and record its charter in a new ADR.

### The crate (`client/`)

`client/Cargo.toml` mirrors `host/Cargo.toml`'s conventions:

- `name = "client"`, `version = "0.1.0"`, `edition = "2021"`,
  `license = "GPL-3.0-only"` (host uses a literal `version`, not
  `version.workspace`).
- Default library crate-type (`rlib`) — **no `[lib]` block** (host has none).
  `client` is a leaf `rlib` that `web`/`csr` will consume; it is **not** the
  `cdylib` (that remains `csr`).
- `[lints] workspace = true`.
- **Empty `[dependencies]`.** The charter permits `common`, `macros`, and raw
  browser-infra deps (`web-sys`, `js-sys`, `wasm-bindgen`,
  `wasm-bindgen-futures`), but an empty crate uses none of them, so none are
  declared here. Each follow-on issue adds the dep it actually uses when it
  relocates a primitive (import discipline — no speculative deps).

`client/src/lib.rs` contains only a crate-level doc comment stating the charter
and a single crate-level cfg gate:

```rust
//! `client` — strictly-client (wasm/browser) shared infrastructure.
//! The symmetric wasm peer of `host`: holds only raw browser glue
//! (web_sys / js_sys / wasm_bindgen / wasm-side leptos plumbing) and never our
//! domain types. Depends on no workspace crate except `common` (+ `macros`).
//! See docs/adr/0069-client-crate-wasm-only-home.md (ADR-0058 trio).
#![cfg(target_arch = "wasm32")]
```

The `#![cfg(target_arch = "wasm32")]` inner attribute gates the **entire crate
root**: on the host target the crate compiles to an **empty rlib** (zero items,
zero coverage-measured regions); on `wasm32` it is active. Every future module
relocated into `client` inherits this gate, so relocated glue needs **no
per-item `#[cfg]`** and **no `#[client_only]`** marker — that is the crate's
reason to exist.

### Workspace wiring (root `Cargo.toml`)

- Add `"client"` to `[workspace] members` (kept alphabetically sorted: `client`
  sorts before `common`).
- Add `client = { path = "client" }` to `[workspace.dependencies]` so follow-on
  issues consume it via `client.workspace = true`. (An unused
  workspace-dependency declaration is silent — no cargo warning — and is part of
  the scaffolding "the rest of this milestone builds on.")

### Gate wiring (wasm-clippy)

Because `client`'s code exists only under `target_arch = "wasm32"`, the
wasm-clippy step is **the only place any `client` _code_ is actually compiled
and clippy-linted** — the host build (`cargo build --workspace`, host
`clippy --all-targets`) compiles `client` too, but as an empty rlib with nothing
to lint. So wasm-clippy is client's sole meaningful static gate. Extend the
existing single wasm-clippy step rather than adding a second one:

- `xtask/src/steps/static_checks.rs` — the `wasm-clippy` `StepSpec` gains
  `-p client` alongside `-p web` (one invocation:
  `cargo clippy -p web -p client --features csr --target wasm32-unknown-unknown -- -D warnings …`).
  `web` defines the `csr` feature, so under resolver v2 the shared
  `--features csr` binds to `web` and is accepted even though `client` has no
  features (cargo applies `csr` only to the package that has it). **This exact
  invocation must be run empirically before trusting the gate** — it is
  currently untested in-repo; the fallback if cargo rejects it is the explicit
  `--features web/csr` form.
- Add a **regression-lock unit test** to `static_checks.rs` asserting the
  `wasm-clippy` step's args contain `-p client`, matching the existing
  arg-locking tests for the other steps (none currently locks `wasm-clippy`).
- `flake.nix` — the mirror `wasm-clippy` derivation carries
  `-p web --features csr` in **two** places that must stay in sync:
  `buildDepsOnly`'s `cargoExtraArgs` (~L1120) **and** `cargoClippyExtraArgs`
  (~L1126). Add `-p client` to **both** so the deps-only prebuild and the clippy
  run stay mirrored (the in-file comment mandating xtask/flake sync applies).

### No other build surface changes

- **Nix source filters need no edit** — both the app-`src` filter and the
  coverage filter admit source by suffix / `filterCargoSources` with no
  per-crate members list, so a new top-level `client/` is auto-admitted
  (ADR-0058's promise; verified against the flake).
- **cargo-leptos / csr wiring needs no edit** — the project has no
  `[package.metadata.leptos]` block; the CSR Nix derivations name the `csr`
  package, and `client` is a transitive leaf that nothing depends on yet in this
  issue.
- **`web` is unchanged** — no `client` dependency edge is added here (deferred
  to #514).

### The ADR (new draft)

Author a numberless draft at `docs/adr/0069-client-crate-wasm-only-home.md`
(heading `# ADR-DRAFT: …`, referenced by path; `cargo xtask adr promote` numbers
it at ship). It must:

- **Activate** the `client` peer ADR-0058 pre-chartered — 0058 remains accepted
  and is cross-linked, not superseded.
- **Reconcile ADR-0056**: this is not reviving the #303 crate split ADR-0056
  rejected. Components stay co-located and dual-target in `web`; `client` is
  only the raw browser glue ADR-0056 point 4 wanted "dual-target-clean, not
  gated," and ADR-0056 explicitly left the door open ("if a crate boundary is
  ever wanted later…").
- **Restate ADR-0055's surviving rules** as binding on `client`: pure logic
  never moves here (it stays host-tested in `web`/`common` —
  relocate-pure-logic); no fake host stub (the crate is genuinely empty on host,
  not a divergent substitute — no-fake-stub).
- **Record the charter**: wasm-only (single crate-level
  `#![cfg(target_arch = "wasm32")]`); depends on no workspace crate except
  `common` (+`macros`); may take raw browser-infra deps; never our domain types;
  `web`/`csr` depend on `client`, never the reverse (keeps the graph acyclic).
- **Record the coverage position**: wasm-only ⇒ zero host-coverage-measured
  lines; `client` is the official home of e2e-verified browser glue, replacing
  per-site `#[client_only]` / `cov:ignore` exemptions in `web`; zero measured
  lines is not a gate failure.
- **Record the gate wiring**: wasm-clippy extended to `-p client`.

## Acceptance criteria

Each is observable so ship's conformance review can tell delivered from not.

1. **Member exists.** `client/` is a `[workspace] members` entry in root
   `Cargo.toml`, alphabetically sorted (before `common`).
2. **Crate conventions mirror `host`.** `client/Cargo.toml` has
   `name = "client"`, `version = "0.1.0"`, `edition = "2021"`,
   `license = "GPL-3.0-only"`, `[lints] workspace = true`, no `[lib]` block
   (default `rlib`), and an **empty `[dependencies]`** (no browser deps).
3. **Crate-level cfg gate.** `client/src/lib.rs` contains the charter doc
   comment and `#![cfg(target_arch = "wasm32")]` and **no items**.
4. **Workspace dep declared.** `[workspace.dependencies]` in root `Cargo.toml`
   contains `client = { path = "client" }`.
5. **Empty on host.** `cargo build --workspace` (host target) succeeds and
   `client` builds as an empty rlib. `cargo xtask check` passes; `client`
   contributes **zero** coverage-measured lines and does **not** fail the
   coverage gate (demonstrating zero-measured-lines ≠ failure).
6. **wasm-clippy lints client.**
   `cargo clippy -p web -p client --features csr --target wasm32-unknown-unknown -- -D warnings …`
   passes (run empirically), the `wasm-clippy` `StepSpec` in
   `xtask/src/steps/static_checks.rs` names `-p client`, and a
   `static_checks.rs` unit test arg-locks that the `wasm-clippy` step contains
   `-p client` (matching the existing per-step arg-lock tests).
7. **Flake parity.** The mirror `wasm-clippy` derivation in `flake.nix` names
   `-p client` in **both** the `cargoExtraArgs` (deps-only prebuild) and
   `cargoClippyExtraArgs` (clippy run) positions, keeping the two in sync per
   the in-file comment.
8. **No scope creep.** `web/` source is unchanged (no `client` dependency edge);
   no primitive is relocated; the Nix source filters and any cargo-leptos/csr
   wiring are unchanged.
9. **ADR recorded.** A numberless draft exists at
   `docs/adr/0069-client-crate-wasm-only-home.md` with heading
   `# ADR-DRAFT: …`, covering: activation of the ADR-0058 client peer (0058
   cross-linked, not superseded); reconciliation with ADR-0056 (not the #303
   split); ADR-0055's surviving rules bind on `client`; the charter (layering,
   deps, never-domain); the coverage position (wasm-only, zero host lines,
   replaces per-site `client_only`/`cov:ignore`); and the wasm-clippy gate
   wiring.
10. **Gate green.** `cargo xtask validate --no-e2e` passes on the branch.

## Out of scope (owned by follow-on issues)

- Relocating any primitive into `client` (localStorage, datetime, navigation,
  media upload, CSR boot, `server_resource`/`Invalidator`) — #514–#519.
- Adding browser-infra deps to `client/Cargo.toml`, or the `web → client`
  dependency edge — added by the first issue that relocates a symbol (#514).
- Retiring the `#[client_only]` macro and dropping `js-sys`/`wasm-bindgen` from
  `web` — #520.
