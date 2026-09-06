# Issue #869 — content-addressed `/pkg` assets

## Outcome

Jaunder serves every runtime CSR asset under a content-addressed `/pkg/` URL
with a public one-year immutable cache policy. Warm browser navigations reuse
the bundle without conditional round trips, while cold boot, compression,
streaming instantiation, and the single-binary deployment remain intact.

## Load-bearing decisions

- `devtool csr-bundle` owns asset identity after all content transformations.
  Each runtime asset receives a full SHA-256 hexadecimal digest in its filename,
  computed from its final served identity bytes.
- The runtime module graph is rewritten dependency-first. Cyclic runtime imports
  are rejected at build time rather than assigned weaker group identities or
  introducing another bundler.
- Devtool emits one build-only, role-tagged bundle manifest outside the staged
  `/pkg/` tree. It covers every runtime asset and its required representations:
  glue and WASM require identity, gzip, and Brotli; any other runtime module
  requires identity unless the manifest declares additional representations. The
  manifest is the sole naming interface for shell rendering, server staging, and
  host audits.
- Manifest production and consumption fail closed on missing, extra, duplicate,
  path-escaping, digest-mismatched, or unreferenced runtime files and required
  representations. Required glue and WASM roles are unique.
- The CSR shell and public-projector shell are generated from the same manifest.
  Asset names do not live in `web`: embedding the final WASM name into the WASM
  itself would create a content-hash self-reference.
- Fixed `/pkg/jaunder.js` and `/pkg/jaunder.wasm` assets and aliases are
  removed. A deployment changes an asset URL exactly when that asset's final
  content changes.
- Every manifest-backed `/pkg/` response, including `304 Not Modified`, carries
  `Cache-Control: public, max-age=31536000, immutable`. Representation-specific
  ETags and `Vary: Accept-Encoding` remain.
- Content negotiation continues to select Brotli, gzip, or identity bytes while
  deriving MIME from the logical runtime asset, so WASM remains
  `application/wasm` and `instantiateStreaming` remains valid.
- WASM audit and budget tooling resolve the manifest-selected artifact. The raw
  WASM budget and its existing ceiling retain their meaning.
- No preload is introduced. The accepted no-preload and no-prewarm decisions
  remain in force.
- Performance is a reported outcome, not a retention gate. Correct content
  addressing and successful browser compatibility retain the feature even when
  timing deltas fall below noise.
- The bundle-manifest ownership seam is recorded in
  `docs/adr/drafts/content-addressed-csr-bundle-manifest.md` and projected into
  `docs/ARCHITECTURE.md`. It does not change Jaunder's domain vocabulary.

## Acceptance

- A production CSR bundle contains no unhashed runtime file under `/pkg/`, no
  fixed-name compatibility alias, and no served manifest. Its build-only
  manifest matches the staged runtime inventory and verifies every required
  identity and compressed representation; glue and WASM each have identity,
  gzip, and Brotli bytes.
- Both the static CSR shell and a rendered public-projector document reference
  the manifest's glue and WASM URLs exactly once and preserve early-fetch,
  fallback, and initialization ordering.
- Producer tests prove deterministic graph rewriting and manifest output,
  including rejection of cycles and every invalid-manifest class above.
- Server tests prove fail-closed staging and, for identity, gzip, and Brotli,
  correct MIME, ETag, `Vary`, immutable `Cache-Control`, body bytes, and
  matching `304` headers.
- Audit and budget tests prove they select the manifest's WASM role rather than
  a fixed path. The existing raw-byte ceiling still gates the same optimized
  compiler input.
- Existing Chromium and Firefox boot coverage loads the generated bundle through
  streaming and buffered initialization paths without duplicate WASM requests.
- The performance record compares the complete pre-change package (A) with the
  complete content-addressed immutable package (B): single-worker SQLite,
  Chromium and Firefox, three runs per arm and browser, run-by-run interleaved,
  distinct `e2eSalt` values, no prewarm, and otherwise gate-identical settings.
- The deciding analysis uses the current complete eligible navigation census,
  grouped by exact trace source, browser, and cache warmth. It requires zero
  dropped records and complete marks; document-frame WASM timing is never mixed
  with Node-frame lifecycle timing.
- The record publishes per-run warm `wasm_fetch`, `/pkg` request counts and
  server `200`/`304` outcomes, cold-load behavior, suite wall-clock, retries,
  artifact/source identities, and before/after `/proc/loadavg` samples. Runs
  taken while any other CPU-intensive work, including another agent's build or
  test, is active are discarded and repeated. The record predicts that warm
  revalidations disappear and warm `wasm_fetch` falls, then reports the observed
  magnitude without a minimum-win threshold.

## Boundaries

- Do not change public assets outside `/pkg/`, base stylesheet handling, the
  WASM optimization level, or the raw-size ceiling.
- Do not add a preload, a test warmup, a stable asset alias, a new JS bundler,
  or runtime filename discovery by extension or prefix.
- Do not infer cache hits from browser `transferSize`; server request/status
  evidence is authoritative for revalidation and request avoidance.
- Experiment-only salts or switches must not remain in the delivered tree.
