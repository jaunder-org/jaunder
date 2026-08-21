# Reuse the Media Upload Request Helper Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor `end2end/tests/media.spec.ts` so one spec-scoped helper owns
the authenticated success-path media upload request.

**Scope:** In scope: Playwright e2e test helper placement and audited call-site
rewrites in `end2end/tests/media.spec.ts`. Out of scope: production media code,
server-function routing, unauthenticated upload behavior, widget-upload tests,
and media delete/reference assertions.

**Tasks:** Task 1 hoists `uploadMedia`, routes audited success-path callers
through it, and verifies the focused spec plus `cargo xtask check`.

**Key risks/decisions:** Preserve the raw unauthenticated upload test; preserve
caller-owned assertions for served content, cache headers, canonical filename
spelling, delete dispatch counts, and reference guards. The helper needs
caller-specific content for served-byte assertions but should keep the existing
default content for delete-guard callers.

**Architecture:** This is a local e2e test refactor. The existing
delete-guard-local `uploadMedia` becomes a spec-scoped helper above both
`test.describe` blocks; the helper still uses `page.request.post` with the
authenticated page's cookie jar and returns the server's upload response.

**Tech Stack:** Playwright, TypeScript, Jaunder e2e helpers,
`cargo xtask e2e-local`, `cargo xtask check`.

## Global Constraints

- Follow the approved spec:
  `docs/superpowers/specs/2026-08-21-issue-1036-reuse-media-upload-helper.md`.
- Do not change production code, server-function routing, media semantics, or
  public/test interfaces outside the local helper shape needed by this spec.
- Keep `unauthenticated upload is rejected` raw.
- Keep served-file, canonical-name, cache, delete, and reference assertions
  local to their owning tests.
- Run commands via `devtool run -- ...` when inspecting or acting on their
  result.
- No `Co-Authored-By` trailer in commits.

---

## File Structure

- Modify: `end2end/tests/media.spec.ts`
  - Owns the spec-scoped `uploadMedia` helper.
  - Keeps media upload/serving tests and media delete guard tests in their
    current describe blocks.
- No new files.

## Scope Check

The approved spec covers one test file and one helper seam. No separable
concerns need filing before implementation.

### Task 1: Hoist and Reuse the Authenticated Upload Helper

**Files:**

- Modify: `end2end/tests/media.spec.ts`
- Test: `end2end/tests/media.spec.ts`

**Interfaces:**

- Consumes:
  - `BASE_URL` from `./helpers`
  - `expect` from `./fixtures`
  - `Page` from `@playwright/test`
- Produces:

```ts
type UploadResponse = { url: string; filename: string };

async function uploadMedia(
  page: Page,
  name: string,
  content?: Buffer,
): Promise<UploadResponse>;
```

The helper posts multipart form data to `BASE_URL + "/api/media/upload"` with
`mimeType: "image/jpeg"`, asserts `response.status()` is `200`, and returns
`await response.json()`. When `content` is omitted, it uses
`Buffer.from("delete guard content")`.

- [x] **Step 1: Establish the behavioral baseline**

Run:

```bash
devtool run -- cargo xtask e2e-local media.spec.ts
```

Expected: PASS. If this fails before edits, stop and diagnose the existing spec
failure before refactoring.

Result: the first sandboxed attempt failed before tests because the local server
could not become reachable (`Operation not permitted`). The rerun with approved
local server/Playwright permissions passed before the refactor.

- [x] **Step 2: Move the helper to spec scope**

In `end2end/tests/media.spec.ts`, move the existing `uploadMedia` function out
of `test.describe("Media delete guard", ...)` and place it after the imports and
before `test.describe("Media upload and serving", ...)`.

Add the local response type and optional content parameter:

```ts
type UploadResponse = { url: string; filename: string };

/** Uploads `name` and returns the upload response (`url`, canonical `filename`). */
async function uploadMedia(
  page: Page,
  name: string,
  content: Buffer = Buffer.from("delete guard content"),
): Promise<UploadResponse> {
  const response = await page.request.post(BASE_URL + "/api/media/upload", {
    multipart: {
      file: {
        name,
        mimeType: "image/jpeg",
        buffer: content,
      },
    },
  });
  expect(response.status()).toBe(200);
  return await response.json();
}
```

Remove the old nested helper from the delete-guard describe block.

- [x] **Step 3: Route audited callers through the helper**

Replace the authenticated success-path inline upload requests with `uploadMedia`
calls:

```ts
const fileContent = Buffer.from("fake image content for testing");
const json = await uploadMedia(page, "test-image.jpg", fileContent);
```

```ts
const fileContent = Buffer.from("spaced filename content");
const json = await uploadMedia(page, "my holiday photo.jpg", fileContent);
```

```ts
await uploadMedia(
  page,
  "my holiday photo.jpg",
  Buffer.from("spaced filename content"),
);
```

Leave these behaviors local and unchanged:

- `expect(json.filename).toBe("test-image.jpg")`
- `expect(json.url).toContain("/media/upload/")`
- cache-control assertion on the served response
- canonical encoded filename and URL assertions
- served text assertion for `"spaced filename content"`
- media row visibility, delete request/list request counts, dialog handling,
  disabled button assertion, and removal assertion

Do not change `unauthenticated upload is rejected`; it must keep its raw
`page.request.post(BASE_URL + "/api/media/upload", ...)` request and rejection
assertions.

- [x] **Step 4: Verify focused behavior**

Run:

```bash
devtool run -- cargo xtask e2e-local media.spec.ts
```

Expected: PASS.

- [x] **Step 5: Verify the repository check**

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS. If formatting or other check-mode auto-fixes modify files,
inspect them and keep only changes that belong to this issue.

- [x] **Step 6: Commit**

Stage only the spec, plan checkbox update, and test-file changes for this issue:

```bash
git add docs/superpowers/specs/2026-08-21-issue-1036-reuse-media-upload-helper.md
git add docs/superpowers/plans/2026-08-21-issue-1036-reuse-media-upload-helper.md
git add end2end/tests/media.spec.ts
git commit -m "test(e2e): reuse media upload helper"
```

Expected: commit succeeds; the pre-commit hook runs its gate. Do not add a
`Co-Authored-By` trailer.

## Self-Review

- Spec coverage: Task 1 covers the single spec-scoped helper, all audited
  authenticated callers, the unauthenticated raw exclusion, preservation of
  local assertions, focused e2e verification, and `cargo xtask check`.
- Placeholder scan: no TODO/TBD placeholders.
- Type consistency: the produced helper signature is used by every planned
  call-site rewrite and remains compatible with existing delete-guard callers
  that pass only `page` and `name`.
