# ADR-DRAFT: Vite owns CSR asset output and build inventory

- Status: proposed
- Date: 2026-11-04
- Issue: [#1884](https://github.com/jaunder-org/jaunder/issues/1884)

## Context

ADR-0175 assigned the former producer ownership of content-addressed CSR URLs,
full-SHA filenames, cycle rejection, and a role-tagged inventory schema. The
isolated Vite proof established feasibility, and the production migration now
uses pinned Vite 8.3.5, `vite-plugin-compression2` 2.5.4, its native build-only
manifest and ordinary content-sensitive filenames. Production proof passed the
host and Nix builds, static shell/projector URL checks, URL stability and
runtime JS/WASM mutation invalidation, and the retained embedded HTTP path. The
native manifest is not served.

Host and Nix optimized WASM artifacts differ in bytes and URLs: 4,212,240 bytes
(host) and 4,222,496 bytes (Nix), both under the 5,000,000-byte ceiling. Pins,
features, tool versions and optimizer flags were checked in both pipelines. Path
provenance explains differing build contexts but does not prove the cause of
every differing byte; Nix intermediates were no longer available. Byte identity
across environments is not required. These measurements do not mean the
canonical `wasm-budget` gate passed, and full browser/HTTP/coverage and final
gate acceptance remains outstanding.

## Decision

If the proposed migration is approved and its acceptance evidence passes, use
the pinned Vite build and its ordinary content-sensitive output naming as the
source of CSR asset URLs and its native build manifest as build-only inventory.
Do not require full-SHA filenames or reproduce a role-tagged parallel schema,
custom graph rewriter, hash implementation, or cycle policy. Content-sensitive
names support cache invalidation for tested outputs; they are not a
cryptographic integrity or collision-resistance promise. Compute file digests
only where a concrete audit, qualification, or coverage consumer needs them.

The existing `rust-embed` and `site.rs` path remains responsible for serving
identity and compressed representations with logical-path MIME, `Vary`,
representation-specific validators, and immutable caching. Keep the manifest
outside the served site. Preserve raw post-`wasm-opt -Oz` size measurement
independently of bundler naming.

## Consequences

- ADR-0175's producer-owned full-SHA naming, mandatory cycle rejection, and sole
  role-tagged manifest clauses would be superseded; its runtime serving
  guarantees remain in force.
- Consumers must read the native manifest and ordinary output paths directly.
  Any retained adapters must express concrete consumer needs, not recreate the
  old schema.
- Production migration and its bounded proofs are implemented, but this ADR
  remains proposed pending outstanding browser/HTTP/coverage evidence and final
  gates. The host/Nix byte difference is accepted as environment-specific, not
  attributed solely to paths; measured sizes do not replace the canonical
  `wasm-budget` verdict.
