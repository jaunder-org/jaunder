# Issue #859 server-argument Bad Request implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` only if execution
> develops an independently owned slice. This outline exists because the change
> establishes a framework-adapter seam and changes a public HTTP protocol
> contract.

## Scope

In:

- Preserve typed server-function input-decode classification across the internal
  framework response hop.
- Normalize malformed typed arguments to HTTP 400 at the single `/api` routing
  seam while preserving the existing public error body.
- Cover ordinary requests, progressive-enhancement redirects, unchanged 500
  paths, and documentation of the corrected architecture.

Out:

- Framework forks, Cargo patches, dependency upgrades, copied Leptos handlers,
  per-handler mappings, error-message parsing, and public body redesign.

## Task outline

- [x] Task 1: Make the real server-function route enforce the 400/500 contract
  - Contract: `WebError` provides one internal, machine-readable classification
    for `Args`, `MissingArg`, and input-side `Deserialization`. A dedicated
    response-normalizer module consumes it at `/api`, restores 400 and removes
    `Location` for malformed progressive-enhancement responses, strips the
    classification, and emits the established public
    `WebError::ServerFunction { message }` representation. All other errors and
    valid redirects pass through unchanged.
  - Verification: focused structural tests cover every `ServerFnErrorErr`
    classification branch, including direct proof that output-side
    `Serialization` is not classified as client input. Real-router integration
    tests cover URL/form, JSON, missing arguments where the framework exposes
    them distinctly, tag non-leakage, malformed-form redirect removal, a valid
    redirect, and a post-decode 500. Existing `PostBody`, `PostFormat`, and
    inbound-password regressions assert 400 on both storage backends. Run the
    focused `cargo xtask test-local` filters owning these contracts, then the
    `jaunder-commit` gate.

## Risk checks

- The normalizer parses only Jaunder's internal structured representation; no
  display string or domain message controls status.
- The internal classification cannot cross the public boundary on 400, 500, or
  redirected responses, and normalizing the body does not change the established
  public variant or message.
- The `/api` normalizer does not affect non-server-function routes, successful
  responses, valid progressive-enhancement redirects, or genuine server errors.
- Body collection and reconstruction preserve required headers and do not
  introduce an unbounded allocation beyond the framework's existing error-body
  limits; if no bound exists, the adapter must avoid buffering arbitrary success
  bodies.
- URL/form and JSON codec paths are both represented; per-codec duplication does
  not replace framework error-category classification.
- ADR-0065 and `docs/ARCHITECTURE.md` remain aligned with delivered behavior;
  `CONTEXT.md` remains unchanged because the domain language does not change.
