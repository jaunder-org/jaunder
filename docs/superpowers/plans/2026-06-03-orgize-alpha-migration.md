# orgize 0.10 Migration + Infallible Rendering — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move post rendering to `orgize 0.10.0-alpha.10` (which renders Org links as anchors) and, because its HTML export is infallible, remove the entire `RenderError` error path end-to-end.

**Architecture:** `orgize 0.10` exposes `Org::parse(body).to_html() -> String` (infallible), replacing 0.9's fallible `write_html`. So `render()`/`render_org()` return `String`, `RenderError` and the single-variant wrapper enums `CreateRenderedPostError`/`UpdateRenderedPostError` are deleted, and the `Render` arms in `PerformUpdateError`/`PerformCreationError` and their consumers (web, server) go away. The change is type-coupled, so the code change lands as **one atomic commit** (Task 1); coverage rebaseline + full verify is Task 2.

**Tech Stack:** Rust workspace (`common`, `storage`, `web`, `server`); `orgize`; `cargo nextest`; `scripts/check-coverage`; `scripts/verify`.

**Reference spec:** `docs/superpowers/specs/2026-06-03-orgize-alpha-migration-design.md`.

---

## Why one atomic task

Changing `render()` from `Result<String, RenderError>` to `String` breaks every `render(...)?` call site simultaneously, and deleting `RenderError` breaks `storage`, `web`, and `server` at once. A partial commit would not compile. So Task 1 makes all the code edits and is verified/committed as a unit. The **link-rendering gate** (Step 5) runs before the cross-crate edits so we bail early if the alpha can't render links.

---

### Task 1: Migrate to orgize 0.10 and make rendering infallible

**Files:**
- Modify: `Cargo.toml` (workspace dep)
- Modify: `common/src/render.rs`
- Modify: `storage/src/lib.rs`
- Modify: `storage/src/post_service.rs`
- Modify: `web/src/posts/server.rs`
- Modify: `server/src/atompub/posts.rs`
- Modify: `server/tests/storage.rs`

- [ ] **Step 1: Bump the orgize dependency**

In `Cargo.toml` (workspace root), change:
```toml
orgize = "0.9"
```
to:
```toml
orgize = "0.10.0-alpha.10"
```

- [ ] **Step 2: Migrate `render_org` to the 0.10 API (infallible)**

In `common/src/render.rs`, replace the `render_org` function:
```rust
/// Renders Org-mode to HTML using orgize.
fn render_org(body: &str) -> Result<String, RenderError> {
    let org = orgize::Org::parse(body);
    let mut buf = Vec::new();
    org.write_html(&mut buf)
        .map_err(|e| RenderError::OrgRender(e.to_string()))?;
    String::from_utf8(buf).map_err(|e| RenderError::OrgRender(e.to_string()))
}
```
with:
```rust
/// Renders Org-mode to HTML using orgize.
fn render_org(body: &str) -> String {
    orgize::Org::parse(body).to_html()
}
```

- [ ] **Step 3: Make `render` infallible**

