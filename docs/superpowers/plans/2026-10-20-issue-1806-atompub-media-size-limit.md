# AtomPub Media Upload Size Limit Alignment Implementation Outline

> Execute with `jaunder-iterate`. This outline exists because the change adjusts
> a public AtomPub request boundary and must preserve finite resource handling.

## Scope

In:

- Make only `POST /atompub/{username}/media` honor the existing configured
  input-size bound.
- Preserve the shared image privacy preparation and prepared-output-size
  enforcement, authentication and filename checks, upload capability behavior,
  and truthful oversize responses.
- Add real HTTP integration and Playwright E2E regression coverage required by
  the approved spec.

Out:

- Changing global request limits, web upload behavior, or the media-size
  setting.
- Production configuration/proxy work, retries, unrelated AtomPub operations, or
  redesign of the shared media preparation pipeline.

## Task outline

- [x] Task 1: Align AtomPub media POST request handling with the configured
      limit and prove the full protocol behavior.
  - Contract: the effective finite request bound is the existing
    `media.max_file_size_bytes`, including when below Axum's 2 MiB default; no
    global limit is disabled and no independent lower AtomPub cap is introduced.
    Accepted requests continue through the existing shared media preparation and
    persistence path.
  - Verification: backend-parametric HTTP integration tests prove a >2 MiB
    in-limit upload succeeds, a below-2-MiB configured limit accepts an in-limit
    upload and rejects an over-limit request without creating Media, exact
    boundary behavior follows the shared media limit semantics, and
    auth/error/output-preparation behavior remains intact. Playwright coverage
    extends the existing AtomPub request flow in `end2end/tests/atompub.spec.ts`
    with a >2 MiB in-limit upload.
  - Focused Rust proof:
    `devtool run -- cargo xtask test-local -- -p jaunder -E 'test(/^atompub::atompub_media::/)'`.
  - Focused browser proof:
    `devtool run -- cargo xtask e2e-local atompub.spec.ts`.

## Risk checks

- Enforce the setting before body buffering can exceed its configured finite
  bound; do not rely only on the manager's later check after full-body
  extraction.
- Rejection must happen before Media persistence, including for configured
  limits below 2 MiB.
- Preserve current image privacy preparation and its prepared-output-size check;
  preserve auth and sanitized Slug behavior.
- Do not claim the prior production HTTP 413 came from Axum, the application
  setting, or a proxy.
