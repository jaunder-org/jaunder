# Name media and auth DTOs for their content

## Outcome

The three remaining public `*Result`/`*Response` DTO names in `common` and `web`
disappear. Media payloads gain content nouns, while login returns the existing
complete `SessionUser` instead of a one-field bootstrap fragment.

## Load-bearing decisions

- Rename `UploadResponse` to `UploadedMedia` in `common::media` and migrate
  every storage, web, AtomPub, integration-test, and client reference.
- Preserve `UploadedMedia`'s five fields and serde field names exactly: content
  hash, canonical filename, content type, byte size, and root-relative URL.
- Keep `UploadedMedia` distinct from the media-list `Item`. An uploaded value
  has implicit `MediaSource::Upload` and no creation timestamp; making list-only
  fields optional would weaken both shapes.
- Multipart form data remains the upload input codec only. The output remains
  the same JSON object; the Rust type rename is wire-transparent.
- Delete `LoginResponse`. `login` returns the existing `SessionUser`, populated
  from the authenticated user record's canonical username and operator flag.
- The login client stores the returned `SessionUser` directly instead of
  reconstructing one from the submitted username plus a response fragment.
- Login still establishes the real session only through the `HttpOnly` cookie.
  The body contains no session token or credential; returning the complete
  advisory marker identity does not change authorization semantics.
- Rename `DeleteResult` to `MediaDeletion` and migrate every web and
  integration-test reference.
- Preserve `MediaDeletion`'s `deleted` and `referenced_in_posts` fields and JSON
  names. Do not replace the object with an enum or generalize its post-specific
  blocker field.
- These are vertical-specific content names under ADR-0097's deferred media/auth
  work; they do not extend the posts content-weight axis.
- No new ADR or glossary entry is warranted: the changes apply existing naming
  and session decisions and introduce no durable architectural trade-off or new
  domain relationship.
- Update ADR-0097, ADR-0107, ADR-0084, and `docs/ARCHITECTURE.md` where their
  present-tense projections name the old DTOs or the one-field login body.
  Historical archive documents remain unchanged.

## Acceptance

- No public struct under `common/` or `web/` ends in `Result` or `Response`.
- No reference to `UploadResponse`, `LoginResponse`, or `DeleteResult` remains.
- Upload through the multipart server function serializes exactly the `sha256`,
  `filename`, `content_type`, `size_bytes`, and `url` JSON keys; the existing
  canonical filename, content type, URL, and serving behavior remain covered.
- Storage byte uploads and AtomPub media creation still consume the uploaded
  media identity without conversion or duplicated DTOs.
- A successful login returns the exact `SessionUser` JSON shape: authenticated
  canonical `username` and `is_operator`.
- The login integration contract still recovers the real session token from
  `Set-Cookie`, proves the cookie is set, and proves that token value occurs
  nowhere in the response body.
- The login component writes that returned `SessionUser` directly into the
  shared marker/session context; successful redirect, cookie, first-login
  chrome, and error behavior are unchanged.
- Media deletion serializes exactly the `deleted` and `referenced_in_posts` JSON
  keys, still distinguishes deletion from a reference refusal, and reports the
  exact refusing Post IDs used by the UI.
- Existing focused unit, integration, and end-to-end contracts for auth and
  media pass after the clean cutover.
- The applicable xtask verification ladder is green.

## Boundaries

- No media-list `Item` unification, optional-field expansion, or upload field
  removal.
- No bare-boolean login response and no new login-specific wrapper.
- No enum wire migration for media deletion.
- No endpoint path, server-function input, multipart parsing, cookie,
  authorization, storage schema, or domain-newtype change.
