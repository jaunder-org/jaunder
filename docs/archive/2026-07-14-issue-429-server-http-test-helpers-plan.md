# Plan — consolidate duplicated server HTTP test helpers (#429)

Spec:
[2026-07-14-issue-429-server-http-test-helpers.md](../specs/2026-07-14-issue-429-server-http-test-helpers.md).
This plan is "how"; see the spec for "what/why".

## Review header

**Goal.** Lift the four duplicated HTTP request/response test helpers into the
single shared `server/tests/helpers/mod.rs` and convert every call site — pure
dedup, no suppression added or removed, no test-assertion behavior changed.

**Scope.**

- _In:_ `server/tests/helpers/mod.rs` (add `make_app`, `body_string`,
  `post_json`, `get_asset`; generalize the private `post_form_inner`); the 10
  test files that hold the local copies/call sites under
  `server/tests/{atompub,feed,misc,web}/`.
- _Out:_ `post_form*` public signatures; any test assertion; any
  `#[allow]`/`#[expect]`; `server/tests/{projector,storage}/`; production code.

**Tasks (one line each).**

1. Lift `body_string` (×4, identical) → `helpers::body_string`; convert atompub
   call sites.
2. Lift `make_app` (×6, unified to always create the media dirs) →
   `helpers::make_app`; convert call sites.
3. Generalize `post_form_inner` → `post_inner(content_type)`; add
   `helpers::post_json` (`serde_json::Value` body); convert
   feed/web_posts/web_tags call sites.
4. Move `get_asset` → `helpers::get_asset` (add `Backend`/`TestEnv` imports to
   helpers); convert `misc/static_assets` call sites.

**Key risks / decisions.**

- Each task adds a helper **and** converts all its call sites in the same
  commit, so the helper is never an unused `pub fn` (which would trip
  `dead_code` under `-D warnings` — the integration binary carries no
  `#[allow(dead_code)]`; every helper item must be reachable).
- **Import pruning is mandatory and compiler-guided.** Removing a local helper
  leaves its imports (`Body`, `Request`, `header`, `ServiceExt`, `StatusCode`,
  `TempDir`, `ensure_server_fns_registered`, `test_options`, `noop_mailer`, …)
  unused; prune each touched file to exactly what it still uses, or
  `unused_imports` fails the gate.
- `make_app` unification (always creating media dirs) is behaviorally safe —
  empty dirs under a per-test `TempDir`, never asserted against (spec Decision 1
  / AC4).
- No separable concerns surfaced during design → no issue-filing task.

**For agentic workers.** Execute with **`jaunder-iterate`**, delegating a task
to a subagent via **`jaunder-dispatch`** when useful. Tick checkboxes in real
time.

## Global constraints

- Review anchor: three-dot `git diff main...HEAD` (fork tag
  `wt-base-issue-429`).
- Single integration binary: `server/Cargo.toml`
  `[[test]] name = "integration"`, `path = "tests/main.rs"`, with
  `mod helpers;`. New `pub` items in `helpers/mod.rs` are reachable everywhere
  as `crate::helpers::…`.
- **No `Co-Authored-By` trailer** on commits (user global preference).
- Per-task verification:
  - Fast compile/lint: `cargo clippy -p server --tests` (must be warning-clean).
  - Behavior: `cargo nextest run -p server --test integration` (needs the
    dual-backend harness / PostgreSQL; if PG is unavailable locally, rely on the
    commit-time gate).
  - Commit gate: the pre-commit hook runs full `cargo xtask check` — run
    `cargo xtask check` first so it passes clean (**`jaunder-commit`**).
- Final: `cargo xtask validate` green (AC6).

---

## Task 1 — `body_string` → `helpers::body_string`

**Spec:** Decision 4, AC1/AC3. The 4 copies are byte-identical.

**Files.**

- `server/tests/helpers/mod.rs` — add:
  ```rust
  /// Read a response body fully and decode it as UTF-8.
  pub async fn body_string(response: axum::response::Response) -> String {
      let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
          .await
          .unwrap();
      String::from_utf8(bytes.to_vec()).unwrap()
  }
  ```
