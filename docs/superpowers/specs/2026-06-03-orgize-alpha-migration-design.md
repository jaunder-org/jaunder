# Migrate to `orgize 0.10` and make post rendering infallible

**Bead:** `jaunder-c8wv` — "Move to using the alpha version of `orgize`"

**Status:** Approved design (2026-06-03)

## Problem

The post body renderer uses `orgize 0.9`, which does not render Org-mode links
(`[[url][text]]`) as HTML anchors — a non-starter for publishing. The fix is to
move to the `orgize 0.10` line (currently `0.10.0-alpha.10`, a published
pre-release), which renders links correctly.

`orgize 0.10` is a ground-up rewrite (rowan-based) with a new, **infallible**
HTML export API: `Org::parse(body).to_html()` returns a `String` (the `0.9`
`write_html(&mut buf)` + UTF-8 conversion, which were the only fallible steps,
are gone). Because rendering can no longer fail, the entire `RenderError` error
path through the codebase becomes dead and is removed (approach "2b": full
cleanup, including collapsing the now-single-variant wrapper error enums).

## Scope (agreed)

Definition of done (scope "A"): migrate `render_org` to `orgize 0.10.0-alpha.10`;
Org links render as real `<a href>` anchors; all currently-tested Org features
(headings, bold/italic/code, lists, code blocks) still render; all tests green
with `org_link` strengthened to assert a genuine anchor. A broader audit of
other Org constructs is explicitly out of scope (file follow-up beads instead).

Cleanup depth (agreed "2b"): make rendering infallible end-to-end, delete
`RenderError`, and **delete** the single-variant wrapper enums
`CreateRenderedPostError` / `UpdateRenderedPostError` (not merely strip their
`Render` arm).

Dependency-source policy (agreed "C"): use the published `0.10.0-alpha.10`. If it
still fails to render links (or has another blocker), **stop and reassess** with
the user; a git/fork dependency is a deliberate last resort, not an automatic
fallback.

## Current state (verified 2026-06-03)

`orgize`'s entire footprint is one function plus the workspace dep:

- Root `Cargo.toml:59`: `orgize = "0.9"` (locked `0.9.0`).
- `common/Cargo.toml:13`: `orgize.workspace = true`.
- `common/src/render.rs:226`: `render_org` (the only call into orgize).

The `RenderError`/`::Render` plumbing fans out from there:

- `common/src/render.rs`: `RenderError` enum (`:55`), `render()` returns
  `Result<String, RenderError>` (`:69`), `render_org` returns
  `Result<String, RenderError>` (`:226`); tests `render_error_display`,
  `render_error_debug`.
- `storage/src/lib.rs:63`: `pub use common::render::RenderError;`.
- `storage/src/post_service.rs`: error enums `CreateRenderedPostError` (`:22`,
  variants `Render`+`Storage`), `UpdateRenderedPostError` (`:30`, `Render`+`Storage`),
  `PerformUpdateError` (`Render` at `:115`), `PerformCreationError` (`Render` at
  `:201`). `render(&body, &format)?` at `:57`, `:88`, `:169`. The
  `perform_post_creation` retry match has an `Err(CreateRenderedPostError::Render(e))`
  arm (`:279`).
- `web/src/posts/server.rs`: `perform_update_error` (`:241`, arm
  `PerformUpdateError::Render(_) => InternalError::server(error)` at `:249`),
  `perform_creation_error` (`:254`, arm
  `PerformCreationError::Render(e) => InternalError::validation(e.to_string())`
  at `:264`); test refs at `:306`, `:338`.
- `server/src/atompub/posts.rs`: `creation_status` (`:178`), `update_status`
  (`:188`) match on `::Render`; test refs/import at `:412`, `:429`, `:459`.
- `server/tests/storage.rs`: integration tests for `create_rendered_post` /
  `update_rendered_post` that name `CreateRenderedPostError::Storage(_)` (`:5480`)
  and `UpdateRenderedPostError::Storage(_)` (`:5588`), imports at `:5436`, `:5571`.

`create_rendered_post` / `update_rendered_post` are public; their only callers are
`perform_post_creation` (uses `create_rendered_post`) and the
`server/tests/storage.rs` integration tests. Neither web nor server production
code calls them.

## Design

### 1. Dependency bump
Root `Cargo.toml`: `orgize = "0.9"` → `orgize = "0.10.0-alpha.10"`. No other
crate references orgize. `storage` already dropped its orgize dep in prior work.

### 2. `common/src/render.rs` — infallible rendering
- `render_org`:
  ```rust
  fn render_org(body: &str) -> String {
      orgize::Org::parse(body).to_html()
  }
  ```
