# Issue #1884: isolated CSR bundler feasibility proof

## Outcome

Request authority for one isolated, Vite-first feasibility proof before choosing
a bundler or approving migration. The proof tests whether supported pinned
tooling can process this project's real CSR runtime with materially less
maintained project-specific machinery; it may pass, fail, or be blocked and is
not a tool selection or production migration approval.

## Approach and tradeoffs

Use release `wasm-bindgen` output from the currently supported WASM build,
preserving production `wasm-opt -Oz`. Reuse the existing `initMeasured` and
early-fetch behavior needed by the actual shell; count any extraction or
adapter. Verify the stylesheet marker resolves and startup does not fetch WASM
twice. Do not replace these inputs with a hand-built graph or weakened
optimization path.

Determine a project-compatible supported Vite release pinned from the actual
nixpkgs/tool closure. Compare its fit and maintained-code cost with esbuild and
direct Rollup; Vite's HTML handling motivates testing it first, not selecting
it. Do not pin old Vite 6 merely to assume a Rollup backend. Check the chosen
compression package's release/API compatibility rather than assuming it.

All probe files and output stay under ignored `.xtask/issue-1884-probe/`; never
write `target/site`, production outputs, or shared consumer contracts. Use the
actual shell, including auth prepaint and stylesheet markers, with no WASM
preload. Demonstrate the browser resolves the runtime import graph and
instantiates WASM. Unchanged inputs must retain content-derived URLs; changed
runtime or WASM bytes must invalidate the corresponding URL.

Authorize probe-local exact dependency pins, lockfile, config, and isolated Nix
derivation under that ignored directory, plus store prefetch needed to populate
pinned inputs. Run the pinned build directly on the host and execute the actual
offline Nix sandbox build; network prefetch is distinct and does not prove
offline execution. Do not use `npx`, `npm run`, `npm exec`, `pnpm`, `yarn`, or a
`nix develop` wrapper. If a direct supported binary is absent and tracked
devshell or other wiring is necessary, stop and request bounded approval before
any dependent execution—not afterward.

Show emitted gzip and Brotli representations decode to identity bytes. If
feasible, use the existing serving seam for a narrow isolated HTTP check of
identity/gzip/Brotli selection, explicit `q=0` exclusion, decoded-byte
equivalence, logical MIME, `Vary`, cache headers, representation-specific ETag,
and matching-representation 304. Reuse that seam; do not build a general harness
or copy production serving code. If infeasible, report why and leave behavior
unproven. Assess whether native inventory helps build-only staging, audit,
qualification, and coverage consumers. A read-only adapter may demonstrate
role/asset/representation use; do not recreate a bespoke schema or rewrite
production consumers.

Count maintained production code, probe configuration, adapters/extractions, and
tests separately; identify plausible deletion candidates without presenting
projected deletions as measured. The 3,117 whole-file LOC baseline includes
unrelated code and is not a removable-code estimate.

## Acceptance

- Record host execution and actual offline sandbox execution, or their exact
  failure/blocker; distinguish results from reused evidence and qualify its
  scope. Report each success criterion as passed, failed, blocked, or not
  established—never infer green from unknowns.
- Assess real graph/instantiation, shell markers, single WASM fetch, stylesheet
  resolution, stable/invalidated URLs, compression/decode, native inventory
  usefulness, and the narrow serving check where feasible.
- Compare esbuild and direct Rollup capability gaps and counted maintained code.
  Reject any path requiring custom graph rewriting, hashing, or parallel schema
  reimplementation.
- Keep artifacts isolated and ignored; no production source, build output,
  shared consumer contract, or serving behavior changes.
- Phase-2 report states evidence and limitations, recommends whether to proceed
  and with which tool, and outlines risk-triggered planning before any
  production rewrite. Reconcile ADR-0175 only if an eventual proposal actually
  contradicts it.

## Boundaries

This request authorizes only the isolated proof and probe-local
pins/lockfile/config/derivation/prefetch stated above. No tracked devshell or
production wiring, consumer rewrites, migration, tool selection, or
implementation is authorized. Keep #1872 / PR #1880 Just work out of scope.
Defer an outline: this experiment creates no durable schema, architecture, or
multi-agent implementation contract; reassess outline triggers before any later
migration.