- `server/tests/atompub/atompub_posts.rs`, `atompub_rsd.rs`, `atompub_media.rs`,
  `atompub_service.rs` — delete the local `async fn body_string`; add
  `body_string` to the existing `use crate::helpers::{…}` import. Prune any
  import left unused **only by this removal** (do not remove imports still used
  by the file's other local helpers, e.g. `make_app` still present until Task
  2).

**Verify.**

- `cargo clippy -p server --tests` → PASS, warning-clean.
- `cargo nextest run -p server --test integration atompub` → PASS (unchanged).

**Commit** (after `cargo xtask check` clean):
`test-infra(server): lift body_string into helpers (#429)`.

---

## Task 2 — `make_app` → `helpers::make_app` (unified)

**Spec:** Decision 1, AC1/AC3/AC4. Six copies, two variants; unify to always
create the media dirs.

**Files.**

- `server/tests/helpers/mod.rs` — add:
  ```rust
  use tempfile::TempDir;
  // …
  /// Build a fresh router from `state` over `storage` as the media root, with the noop
  /// mailer and insecure cookies. Always creates the `media/{upload,cached,tmp}` layout so
  /// upload-exercising and read-only tests share one helper (the dirs are harmless empty
  /// setup for tests that never upload).
  pub fn make_app(state: Arc<storage::AppState>, storage: &TempDir) -> axum::Router {
      ensure_server_fns_registered();
      let storage_path = storage.path().to_path_buf();
      std::fs::create_dir_all(storage_path.join("media").join("upload")).unwrap();
      std::fs::create_dir_all(storage_path.join("media").join("cached")).unwrap();
      std::fs::create_dir_all(storage_path.join("media").join("tmp")).unwrap();
      jaunder::create_router(test_options(), state, noop_mailer(), false, storage_path)
  }
  ```
  (Add `use tempfile::TempDir;` if not already present; `Arc`,
  `ensure_server_fns_registered`, `test_options`, `noop_mailer` are already in
  scope.)
- `server/tests/feed/feed_handlers.rs`, `misc/media_handlers.rs`,
  `atompub/atompub_media.rs`, `atompub/atompub_posts.rs`,
  `atompub/atompub_rsd.rs`, `atompub/atompub_service.rs` — delete the local
  `fn make_app`; add `make_app` to the `use crate::helpers::{…}` import.
  **Prune** now-unused imports per file: after removing `make_app` (and, in
  atompub files, `body_string` already gone in Task 1), typical casualties are
  `tempfile::TempDir`, `ensure_server_fns_registered`, `test_options`, and —
  where the file has no other local HTTP helper — `noop_mailer`. Let
  `cargo clippy` name them; remove exactly those.

**Verify.**

- `cargo clippy -p server --tests` → PASS, warning-clean.
- `cargo nextest run -p server --test integration` (feed, misc, atompub subsets)
  → PASS.

**Commit:** `test-infra(server): unify make_app into helpers (#429)`.

---

## Task 3 — `post_json` → `helpers::post_json` over generalized `post_inner`

**Spec:** Decision 2, AC1/AC2/AC3. Fold the JSON POST onto the shared request
machinery.

**Files — `server/tests/helpers/mod.rs`.**

1. Rename `post_form_inner` → `post_inner` and add a `content_type: &str`
   parameter; set the `CONTENT_TYPE` header from it:
   ```rust
   async fn post_inner(
       state: Arc<storage::AppState>,
       mailer: Arc<dyn MailSender>,
       uri: &str,
       body: impl Into<String>,
       content_type: &str,
       auth: Auth<'_>,
       user_agent: Option<&str>,
       secure_cookies: bool,
   ) -> (StatusCode, Option<String>, String) {
       ensure_server_fns_registered();
       let mut builder = Request::builder()
           .method("POST")
           .uri(uri)
           .header(header::CONTENT_TYPE, content_type);
       // … body unchanged …
   }
   ```
2. Update every `post_form*` wrapper to call
   `post_inner(…, "application/x-www-form-urlencoded", …)` (signatures of the
   public wrappers are unchanged — AC "out of scope").
