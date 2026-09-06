# Content-addressed `/pkg` assets implementation outline

> Execute with `jaunder-iterate`; delegate bounded slices with
> `jaunder-dispatch`. This outline exists because the approved spec introduces a
> durable cross-workspace build interface, changes the asset URL protocol, and
> requires a controlled multi-browser measurement.

Authoritative contract:
`docs/superpowers/specs/2026-09-03-issue-869-content-addressed-pkg-assets.md`.

## Scope

In:

- Content-address the complete runtime CSR asset graph and emit one verified,
  build-only manifest.
- Migrate both shells, server embedding/serving, Nix/host assembly, and WASM
  audit/budget consumers to that interface.
- Preserve compressed serving and boot behavior, add immutable cache semantics,
  and capture the approved A/B evidence.
- Ship the proposed ADR and truthful architecture projection with the feature.

Out:

- Preload or prewarm changes, fixed-name aliases, a general JS bundler, public
  assets outside `/pkg/`, and WASM optimization or ceiling changes.

## Cross-task contracts

- Add a focused package under the `tools` workspace whose Rust crate is
  `csr_bundle`. It owns manifest serialization, role/inventory/path/digest
  validation, and lookup by semantic role without application or storage
  dependencies. Both `toolsSrc` and the product source filter include this
  location; the server consumes it only as a build dependency, and xtask uses a
  direct path dependency.
- Bundle root layout: `manifest.json` is build-only; `index.html` is the
  rendered static shell; `pkg/**` contains only content-addressed runtime assets
  and their declared compressed sidecars. Site assembly copies `index.html` and
  `pkg/**`, never `manifest.json`.
- Manifest version 1 records each runtime asset's semantic role, `/pkg/` URL,
  identity SHA-256, and required representation paths/digests. Glue and WASM are
  unique required roles with identity, gzip, and Brotli; other modules require
  identity unless more representations are declared.
- `csr/index.html` becomes a source template. Devtool renders it only after the
  dependency-first graph rewrite. `server/build.rs` verifies the same manifest
  and generates host-side glue/WASM URL constants for the projector; `web`
  carries no final asset names.
- An experiment-only build selector may switch the complete legacy package (A)
  against the complete manifest/immutable package (B). It must alter both URL
  production and response policy together and be deleted with all salts after
  capture.

## Task outline

- [x] Task 1: Produce a deterministic, verified content-addressed bundle
  - Contract: implement the `csr_bundle` manifest interface and update
    `devtool csr-bundle` to discover runtime JS dependencies, reject cycles,
    rewrite leaves before importers, hash final identity bytes with full
    SHA-256, generate required sidecars, render the CSR shell, and write the
    bundle-root layout above atomically.
  - Verification: focused manifest/devtool tests prove stable output, nested
    module rewriting, final-byte digests, required role/representation coverage,
    and rejection of cycles, unsafe paths, duplicates, extras, missing files,
    unreferenced files, and digest mismatches.

- [x] Task 2: Consume the manifest in the embedded server and both shells
  - Depends on: Task 1 manifest and output layout.
  - Contract: make host and Nix bundle assembly pass the bundle root unchanged
    to `server/build.rs`; verify before staging; generate projector URLs; serve
    the generated static shell; remove fixed names/constants/aliases. Apply
    `public, max-age=31536000, immutable` to manifest-backed 200 and 304
    responses while preserving representation ETags, `Vary`, logical MIME, SPA
    fallback, and early-fetch/init ordering.
  - Verification: server build/unit/integration tests prove both rendered shells
    contain each manifest URL literal exactly once and preserve ordered early
    fetch, initializer fallback, and init; no legacy path remains. They also
    cover fail-closed staging and identity/gzip/Brotli body, MIME, cache,
    `Vary`, ETag, and matching 304 headers.

- [x] Task 3: Migrate host tooling and certify the integrated clean cutover
  - Depends on: Tasks 1 and 2.
  - Contract: update `build-csr`, Nix site assembly, `audit-wasm`, and
    `wasm-budget` to resolve the manifest's WASM role and generated shell;
    remove every fixed-path parser fixture/message/callsite. Keep raw-byte
    measurement and the current ceiling unchanged; keep the no-preload drift
    guards at the manifest seam.
  - Verification: focused xtask/tool tests prove manifest selection and missing
    or inconsistent artifacts fail; the integrated static-check lane is clean.
    Chromium and Firefox boot coverage exercise streaming and buffered paths
    with one WASM request and no fixed aliases.

- [ ] Task 4: Capture and publish the cache-effect A/B
  - Depends on: a committed, browser-certified implementation from Task 3.
  - Contract: pre-register A as the complete legacy package and B as the
    complete new package. Add an experiment-only `traces run --backend` selector
    whose absence preserves the existing both-backend behavior, and use
    `cargo xtask traces run --backend sqlite --single-worker --top 25` in both
    browsers, three runs per arm, A1/B1/A2/B2/A3/B3, with distinct `e2eSalt`
    values and otherwise identical settings. Sample `/proc/loadavg` before and
    after each run and repeat any run overlapping other CPU-intensive work.
  - Verification: require the current complete eligible navigation census,
    dropped=0, complete document marks, exact source/browser/cache-warmth
    groups, and recorded source/artifact identities. Publish the pre-registered
    prediction, per-run warm `wasm_fetch`, `/pkg` request and 200/304 counts,
    cold behavior, suite wall-clock, retries, before/after load samples, and the
    observed magnitude with no minimum-win threshold in `docs/observability.md`.
    Do not use browser `transferSize` as the cache oracle or mix document and
    Node clocks. Delete the A/B selector, backend selector and its temporary
    tests, and every salt; then rerun focused checks proving the delivered
    package is B and the trace runner retains its pre-issue interface.

## Risk checks

- The manifest cannot enter the embedded `/pkg/` tree or receive a public URL.
- A new runtime asset or sidecar cannot bypass inventory validation or immutable
  naming; cyclic imports fail rather than weakening identity.
- Server builds with a declared bundle root fail before compiling when the
  manifest, rendered shell, runtime inventory, or required representations
  drift.
- Brotli/gzip selection keeps `application/wasm`; every 304 repeats the cache
  and representation-selection headers needed by intermediaries.
- The public projector and static shell use identical manifest roles without
  compiling a final asset URL into WASM.
- `CONTEXT.md` remains unchanged because no ubiquitous domain term changes;
  `docs/README.md` remains promoter-owned.
- Each task reaches `jaunder-commit` only after its named evidence; no lint
  suppression is introduced without explicit user approval and no commit gains a
  `Co-Authored-By` trailer.
