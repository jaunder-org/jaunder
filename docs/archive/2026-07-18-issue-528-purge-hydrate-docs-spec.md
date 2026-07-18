# Spec — Issue #528: purge stale hydrate-crate/hydration references from ARCHITECTURE.md and CONTRIBUTING.md

## Problem

`docs/ARCHITECTURE.md` and `CONTRIBUTING.md` still describe the removed
`hydrate` crate and a hydration flow. Reality (post-#180 / ADR-0040 / ADR-0041):
there is no hydration and no reactive SSR render — the wasm entry is the `csr`
crate (`csr/src/lib.rs`, `web::mount_csr()`), and authed page routes serve
static CSR shells adopted by a public projector data blob. The docs mislead new
readers/agents.

## Scope of stale references (verified against the tree, 2026-07-18)

The issue's survey named the crate table row, the mermaid nodes, `hydrate/` in
the repo layout, the "Hydration component hotspots" prose, and the
WASM-entry-point path. Verification found the staleness is slightly **broader**:
the observability prose in **both** files describes `cargo xtask traces analyze`
output that no longer exists. The analyzer's real sections (from
`xtask/src/traces/render.rs`) are: slowest spans (overall + per `e2e.test`), and
top **action / navigation-phase / long-task / resource-initiator** hotspots,
plus per-project totals. There is **no** "hydration" section, no "commit →
hydration split by cacheWarmth", and no `wasm_init` / `leptos_hydrate` hotspot
category.

### `docs/ARCHITECTURE.md`

- Crate table row `` `hydrate` | WASM Binary | …"hydrates" the static HTML… `` →
  `csr` crate (WASM binary; the browser CSR entry that mounts `web::App`).
- Mermaid: `WASM[hydrate crate]` → `WASM[csr crate]`; edge
  `WASM -->|Hydrates| DOM` → mount/CSR wording.
- Tracing-layers bullet (line ~108): "capturing navigation, hydration, and
  resource summaries" → drop "hydration".
- Tooling bullet (line ~130): "slowest spans, hydration hotspots, and navigation
  phase bottlenecks" → match the analyzer's real hotspot categories.

### `CONTRIBUTING.md`

- Repo layout (line ~20): `` `hydrate/`: frontend driver `` →
  `` `csr/`: WASM frontend entry (the CSR bundle) ``.
- Traces analyzer bullets (lines ~295–296): replace
  `commit -> hydration splits by cacheWarmth` and
  `Hydration component hotspots (wasm_init, leptos_hydrate, …)` with the
  analyzer's actual reported sections.
- Coverage prose (line ~533): `WASM entry point (`hydrate/src/lib.rs`)` →
  `csr/src/lib.rs`.

## Decisions

1. **`web/src/pages/*` prose — DEFER.** `pages/` still physically exists (the
   #508 dissolution is pending; only the ADR #526 landed). The three `pages/`
   references (repo-layout line ~25, coverage lines ~432 and ~530) are
   **currently accurate**; rewriting them to a
   `mod.rs`/`server.rs`/`component.rs` layout that isn't fully real yet would
   introduce _new_ inaccuracy. Leave them and let #508 update them. (The issue
   explicitly permits deferring this part.)

2. **`docs/observability.md` — out of scope, leave.** Not a named file; its
   hydration mentions are historically accurate narrative describing the
   _removal_ (e.g. "no hydration left to remove", "the CSR mount, not hydration;
   renamed in #224"), not false claims that hydration exists.

3. **`csr/src/lib.rs` comment ("mirrors hydrate/src/lib.rs …") — FIX (per
   user).** A source-code comment referencing the dead `hydrate` crate path.
   Trivial one-line honesty fix folded into this PR: drop/replace the dead
   `hydrate/src/lib.rs` reference. Comment-only, no behavior change.

## Acceptance

- No mention of a `hydrate` crate/feature or a hydration flow remains in
  `docs/ARCHITECTURE.md` or `CONTRIBUTING.md`.
- The architecture crate table + mermaid diagram name `csr` as the WASM entry
  and drop the "Hydrates" edge.
- Observability prose in both files matches the real `traces analyze` output.
- The `csr/src/lib.rs` comment no longer names the dead `hydrate/src/lib.rs`
  path.
- `pages/` prose is intentionally left for #508.
- `cargo xtask check --no-test` stays green (markdown/format only; no runtime
  change).
