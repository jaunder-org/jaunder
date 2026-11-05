# ADR-DRAFT: Vite owns CSR asset output and build inventory

- Status: proposed
- Date: 2026-11-04
- Issue: [#1884](https://github.com/jaunder-org/jaunder/issues/1884)

## Context

ADR-0175 assigns `devtool csr-bundle` ownership of content-addressed CSR URLs,
full-SHA filenames, cycle rejection, and a role-tagged inventory schema. The
isolated Vite 8.3.5 proof establishes an HTML build, a native build manifest,
and content-sensitive output URL changes for tested WASM and JavaScript changes.
It does not establish full-SHA collision properties, preserve all prior graph
policies, migrate consumers, or test production serving. The current manifest is
build-only; browser startup uses rendered shell URLs.

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
- This proposal is conditional on separately approved migration design and
  demonstrated host/Nix, consumer, browser-ordering, and HTTP-serving behavior.
  Until accepted, ADR-0175 remains current architecture.
