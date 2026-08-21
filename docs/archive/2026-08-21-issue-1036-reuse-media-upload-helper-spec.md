# Issue 1036: Reuse the Media Upload Request Helper

## Summary

`end2end/tests/media.spec.ts` repeats the same authenticated success-path media
upload request in several tests. The delete-guard block already has an
`uploadMedia` helper, but it is scoped too narrowly for the rest of the spec to
reuse it. Move that helper to spec scope and route the audited upload callers
through it without changing the behavior each test proves.

This is a Playwright e2e test refactor only. It must not change production code,
server-function routing, media semantics, or public/test interfaces outside the
local helper shape needed by this spec.

## Current State

- `authenticated user can upload and access media` builds a multipart
  `page.request.post(BASE_URL + "/api/media/upload", ...)` inline.
- `a filename needing percent-encoding uploads and serves` builds the same
  upload request inline with different filename/content.
- `ordinary media delete confirms and removes unreferenced item` builds the same
  upload request inline before exercising the media library row.
- The `Media delete guard` describe block defines `uploadMedia(page, name)` and
  uses it from its existing delete-guard callers.
- `unauthenticated upload is rejected` intentionally drives the upload endpoint
  raw because it proves the no-session rejection path.

## Decisions

- Hoist the existing `uploadMedia` helper to spec scope, outside both
  `test.describe` blocks.
- Keep the helper focused on authenticated success-path uploads. It should
  construct the multipart upload request, assert the successful response status,
  and return the parsed `{ url, filename }` upload response.
- Allow the helper to accept caller-specific file content where a test asserts
  served bytes. Default content is acceptable for tests that only need a valid
  uploaded media item.
- Route these audited callers through the helper:
  - `authenticated user can upload and access media`
  - `a filename needing percent-encoding uploads and serves`
  - `ordinary media delete confirms and removes unreferenced item`
  - the delete-guard tests that already call `uploadMedia`
- Keep `unauthenticated upload is rejected` raw so it continues to prove the
  unauthenticated endpoint behavior directly.
- Keep served-file assertions, canonical filename assertions, cache-header
  assertions, delete-request/list-request counting, delete confirmation, and
  reference-guard assertions local to the tests that own those behaviors.

No new domain vocabulary or architectural decision is introduced; this does not
require a `CONTEXT.md` or ADR update.

## Acceptance Criteria

- A single spec-scoped upload helper owns the authenticated success-path
  multipart `media/upload` request in `end2end/tests/media.spec.ts`.
- All audited authenticated success-path callers listed above use that helper.
- `unauthenticated upload is rejected` still performs its own raw upload
  request.
- Existing observable behavior is unchanged:
  - ordinary upload still returns the original display filename and a media URL;
  - percent-encoding upload still returns the canonical encoded filename and
    URL;
  - served-file content and cache headers are still asserted by their owning
    tests;
  - media deletion and reference-guard assertions remain local to their tests.
- The affected focused e2e spec passes with
  `cargo xtask e2e-local media.spec.ts`.
- `cargo xtask check` passes.
