# ADR-DRAFT: Vite owns CSR asset output and build inventory

- Status: proposed
- Date: 2026-11-04
- Issue: [#1884](https://github.com/jaunder-org/jaunder/issues/1884)

## Context

ADR-0175 assigned the former producer ownership of content-addressed CSR URLs,
full-SHA filenames, cycle rejection, and a role-tagged inventory schema. These
responsibilities coupled a custom graph rewriter and hash implementation to
consumers that needed only emitted paths and runtime serving behavior.

Vite provides the HTML entry, emitted asset names, and a native build-only
manifest. Its build HTML transform receives the emitted output bundle, allowing
the shell's early classic WASM fetch to use the actual output name. The
application stylesheet remains an independently produced system artifact.

## Decision

Use the pinned Vite build and its native content-sensitive output names and
manifest for CSR build inventory. Do not require full-SHA filenames or reproduce
a parallel role-tagged schema, graph rewriter, hash implementation, or cycle
policy. Content-sensitive names support cache invalidation; they are not a
cryptographic integrity or collision-resistance promise. Compute digests only
where a concrete audit, qualification, or coverage consumer needs them.

Keep the manifest outside the served site. The existing `rust-embed` and
`site.rs` path remains responsible for identity and compressed representations,
logical-path MIME, `Vary`, representation-specific validators, and immutable
caching. Preserve the raw post-`wasm-opt -Oz` size measurement independently of
bundler naming.

## Consequences

- The former producer-owned full-SHA naming, mandatory cycle rejection, and
  role-tagged manifest are no longer required; runtime serving guarantees
  remain.
- Consumers read native manifest paths for their actual needs without
  reconstructing the former schema.
- Content-sensitive names are an output-naming behavior, not an integrity
  assertion; the serving and cache-membership contracts remain independently
  enforced.