In `common/src/render.rs`, replace the `render` function and its doc comment. Current:
```rust
/// Renders `body` to HTML based on `format`. Pure function.
///
/// # Errors
///
/// Returns `Err(RenderError)` if the body cannot be rendered for the given format.
pub fn render(body: &str, format: &PostFormat) -> Result<String, RenderError> {
    match format {
        PostFormat::Markdown => Ok(render_markdown(body)),
        PostFormat::Org => render_org(body),
        PostFormat::Html => Ok(body.to_string()),
    }
}
```
New:
```rust
/// Renders `body` to HTML based on `format`. Pure, infallible function.
#[must_use]
pub fn render(body: &str, format: &PostFormat) -> String {
    match format {
        PostFormat::Markdown => render_markdown(body),
        PostFormat::Org => render_org(body),
        PostFormat::Html => body.to_string(),
    }
}
```
(The `# Errors` doc section is removed because the function no longer returns `Result`; `#[must_use]` is added per the codebase's convention for pure value-returning functions.)

- [ ] **Step 4: Delete the `RenderError` type**

In `common/src/render.rs`, delete the `RenderError` enum and its section comment:
```rust
// ---------------------------------------------------------------------------
// Render errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("org-mode render error: {0}")]
    OrgRender(String),
}
```
Leave `use thiserror::Error;` in place — it is still used by `InvalidPostFormat`.

- [ ] **Step 5: Update the `common` render tests and run the LINK-RENDERING GATE**

In `common/src/render.rs` tests:
- Delete `render_error_display` and `render_error_debug`.
- In `render_dispatches_markdown`, `render_dispatches_org`, and `render_html_format_is_identity`, drop `.unwrap()` (now returns `String`). E.g.:
  ```rust
  #[test]
  fn render_html_format_is_identity() {
      let body = "<p>hi <b>there</b></p>";
      assert_eq!(render(body, &PostFormat::Html), body.to_string());
  }
  ```
- In the seven `org_*` tests (`org_headings`, `org_paragraph`, `org_bold_italic_code`, `org_list`, `org_code_block`, `org_link`, `org_empty_input`), drop `.unwrap()` from `render_org(...)` calls (now returns `String`).
- Strengthen `org_link`:
  ```rust
  #[test]
  fn org_link() {
      let html = render_org("[[https://example.com][example]]");
      assert!(
          html.contains("<a href=\"https://example.com\""),
          "expected an anchor element, got: {html}"
      );
      assert!(html.contains("example"));
  }
  ```

Then run the gate:
Run: `cargo test -p common --lib render::tests::org_link -- --nocapture`
Expected: PASS — the rendered HTML contains `<a href="https://example.com"`.

**GATE:** If `org_link` FAILS (no anchor in output), STOP. Do not proceed to the cross-crate edits. Report back with the actual `to_html()` output for `[[https://example.com][example]]` so we can reassess (spec policy "C": published-only; fork is a separate decision).

- [ ] **Step 6: Verify/adjust the other `common` org tests against real 0.10 output**

Run: `cargo test -p common --lib render::tests`
Expected: PASS. If `org_empty_input`, `org_bold_italic_code`, or `org_code_block` fail because 0.10's tag names/wrappers differ from 0.9, adjust **only the assertions** to match 0.10's actual output (do not change `render_org`). For `org_empty_input`, ensure the structural-tag stripping covers 0.10's empty-document wrapper (e.g. `<main></main>`); print the output with `-- --nocapture` if needed to see the exact shape. Re-run until green.

- [ ] **Step 7: Remove the `RenderError` re-export from storage**

In `storage/src/lib.rs`, delete these lines:
```rust
// `RenderError` lives in `common::render` but is part of storage's public
// surface: `web` and `server` match on it via the `Perform*Error::Render`
// arms, so re-export it here to keep the `storage::RenderError` path stable.
pub use common::render::RenderError;
```

- [ ] **Step 8: Collapse the wrapper error enums in `post_service`**

In `storage/src/post_service.rs`:

(a) Update the import:
```rust
use common::render::{derive_post_metadata, render, RenderError};
```
→
```rust
use common::render::{derive_post_metadata, render};
```

(b) Delete both wrapper enums entirely:
```rust
#[derive(Debug, Error)]
pub enum CreateRenderedPostError {
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Storage(#[from] CreatePostError),
}

#[derive(Debug, Error)]
pub enum UpdateRenderedPostError {
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Storage(#[from] UpdatePostError),
}
```

(c) In `PerformUpdateError`, delete the line:
```rust
    #[error(transparent)]
    Render(#[from] RenderError),
```
(d) In `PerformCreationError`, delete the line:
```rust
    #[error(transparent)]
    Render(#[from] RenderError),
```

- [ ] **Step 9: Update `create_rendered_post` / `update_rendered_post` signatures and bodies**

In `storage/src/post_service.rs`, `create_rendered_post`: change the return type from `Result<i64, CreateRenderedPostError>` to `Result<i64, CreatePostError>`, update its `# Errors` doc to name `CreatePostError`, and change the render line:
```rust
    let rendered_html = render(&body, &format)?;
```
to:
```rust
    let rendered_html = render(&body, &format);
```
(The final `Ok(storage.create_post(&input).await?)` is unchanged — `create_post` already returns `Result<_, CreatePostError>`.)

`update_rendered_post`: change the return type from `Result<PostRecord, UpdateRenderedPostError>` to `Result<PostRecord, UpdatePostError>`, update its `# Errors` doc to name `UpdatePostError`, and change `let rendered_html = render(&body, &format)?;` to `let rendered_html = render(&body, &format);`.

- [ ] **Step 10: De-`?` the remaining render call and fix the creation retry match**

In `storage/src/post_service.rs`, `perform_post_update`: change `let rendered_html = render(&body, &format)?;` to `let rendered_html = render(&body, &format);`.

In `perform_post_creation`, replace the match arms that reference `CreateRenderedPostError`:
```rust
            Err(CreateRenderedPostError::Storage(CreatePostError::SlugConflict)) => {}
            Err(CreateRenderedPostError::Storage(CreatePostError::Internal(e))) => {
                return Err(PerformCreationError::Storage(e));
            }
            Err(CreateRenderedPostError::Render(e)) => {
                return Err(PerformCreationError::Render(e));
            }
```
with:
```rust
            Err(CreatePostError::SlugConflict) => {}
            Err(CreatePostError::Internal(e)) => {
                return Err(PerformCreationError::Storage(e));
            }
```
(`CreatePostError` is already imported via the `use crate::{ CreatePostError, ... }` block. `CreatePostError` has exactly `SlugConflict` and `Internal`, so this match is exhaustive.)

- [ ] **Step 11: Remove the dead error tests in `post_service`**

In `storage/src/post_service.rs` tests, delete these (they reference deleted types/variants): `create_rendered_post_error_from_render`, `update_rendered_post_error_from_render`, `create_rendered_post_error_debug`, `update_rendered_post_error_debug`, `create_rendered_post_error_from_storage_display`, `create_rendered_post_error_from_storage_debug`, `update_rendered_post_error_from_storage_display`, `update_rendered_post_error_from_storage_debug`, and `perform_update_error_from_render`. Keep all other `perform_update_error_*` / `perform_creation_error` / `test_perform_post_creation_*` tests.

- [ ] **Step 12: Remove the `Render` arms in the web mappers + tests**

In `web/src/posts/server.rs`, `perform_update_error`: delete the arm
```rust
        PerformUpdateError::Render(_) => InternalError::server(error),
```
`perform_creation_error`: delete the arm
```rust
        PerformCreationError::Render(e) => InternalError::validation(e.to_string()),
```
In the tests, delete the two `Render` assertion blocks:
```rust
        assert!(matches!(
            perform_update_error(PerformUpdateError::Render(storage::RenderError::OrgRender(
                "bad".to_string()
            )))
            .public(),
            WebError::Server { .. }
        ));
```
and
```rust
        assert!(matches!(
            perform_creation_error(PerformCreationError::Render(
                storage::RenderError::OrgRender("bad".to_string())
            ))
            .public(),
            WebError::Validation { .. }
        ));
```

- [ ] **Step 13: Fix the server AtomPub tests (handler code is unchanged)**

In `server/src/atompub/posts.rs`, the `creation_status`/`update_status` functions use a `_ =>` catch-all and need NO change. In their test module:
- Change the import `use storage::{PerformCreationError, PerformUpdateError, RenderError};` to `use storage::{PerformCreationError, PerformUpdateError};`.
- In `creation_status_maps_validation_to_400_else_500`, delete the assertion block:
  ```rust
          assert_eq!(
              creation_status(&PerformCreationError::Render(RenderError::OrgRender(
                  "e".to_string()
              ))),
              StatusCode::INTERNAL_SERVER_ERROR
          );
  ```
- In `update_status_maps_each_error_class`, delete the assertion block:
  ```rust
          assert_eq!(
              update_status(&PerformUpdateError::Render(RenderError::OrgRender(
                  "e".to_string()
              ))),
              StatusCode::INTERNAL_SERVER_ERROR
          );
  ```

- [ ] **Step 14: Fix the `server/tests/storage.rs` integration tests**

In `server/tests/storage.rs`:
- In `assert_create_rendered_post_slug_conflict`: change `use storage::CreateRenderedPostError;` to `use storage::CreatePostError;`, and change
  ```rust
      matches!(err, CreateRenderedPostError::Storage(_)),
  ```
  to
  ```rust
      matches!(err, CreatePostError::SlugConflict),
  ```
- In `assert_update_rendered_post_not_found`: change `use storage::UpdateRenderedPostError;` to `use storage::UpdatePostError;`, and change
  ```rust
      matches!(err, UpdateRenderedPostError::Storage(_)),
  ```
  to
  ```rust
      matches!(err, UpdatePostError::NotFound),
  ```
(The `.to_string().contains("slug")` / `contains("not found")` assertions still hold: `CreatePostError::SlugConflict` displays "slug already taken…"; `UpdatePostError::NotFound` displays "post not found".)

- [ ] **Step 15: Build, format, lint, and test the whole workspace**

Run: `cargo fmt`
Run: `cargo build`
Expected: clean.
Run: `cargo clippy -- -D warnings`
Expected: clean (no warnings). If `render` trips `clippy::unnecessary_wraps`, that would only happen if it still returned `Result` — confirm it returns `String`.
Run: `cargo nextest run`
Expected: PASS (count is the prior total minus the ~13 deleted tests; all green).

- [ ] **Step 16: Commit**

```bash
git add Cargo.toml Cargo.lock common/src/render.rs storage/src/lib.rs storage/src/post_service.rs web/src/posts/server.rs server/src/atompub/posts.rs server/tests/storage.rs
git commit -m "feat(render): migrate to orgize 0.10, make rendering infallible

orgize 0.10's to_html() is infallible and renders org links as anchors.
Drop RenderError, collapse the CreateRenderedPostError/UpdateRenderedPostError
wrappers, and remove the now-dead Render arms across storage/web/server.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Coverage rebaseline + full verification

**Files:**
- Modify: `.coverage-manifest.json`, `.crap-manifest.json`

- [ ] **Step 1: Measure coverage**

Run: `scripts/check-coverage`
Expected: it reports changes for the affected files. Note the new measured percentages for `common/src/render.rs` and `storage/src/post_service.rs` (and any web/server files whose covered-line counts shifted from the removed arms).

- [ ] **Step 2: Investigate any drop**

Run: `scripts/check-coverage --investigate`
Expected: `common/src/render.rs` ~100% (pure, fully tested). `storage/src/post_service.rs` should be at/above its prior baseline — the previously-uncovered `Render` arm is gone; the remaining gap should only be the fault-injection `CreatePostError::Internal` arm in `perform_post_creation`. Confirm no *new* uncovered logic was introduced.

- [ ] **Step 3: Update the manifests (REQUIRES USER APPROVAL)**

Present the proposed `.coverage-manifest.json` deltas to the user and get approval before editing. Then apply the approved values and let `scripts/check-coverage` re-key `.crap-manifest.json`. Confirm the CRAP diff is only path/score updates for the changed functions.

- [ ] **Step 4: Confirm the gate is green**

Run: `scripts/check-coverage`
Expected: `Coverage and CRAP OK`.

- [ ] **Step 5: Full verification**

Run: `scripts/verify`
Expected: fmt, build, tests, lint, coverage, and the nix checks pass. (The qemu-VM e2e is timing-sensitive in constrained sandboxes; a green run on the user's machine is authoritative.)

- [ ] **Step 6: Commit and close the bead**

```bash
git add .coverage-manifest.json .crap-manifest.json
git commit -m "test(coverage): rebaseline after orgize 0.10 migration

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
bd close jaunder-c8wv
```

---

## Self-Review

**Spec coverage:**
- Dep bump → Task 1 Step 1. ✓
- `render_org`/`render` infallible → Steps 2–3. ✓
- Delete `RenderError` → Step 4; re-export → Step 7. ✓
- Strengthen `org_link` + link gate → Step 5; adjust other org tests → Step 6. ✓
- Collapse `CreateRenderedPostError`/`UpdateRenderedPostError`, drop `Render` from `Perform*Error` → Steps 8–11. ✓
- web mappers + tests → Step 12; server tests (handlers unchanged) → Step 13. ✓
- `server/tests/storage.rs` → Step 14. ✓
- Build/clippy/test → Step 15; coverage rebaseline + verify + close bead → Task 2. ✓

**Placeholder scan:** No TBD/TODO. The only deferred specifics are the measured coverage numbers (Task 2, unknowable until impl) and the possible `org_empty_input` tag-stripping tweak (Step 6, gated on real 0.10 output) — both legitimately determined at implementation time, with explicit instructions for how to resolve them.

**Type consistency:** `render(... ) -> String`, `render_org(...) -> String`, `create_rendered_post -> Result<i64, CreatePostError>`, `update_rendered_post -> Result<PostRecord, UpdatePostError>`, `PerformCreationError`/`PerformUpdateError` keep all non-`Render` variants. `CreatePostError = {SlugConflict, Internal}` and `UpdatePostError = {NotFound, Unauthorized, Internal}` (verified) make the rewritten matches exhaustive. Names consistent across tasks.

## Risk notes
- **Link gate (Step 5) is load-bearing** — everything after assumes the alpha renders anchors.
- 0.10 HTML output differs structurally from 0.9; Step 6 adjusts assertions, not logic.
- Pinning a pre-release affects build reproducibility (pinned in `Cargo.lock`).
