# Plan — Issue #528: purge stale hydrate/hydration references

Spec: `docs/superpowers/specs/2026-07-18-issue-528-purge-hydrate-docs.md`.
Docs/comment-only; no runtime change. One logical commit.

## Task 1 — `docs/ARCHITECTURE.md`

- [x] Crate table: replace the `hydrate | WASM Binary | …hydrates…` row with a
      `csr` row (WASM binary; browser CSR entry that boots/mounts `web::App` —
      no hydration).
- [x] Mermaid: `WASM[hydrate crate]` → `WASM[csr crate]`; edge label `Hydrates`
      → mount/CSR wording (e.g. `Mounts (CSR)`).
- [x] Tracing-layers bullet: "navigation, hydration, and resource summaries" →
      drop "hydration" (leave navigation + resource summaries, matching the
      analyzer).
- [x] Tooling bullet for `traces analyze`: "slowest spans, hydration hotspots,
      and navigation phase bottlenecks" → the analyzer's real categories (action
      / long-task / navigation-phase / resource-initiator hotspots).

## Task 2 — `CONTRIBUTING.md`

- [x] Repo layout: "`hydrate/`: frontend driver" → "`csr/`: WASM frontend entry
      (the CSR bundle)".
- [x] `traces analyze` reporting bullets: drop "`commit -> hydration` splits by
      cacheWarmth" and "Hydration component hotspots (`wasm_init`,
      `leptos_hydrate`, …)"; replace with the real sections (per-`e2e.test`
      durations, action / navigation-phase / long-task / resource-initiator
      hotspots, per-project totals).
- [x] Coverage prose "WASM entry point (`hydrate/src/lib.rs`)" →
      "`csr/src/lib.rs`".
- [x] Do **not** touch the three `web/src/pages/*` references (deferred to
      #508).

## Task 3 — `csr/src/lib.rs` comment

- [x] Fix the `#![recursion_limit]` comment that reads "mirrors
      `hydrate/src/lib.rs` and `web/src/lib.rs`" — drop the dead
      `hydrate/src/lib.rs` reference (comment only; no code change).

## Task 4 — Verify & commit

- [x] `rg -i 'hydrat' docs/ARCHITECTURE.md CONTRIBUTING.md` returns nothing.
- [x] `csr/src/lib.rs` no longer names `hydrate/src/lib.rs`.
- [x] Mermaid still renders (balanced `subgraph`/`end`, node/edge syntax
      intact).
- [x] `prettier -w` the two Markdown files (avoid the pre-commit restage
      double-commit).
- [x] `cargo xtask check --no-test` green; `git status --porcelain` clean after
      (fmt auto-fix leaves nothing dirty).
- [x] Commit (single, references #528).