3. Add the JSON wrapper:
   ```rust
   /// POST a JSON body (`Content-Type: application/json`) with secure cookies, optional
   /// cookie auth; returns `(status, body)` (drops `Set-Cookie`, like [`post_form`]).
   pub async fn post_json(
       state: Arc<storage::AppState>,
       uri: &str,
       body: serde_json::Value,
       cookie: Option<&str>,
   ) -> (StatusCode, String) {
       let auth = cookie.map_or(Auth::None, Auth::Cookie);
       let (status, _set_cookie, body) = post_inner(
           state,
           noop_mailer(),
           uri,
           body.to_string(),
           "application/json",
           auth,
           None,
           true,
       )
       .await;
       (status, body)
   }
   ```
   (`serde_json` is a regular server dependency, available to the integration
   tests.)

**Files — call sites.**

- `server/tests/feed/feed_events_hook.rs` — delete local `async fn post_json`;
  import `post_json` from `crate::helpers`; at each call site pass the `Value`
  directly (drop the `.to_string()`, e.g. `body` instead of `body.to_string()`).
  Prune unused imports (`Body`, `Request`, `header`, `ServiceExt`,
  `test_options`, `noop_mailer`, `ensure_server_fns_registered` if no longer
  used directly).
- `server/tests/web/web_posts.rs` — delete local `async fn post_json`; import
  from `crate::helpers`. Call sites (incl. the
  `create_post_json`/`update_post_json` wrappers) already pass
  `(Value, Option<&str>)` → unchanged. Prune unused imports.
- `server/tests/web/web_tags.rs` — delete local `async fn post_json` (no-cookie
  variant); import from `crate::helpers`; add `None` as the cookie arg at each
  call site (`post_json(state, uri, serde_json::json!({…}), None)`). Prune
  unused imports.

**Verify.**

- `cargo clippy -p server --tests` → PASS, warning-clean.
- `cargo nextest run -p server --test integration` (feed, web subsets) → PASS.

**Commit:**
`test-infra(server): fold post_json into the post_form* family (#429)`.

---

## Task 4 — `get_asset` → `helpers::get_asset`

**Spec:** Decision 3, AC1/AC3. Single-use, moved for consistency.

**Files.**

- `server/tests/helpers/mod.rs` — add the `Backend`/`TestEnv` imports it needs
  (`use storage::test_support::{Backend, TestEnv};` — merge with the existing
  `noop_mailer` import), and the fn verbatim (de-qualifying the one qualified
  call — `crate::helpers::tmp_storage_path()` → `tmp_storage_path()` — since it
  now lives in the module; `test_options()` is already a bare call in the
  source):
  ```rust
  /// GET a static asset and return `(status, Content-Type)`. Pins the Sqlite backend —
  /// static-asset serving never touches storage, so it need not run on both.
  pub async fn get_asset(uri: &str) -> (StatusCode, Option<String>) {
      let TestEnv { state, base: _base } = Backend::Sqlite.setup().await;
      let request = Request::builder()
          .method("GET")
          .uri(uri)
          .body(Body::empty())
          .unwrap();
      let app =
          jaunder::create_router(test_options(), state, noop_mailer(), false, tmp_storage_path());
      let response = app.oneshot(request).await.unwrap();
      let status = response.status();
      let content_type = response
          .headers()
          .get(header::CONTENT_TYPE)
          .map(|v| v.to_str().unwrap().to_string());
      (status, content_type)
  }
  ```
  (`Body`, `Request`, `header`, `StatusCode`, `ServiceExt`, `test_options`,
  `noop_mailer`, `tmp_storage_path` are already in scope from the existing
  `post_*` helpers.)
- `server/tests/misc/static_assets.rs` — delete the local `async fn get_asset`;
  import `get_asset` from `crate::helpers`; prune the now-unused imports
  (`Body`, `Request`, `StatusCode`, `ServiceExt`, `test_options`, `Backend`,
  `TestEnv`, `noop_mailer` — whatever the tests themselves no longer reference).

**Verify.**

- `cargo clippy -p server --tests` → PASS, warning-clean.
- `cargo nextest run -p server --test integration static` → PASS.

**Commit:** `test-infra(server): move get_asset into helpers (#429)`.

---

## Final verification (AC5/AC6)

- `git diff main...HEAD` — confirm **no** `#[allow]`/`#[expect]` added or
  removed anywhere, and no test-assertion changes (only helper defs + imports +
  call args).
- `rg -n 'fn (make_app|body_string|post_json|get_asset)\(' server/tests/{atompub,feed,misc,web}`
  → no matches (each helper defined exactly once, in `helpers/mod.rs`).
- `cargo xtask validate` → green.
