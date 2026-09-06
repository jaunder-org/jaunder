# ADR-DRAFT: The CSR bundle manifest owns content-addressed asset identity

- Status: proposed
- Date: 2026-09-03
- Issue: [#869](https://github.com/jaunder-org/jaunder/issues/869)

## Context

Long-lived immutable caching is safe only when an asset URL changes with its
content. Jaunder's CSR asset names were source literals shared by `web`, two
HTML shells, the server, and host audits, while the final bytes do not exist
until after the WASM client has compiled and devtool has run wasm-bindgen and
`wasm-opt`. Compiling the final WASM filename into `web` would make that
filename part of the bytes it hashes, creating a self-reference rather than a
stable content address.

The alternatives leave ownership unclear: consumers can guess asset roles from
embedded filenames, or a source revision can name the whole bundle. Discovery
turns filename conventions into a runtime interface; a revision names source,
not the final bytes, and invalidates unrelated assets together.

## Decision

`devtool csr-bundle` owns runtime asset identity. After final content
transformations, it rewrites the acyclic runtime import graph dependency-first
and assigns each asset a filename containing its full SHA-256 digest. Runtime
import cycles are a build error.

Devtool emits one build-only, role-tagged manifest outside the served `/pkg/`
tree. It inventories every runtime asset and every required representation: glue
and WASM require identity, gzip, and Brotli; other runtime modules require
identity unless more representations are declared. The manifest is the only
naming interface between bundle production, shell rendering, server staging, and
host audit/budget tooling. Producers and consumers fail closed when its roles,
paths, inventory, required representations, or digests disagree with the bundle.
The static CSR shell and server-side public-projector shell are rendered from it
after bundle production; asset URLs are not compiled into `web`. Fixed-name
aliases and the manifest itself are not served.

Manifest-backed `/pkg/` representations use
`Cache-Control: public, max-age=31536000, immutable`, retain
representation-specific ETags and `Vary: Accept-Encoding`, and derive MIME from
the logical asset rather than a compressed sidecar.

## Consequences

- Every runtime `/pkg/` file is independently invalidated when its final
  identity bytes change; a deploy with unchanged content keeps the same URL.
- The bundle manifest becomes a narrow build interface whose validation prevents
  a new runtime asset from silently receiving an unsafe immutable policy.
- Devtool must understand and reject cyclic runtime module graphs; adding a
  bundler or aggregate cycle identity would require revisiting this decision.
- Server builds require the manifest before embedding the CSR site, while the
  single-binary deployment and existing compressed serving model remain.
- WASM audit and raw-size budget tooling follow the manifest's WASM role without
  changing what raw bytes they measure.
