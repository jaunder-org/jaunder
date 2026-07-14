# Spec — consolidate duplicated server HTTP request/response test helpers (#429)

## Context

Follow-on to #358 (consolidated the `post_form*` family) and #298 (collapsed the
six `server/tests` crates into one `integration` binary;
`server/tests/helpers/mod.rs` is the single shared `mod helpers;`, reachable
everywhere as `crate::helpers::…`). This is the same standing entropy-fighting
practice: lift duplicated per-file HTTP plumbing helpers into `helpers/` and
convert the call sites. Pure dedup — **no** suppression is added or removed; the
crate-level `#![expect(clippy::unwrap_used, clippy::expect_used)]` in
`server/tests/main.rs` already covers the consolidated helpers.

Investigation established the exact per-copy shapes (they are **not** all
identical, as the issue assumed — see Decisions).

## Decisions (resolved in design interview)

1. **`make_app` — single helper that always creates the media dirs.** The 6
   copies split into two variants: three (`feed/feed_handlers`,
   `misc/media_handlers`, `atompub/atompub_media`) create
   `media/{upload,cached,tmp}` under the `TempDir`; three
   (`atompub/atompub_posts`, `atompub/atompub_rsd`, `atompub/atompub_service`)
   do not. The dirs are created empty under the per-test `TempDir` and are never
   asserted against, so unconditionally creating them is a no-op for the three
   non-upload tests. Collapse to one `helpers::make_app` that always creates
   them.

2. **`post_json` — `serde_json::Value` body, folded onto `post_form_inner`.**
   The two web copies take a `serde_json::Value` param; the feed copy takes
   `impl Into<String>` and its caller stringifies a `Value`
   (`body.to_string()`). A `serde_json::Value` param unifies all three with
   near-zero churn. The unified signature is
   `post_json(state, uri, body: serde_json::Value, cookie: Option<&str>) -> (StatusCode, String)`.
   It reuses the shared request machinery: `post_form_inner` is renamed
   `post_inner` and its `body: impl Into<String>` is replaced by a `PostBody`
   enum (`Form(String)`/`Json(String)`) that carries the body **and** its
   content type together — every `post_form*` wrapper passes
   `PostBody::Form(body.into())`, and `post_json` passes
   `PostBody::Json(body.to_string())` with `secure_cookies = true`, cookie auth,
   dropping the `Set-Cookie` value (mirroring the canonical `post_form`). **Why
   the enum, not a `content_type: &str` param:** the original inner already had
   7 arguments (the `clippy::too_many_arguments` limit); an 8th would trip the
   lint (a suppression needs approval), so body+content-type collapse into one
   argument instead.

   **Feed reconciliation (discovered in implementation).** Two of the feed
   `post_json` call sites — `/api/unpublish_post` and `/api/delete_post` — pass
   a **form-encoded** string (`format!("post_id={post_id}")`), not a JSON value;
   the old local `post_json` sent them under `Content-Type: application/json` (a
   latent mismatch the form-parsing server fns tolerated). They can't use the
   `Value`-typed `post_json`, so they are routed to the existing `post_form`
   instead — which labels the body `application/x-www-form-urlencoded`,
   correcting the content type. Verified behavior-preserving by the dual-backend
   gate (both tests still `200 OK`); `web_posts` already drives the same
   endpoints via `post_form` (`unpublish_post_form`).

3. **`get_asset` — moved to `helpers/`.** Single-use (`misc/static_assets`) and
   a distinct shape (GET, sets up its own `Backend::Sqlite` backend, returns
   `(StatusCode, Option<String>)` content-type rather than a body). Moved to
   `helpers/` for consistency with the other HTTP plumbing per the maintainer's
   call, even though it is not duplicated. `helpers/mod.rs` gains the
   `Backend`/`TestEnv` imports it needs.

4. **`body_string` — verbatim lift.** The 4 copies are byte-identical.

## Target design — `server/tests/helpers/mod.rs`

Add four public items (and generalize the private inner):

- `pub fn make_app(state: Arc<storage::AppState>, storage: &TempDir) -> axum::Router`
  — registers server fns, creates `media/{upload,cached,tmp}` under
  `storage.path()`, builds the router with the noop mailer and
  `secure_cookies = false`.
- `pub async fn body_string(response: axum::response::Response) -> String` —
  verbatim.
- `pub async fn post_json(state, uri, body: serde_json::Value, cookie: Option<&str>) -> (StatusCode, String)`
  — over the generalized `post_inner` with `PostBody::Json`.
- `pub async fn get_asset(uri: &str) -> (StatusCode, Option<String>)` —
  verbatim, using the shared imports.
- private `post_form_inner` → `post_inner`; its `body: impl Into<String>`
  becomes a `PostBody` enum carrying body + content type (see Decision 2); all
  `post_form*` wrappers pass `PostBody::Form(body.into())`.

## Acceptance criteria

Each is observable from the diff and a green gate.

- **AC1 — one definition each.** After the change, `make_app`, `body_string`,
  `post_json`, and `get_asset` are each defined exactly once, in
  `server/tests/helpers/mod.rs`. No `fn make_app`, `fn body_string`,
  `fn post_json`, or `fn get_asset` remains in any file under
  `server/tests/{atompub,feed,misc,web}/`.
- **AC2 — call sites converted.** Every former call site now resolves to the
  `helpers::` definition (via `use crate::helpers::…` or a `crate::helpers::`
  path). `post_json` call sites match the `serde_json::Value` + `Option<&str>`
  signature: web_posts unchanged, feed no longer `.to_string()`s the body,
  web_tags passes `None` for the cookie.
- **AC3 — imports pruned.** Each touched test file's `use` block is reduced to
  what the file still uses after its local helper defs are removed (no leftover
  `unused_imports` — these would fail the crate-level expect / clippy). Imports
  pulled up into `helpers/mod.rs` are added there.
- **AC4 — behavior preserved.** `make_app` always creating the media dirs does
  not change the outcome of any atompub test; the full server integration suite
  passes on both backends unchanged.
- **AC5 — no new suppressions.** No `#[allow]`/`#[expect]` added or removed
  anywhere. `git diff main...HEAD` shows no new suppression attribute.
- **AC6 — gate green.** `cargo xtask validate` is green.

## Out of scope / non-goals

- No suppression removal (explicitly per the issue).
- No change to `post_form*` wrapper _signatures_ — only the private inner
  changes (body param becomes `PostBody`).
- No behavioral change to any test assertion.

## Notes

- Blocker #298 is **closed/completed** (2026-07-14); the single-binary structure
  this spec targets is in place.
- Review anchor: three-dot `git diff main...HEAD` (fork-point tag
  `wt-base-issue-429` marks the same base).