- `render`:
  ```rust
  pub fn render(body: &str, format: &PostFormat) -> String {
      match format {
          PostFormat::Markdown => render_markdown(body),
          PostFormat::Org => render_org(body),
          PostFormat::Html => body.to_string(),
      }
  }
  ```
- Delete the `RenderError` enum and its `use` of `thiserror::Error` if it becomes
  unused (let the compiler decide).
- Tests: delete `render_error_display`, `render_error_debug`. Drop `.unwrap()`
  from `render_dispatches_markdown`, `render_dispatches_org`,
  `render_html_format_is_identity` and the seven `org_*` tests (they now receive
  `String`). **Strengthen `org_link`** to assert a real anchor, e.g.
  `assert!(html.contains("<a href=\"https://example.com\""))` and that the label
  `example` is present. Update `org_empty_input`'s structural-tag stripping to
  match 0.10's wrapper output (verify the exact empty-document HTML and strip
  accordingly).

### 3. `storage/src/post_service.rs` — collapse wrappers, drop `Render`
- Delete `CreateRenderedPostError` and `UpdateRenderedPostError`.
- `create_rendered_post` returns `Result<i64, CreatePostError>`; body builds the
  input with the infallible `render(...)` and returns `storage.create_post(&input).await`.
- `update_rendered_post` returns `Result<PostRecord, UpdatePostError>`; body uses
  infallible `render(...)` and returns `storage.update_post(...).await`.
- Remove the `Render` variant from `PerformUpdateError` and `PerformCreationError`.
- `perform_post_update`: `render(&body, &format)?` → `render(&body, &format)`.
- `perform_post_creation`: rewrite the retry match to match `CreatePostError`
  directly:
  ```rust
  match create_rendered_post(...).await {
      Ok(post_id) => { /* unchanged: fetch + return */ }
      Err(CreatePostError::SlugConflict) => {}
      Err(CreatePostError::Internal(e)) => return Err(PerformCreationError::Storage(e)),
  }
  ```
- Update the `use common::render::{...}` import to drop `RenderError`.
- Tests: remove the wrapper-error tests (`create_rendered_post_error_*`,
  `update_rendered_post_error_*`) and `perform_update_error_from_render`. Keep the
  surviving `Perform*Error` display/`From`/debug tests for the remaining variants.

### 4. `storage/src/lib.rs`
Remove the `pub use common::render::RenderError;` line and its comment.

### 5. `web/src/posts/server.rs`
- `perform_update_error`: remove the `PerformUpdateError::Render(_)` arm.
- `perform_creation_error`: remove the `PerformCreationError::Render(e)` arm.
- Remove the `::Render` sub-assertions from `perform_update_error_maps_each_arm`
  and `perform_creation_error_maps_each_arm`.

### 6. `server/src/atompub/posts.rs`
- `creation_status` / `update_status`: remove the `::Render` arms.
- Remove the `RenderError` import and the `::Render` sub-assertions in
  `creation_status_maps_*` / `update_status_maps_*`.

### 7. `server/tests/storage.rs`
- The `create_rendered_post` / `update_rendered_post` integration tests: replace
  `use storage::CreateRenderedPostError;` / `UpdateRenderedPostError;` with
  `CreatePostError` / `UpdatePostError`, and update the `matches!(err, …::Storage(_))`
  assertions to match `CreatePostError`/`UpdatePostError` directly.

### 8. Coverage rebaseline
Re-measure with `scripts/check-coverage`. Expected: `common/src/render.rs` stays
~100% (pure, fully tested); `storage/src/post_service.rs` loses its uncovered
`Render` arm (line ~279) — the only remaining gap should be the fault-injection
`CreatePostError::Internal` arm in `perform_post_creation`. Update
`.coverage-manifest.json` and `.crap-manifest.json` for the affected files
(`common/src/render.rs`, `storage/src/post_service.rs`, and possibly
`web/src/posts/server.rs` / `server/src/atompub/*`) **with user approval**, then
run full `scripts/verify`.

## Risks / notes
- **Alpha instability.** Pinning a pre-release affects build reproducibility; the
  exact `0.10.0-alpha.10` is pinned in `Cargo.lock`.
- **HTML output changes.** 0.10 emits a different structure than 0.9 (e.g.
  `<main>`/`<section>` wrappers). Existing feature tests assert loose content
  presence and should survive, but `org_empty_input` (which strips specific
  structural tags) and possibly the bold/italic/code tag assertions must be
  verified against 0.10's actual output and adjusted if the tag names differ.
- **Link verification gates the work.** If `0.10.0-alpha.10` does not render
  links, the migration stops for reassessment (policy C).

## Out of scope (possible follow-up beads)
- Broader audit of other Org constructs (tables, footnotes, nested lists, inline
  images, TODO keywords).
- Removing `create_rendered_post` / `update_rendered_post` as public API if they
  prove redundant (they remain public for the integration tests).
