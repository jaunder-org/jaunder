# Issue #1884: migrate CSR bundling to Vite

## Outcome

Propose replacing bespoke CSR graph processing with the probe-tested Vite 8.3.5
pipeline and simplifying its producer and consumers. This is a design proposal
only: tool/migration approval, a risk-triggered implementation outline, and any
later merge remain separate gates.

## Approach and tradeoffs

Pin Vite 8.3.5 and `vite-plugin-compression2` 2.5.4 as proved, exact versions;
use Vite's HTML entry, ordinary content-sensitive output hashes and native build
manifest. Run the same locked host and Nix build paths. Keep `wasm-bindgen`,
production `wasm-opt -Oz` flags, and raw optimized-WASM size budget. Do not add
a Rust CLI wrapper or retain current orchestration merely because it exists;
#1872/PR #1880 Just edits remain excluded and overlapping future interfaces must
be coordinated.

Reconcile ADR-0175 explicitly through the accompanying numberless ADR draft:
standard tool owns graph/output naming, short content-sensitive hashes replace
required full-SHA filenames without claiming cryptographic integrity, ordinary
bundler graph handling replaces bespoke cycle policy, and the native build-only
manifest replaces the role-tagged project schema. The manifest remains unserved.
Compute digests only where consumers need evidence; do not re-create full
inventory/schema validation or graph/hash rewriting.

Keep the existing `rust-embed` and `site.rs` serving path. Use the standard
manifest and output paths to simplify staging, server build, audit,
qualification, and coverage consumers; preserve logical MIME,
`Vary: Accept-Encoding`, identity/gzip/Brotli selection including `q=0`,
representation-specific ETags/304s, and immutable caching. Do not substitute
external `ServeDir` or duplicate the custom inventory. Diagnostic coverage may
need a standard entry exporting its required modules, with tests for that path;
preserve coverage-artifact reconciliation separately.

Retain only necessary integration: raw-WASM measurement, system-theme/style URL
resolution, projector-generated URLs, auth prepaint, and an adapter that
preserves the classic early WASM fetch before the module entry, single fetch,
and no WASM preload. Preserve existing `initMeasured` boot instrumentation, API
timing, and module-shape diagnostics; count any extraction or adapter. The probe
proved single-fetch but moved fetch scheduling; it did not prove original
ordering/timing, so migration must restore and test it rather than assume
equivalence.

Simplify producer and all consumers, not just move logic. Current producer
counts are 1,404 production and 584 test lines; roughly 670 producer production
lines are deletion candidates, not measured savings. They exclude orchestration,
system artifacts, consumers, configuration and replacement tests. Measure the
complete before/after maintained production, configuration/adapters and tests;
there is no arbitrary LOC quota, but demonstrate substantial net reduction and
identify irreducible code.

## Acceptance

- Host and Nix production pipelines use the exact pins, preserve `-Oz` raw
  budget behavior, and demonstrate unchanged-input URL stability plus
  WASM/runtime mutation invalidation without custom hashing; verify resulting
  URLs in both the static shell and public-projector output.
- Browser tests cover actual shell/auth startup, stylesheet, WASM graph, single
  early fetch/order, no preload, projector URLs, and existing `initMeasured`
  API-timing/module-shape diagnostics; existing boot-diagnostic reconciliation
  passes with migrated assets.
- Consumer tests prove standard-manifest
  staging/build/audit/qualification/coverage behavior, including diagnostic
  exports, without a parallel role schema; build-only manifest is not served.
- Existing serving seam passes identity/gzip/Brotli, `q=0`, decoded bytes, MIME,
  `Vary`, immutable cache, representation ETag and matching 304 checks for
  generated assets.
- Compare measured deleted and retained code across producer, consumers,
  config/adapters and tests; report probe-proven facts separately from unproven
  migration behavior.

## Boundaries

This proposes tool, contract and consumer direction for design approval only. It
authorizes no production edits, experiment rerun, tool lock-in, outline, or
merge. After design approval, reassess risk triggers and obtain the required
outline approval before implementation; preserve the phase-1 spec and ignored
probe evidence. Keep #1872/PR #1880 Just changes out of scope.
