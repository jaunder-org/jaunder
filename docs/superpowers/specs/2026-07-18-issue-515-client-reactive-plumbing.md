# Spec — #515: remove SSR-vestigial `server_resource`; relocate the client-only reactive helpers into `client`

- Issue: [#515](https://github.com/jaunder-org/jaunder/issues/515)
- Milestone: client crate: browser glue out of web (M14)
- Governing ADRs: [ADR-0069](../../adr/0069-client-crate-wasm-only-home.md)
  (client charter — **accepted**; `client` is the wasm-only peer of `host`),
  [ADR-0070](../../adr/0070-web-vertical-wasm-only-component-files.md) (the
  four-file host/wasm split — `component.rs` is
  `#[cfg(target_arch = "wasm32")]`, which is why the relocation needs no new
  cfgs; supersedes ADR-0056),
  [ADR-0060](../../adr/0060-web-invalidator-revalidation-idiom.md),
  [ADR-0061](../../adr/0061-web-keyed-list-reactive-store.md) (Invalidator
  contract, unchanged).
- Predecessors: #513 (created the empty `client` crate); #514 (already landed a
  real module, `client/src/storage.rs`, the localStorage primitive — the crate
  is populated, not a greenfield question).
- Spawns a follow-up (filed by the plan's first task): **investigate** whether
  `server_boundary`'s owner-pinning (#89/#138) is likewise SSR-vestigial now
  that no component SSR remains (it may not be — server-fn bodies still read
  leptos context across awaits; hence a separate investigation, not a presumed
  deletion).
- Date: 2026-07-18. **Revised 2026-07-22** on resume: reconciled with the M14
  convergence that landed while this sat idle (ADR-0070 four-file layout, the
  `host`/`csr` peers, #514), the SSR-dead premise hardened from assertion to
  verified, and the Phase-1 test-deletion list and call-site inventory
  corrected.

## Problem

`web` carries the last of the reactive `#[client_only]` markers — the four
browser-only `Invalidator` helpers (`resource`, `action`, `patched`, `sticky` in
`web/src/reactive.rs:52,69,96,143`) — plus `server_resource`
(`web/src/error.rs:126`), the sanctioned `Resource::new` wrapper. Milestone 14
relocates browser glue into the wasm-only `client` crate; these helpers are
browser-only (`Effect`/fetch, e2e-exercised), so they belong there, and moving
them lets the `#[client_only]` coverage-exemption markers go (code in `client`
is empty-on-host under the crate-level `#![cfg(target_arch = "wasm32")]` →
nothing to exempt).

Investigating the move surfaces that **`server_resource`'s reason to exist is
dead**. Its `scoped_fetcher_future` wrap exists _solely_ to survive SSR's
worker-thread owner detachment (#89/#124/#138 — the doc comments and the
`owner_lifetime` test module name SSR explicitly): a `Resource` fetcher polled
on an SSR worker thread detached from the reactive owner would lose server-fn
context.

**Verified premise — there is no component SSR:** a repo-wide search for leptos
SSR render entrypoints (`render_app_to_stream`, `LeptosRoutes` /
`leptos_routes`, `generate_route_list`, `HydrationScripts`, `hydrate_body`)
returns **zero matches**. `web` is CSR-only: authed routes serve a static 200
CSR shell (#487), components render only in the browser, and there is no
hydration. The `leptos/ssr` feature still enabled under `web`'s `server` feature
exists for the `#[server]`-fn _backend_ (leptos requires it to register
server-fn handlers), **not** for rendering components. Every `server_resource`
call site is wasm-only — the converged verticals gate their UI via
`#[cfg(target_arch = "wasm32")] mod component;` (ADR-0070), and the
still-un-converged `web/src/pages/*` sites are gated wholesale by the parent
`#[cfg(target_arch = "wasm32")] pub mod pages;` in `web/src/lib.rs`. So
`server_resource` only ever executes in the browser. The detachment it guarded
cannot arise there: the hazard needed a fetcher to read leptos reactive context
_after_ an `.await`, and no CSR fetcher does — the fetchers are server-fn HTTP
calls (`|()| list_my_audiences()`, `|_| get_profile()`, …) whose context reads
run server-side, so even the one owner-disposal that _does_ occur in CSR (a
component unmounting while its `Resource` is still pending) loses nothing.
`scoped_fetcher_future` is therefore inert, `server_resource` a bare passthrough
to `Resource::new`, and the `clippy.toml` #124 ban that forces it guards a dead
hazard.

This supersedes the issue body's original "server_resource's #124 guard still
bans raw `Resource::new`" done-when: the guard is **removed**, not preserved.

## Key mechanism (why the callers need no new `#[cfg]`)

Under ADR-0070's four-file layout, a converged vertical's UI lives in
`component.rs`, declared `#[cfg(target_arch = "wasm32")] mod component;`; the
still-un-converged `web/src/pages/*` modules are gated wholesale by
`#[cfg(target_arch = "wasm32")] pub mod pages;` in `web/src/lib.rs`. Either way
the `server_resource` call sites and the `Invalidator`-helper call sites are
**already wasm-only** — they compile and run only on the wasm build, exactly as
the relocated `client` symbols will. Relocating `server_resource`'s replacement
(`Resource::new`) and the four helpers into wasm-only `client` therefore needs
**no** new `#[cfg(target_arch = "wasm32")]` gating and no restructuring of any
vertical: the calls simply resolve to `leptos`/`client` symbols on the wasm
build and are absent on host exactly as the `component.rs` bodies already are.
(This is the module-level gate ADR-0070 established; the earlier framing that
"the `#[component]` annotation is itself the gate" is superseded.)

## Decision

Two phases, delivered as two commits.

### Phase 1 — remove the SSR-vestigial `server_resource`

- Delete `server_resource` and `scoped_fetcher_future` from `web/src/error.rs`,
  and the `pub use error::server_resource;` re-export in `web/src/lib.rs`.
- Remove the `leptos::prelude::Resource::new` entry from `clippy.toml`
  `disallowed-methods` (its #124 rationale is dead).
- Rewrite every `crate::server_resource(source, fetcher)` call site (**30**,
  across the converged `web/src/*/component.rs` — `posts` (×12), `media` (×2),
  `backup` (×2), `home`, `registration` — and the still-un-converged
  `web/src/pages/*` — `invites` (×2), `email` (×2), `ui` (×2), `profile` (×2),
  `sessions`, `cockpit`, `site` — plus the internal `Invalidator::resource` in
  `reactive.rs`) to `leptos::prelude::Resource::new(source, fetcher)` directly.
  The two share the `(source, fetcher)` shape; the only delta is the dropped
  `scoped_fetcher_future` wrap. (`action` does **not** use `server_resource` and
  is untouched by Phase 1.)
- Delete the **three** host tests in `web/src/error.rs`'s `owner_lifetime`
  module that target the removed symbols:
  - `server_resource_constructs_under_owner` (calls `server_resource`),
  - `scoped_fetcher_future_keeps_context_across_owner_drop` (calls
    `scoped_fetcher_future`),
  - `post_await_read_loses_ancestor_context_when_parent_owner_dropped`
    (reproduces the #138 problem _via_ `scoped_fetcher_future`). Its sibling
    fix-test `server_boundary_keeps_ancestor_context_alive_across_await` remains
    and keeps #138 behavior covered.
- **Do not touch** `server_boundary`, `owner_ancestry_strong`, or their
  remaining `server_boundary_*` / #89/#138 tests — that is the spawned follow-up
  issue.

### Phase 2 — relocate the four `Invalidator` helpers into `client`

- Add a `client::reactive` module holding the four helpers as **free functions**
  (orphan rules + the `web → client` dependency direction forbid naming, let
  alone `impl`-ing, `Invalidator` in `client`). Each takes the reactive
  primitives it needs as closures over the `Invalidator` core rather than the
  `Invalidator` type:
  - `resource(track: impl Fn() -> u32, fetch) -> Resource<T>` —
    `Resource::new(move || track(), …)`.
  - `action<A>(notify: impl Fn() + …) -> ServerAction<A>` — success-gated
    `Effect` that calls `notify()` on `Some(Ok(_))` (ADR-0060 §2).
  - `patched<…>(track, fetch, patch) -> Signal<ListState>` — patches on
    `Some(Ok(vec))` only, via the caller's `patch` closure (ADR-0061).
  - `sticky<…>(track, fetch) -> Signal<Option<Result<T, E>>>` — retains last
    resolved result (#346).
- Move `ListState` (`web/src/reactive.rs`) to `common` (pure enum, no leptos),
  so `client::patched` can return it and `web` views can name it. Only two sites
  touch it today (the `reactive.rs` definition and `audiences/component.rs`);
  `common` gains **no** `leptos` dependency.
- `client` gains `common`, `serde`, and `leptos` — the latter behind a
  **forwarded `csr` feature** (`csr = ["leptos/csr", …]`, mirroring `web`),
  **not** an unconditional `leptos` dependency: an unconditional dep would let
  workspace feature-unification activate `leptos/csr` and `leptos/ssr` (from
  `web`'s server build) simultaneously, which leptos forbids. `web`'s own `csr`
  feature must forward to it — `csr = ["leptos/csr", "client/csr"]` (today just
  `["leptos/csr"]`, `web/Cargo.toml`) — so `client`'s leptos actually turns on
  in the CSR build; without that edge `client::reactive` won't compile on wasm.
  No `#[client_only]`.
- `Invalidator` core (`new`/`notify`/`track`/`Default`) and the
  `invalidator_scope!` macro **stay in `web/src/reactive.rs`**, host-tested and
  behaviourally unchanged.
- Retire the four `#[client_only]` attributes and the `use macros::client_only;`
  import in `reactive.rs`.
- Rewrite the `web::audiences` call sites (the **sole** consumer —
  `audiences/component.rs`, itself already wasm-only) from `list.patched(…)` /
  `roster.sticky(…)` / `inv.action::<A>()` to the `client` free functions,
  threading `move || inv.track()` / `move || inv.notify()` closures. (One site
  calls `.action()` on an inline `expect_context::<AudienceList>()`; bind it to
  a local first so the `move || inv.notify()` closure can capture it.) Update
  the `use crate::reactive::{…, ListState}` import to source `ListState` from
  `common`.

## Acceptance criteria

Each observable so ship's conformance review can tell delivered from not.

1. **Zero reactive `#[client_only]`.** No `#[client_only]` attribute remains in
   `web/src/reactive.rs`, and the `use macros::client_only;` import there is
   gone. `reactive.rs` held the **last** `#[client_only]` markers in `web`:
   after this issue, `rg '#\[client_only\]' web/src` returns nothing (the only
   other hit today is a prose comment in `forms/field.rs`, not an attribute).
2. **`server_resource` removed.** `server_resource` and `scoped_fetcher_future`
   no longer exist in `web`; `rg 'server_resource' web/src` returns nothing
   (including the `lib.rs` re-export).
3. **#124 ban lifted.** `clippy.toml` `disallowed-methods` no longer bans
   `leptos::prelude::Resource::new`.
4. **Call sites use `Resource::new`.** Every former `server_resource` call site
   compiles against `leptos::prelude::Resource::new`; the wasm-clippy gate
   (`-p web -p client --features csr`) and the coverage build are green.
5. **Dead tests removed.** All three of
   `server_resource_constructs_under_owner`,
   `scoped_fetcher_future_keeps_context_across_owner_drop`, and
   `post_await_read_loses_ancestor_context_when_parent_owner_dropped` are gone;
   `server_boundary` and its remaining #89/#138 tests remain and pass.
6. **Helpers in `client`.** `client::reactive` exposes `resource`, `action`,
   `patched`, `sticky` as free functions; `web::audiences` consumes them. No
   `#[cfg(target_arch = "wasm32")]` was added to `web::audiences`.
7. **`ListState` in `common`.** `ListState` is defined in `common`, consumed by
   both `client::patched` and `web` views; `common` has no `leptos` dependency.
8. **Core stays host-tested.** `Invalidator::{new,notify,track}` and
   `invalidator_scope!` remain in `web/src/reactive.rs`; the `reactive.rs` host
   tests (`notify_changes_the_tracked_revision`,
   `scope_newtype_derefs_to_its_invalidator`) are unchanged and green.
9. **Dependency direction intact.** `client` depends only on `common` (+`leptos`
   via the `csr` feature, `serde`, and its existing `wasm-bindgen`/`web-sys`);
   no `client → web` edge. `web`/`csr` depend on `client`.
10. **Gate green.** `cargo xtask validate --no-e2e` passes; e2e (audiences
    revalidation flows) is exercised by the existing suite in CI.
11. **Follow-up filed.** A separate GitHub issue exists to **investigate**
    whether `server_boundary`'s owner-pinning (#89/#138) is SSR-vestigial (the
    plan's first task).

## Out of scope

- `server_boundary` / `owner_ancestry_strong` owner-pinning — the spawned
  investigation follow-up (not presumed dead: server-fn bodies may still read
  context across awaits).
- Retiring the `#[client_only]` macro itself — the milestone endgame (#520).
- Moving `Invalidator` to `common` / any `leptos`-in-`common` change —
  explicitly rejected; the core stays host-tested in `web`.
