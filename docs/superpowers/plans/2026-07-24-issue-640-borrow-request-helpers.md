# Plan — #640: borrow `state` in the test request helpers

**Spec:**
[`docs/superpowers/specs/2026-07-24-issue-640-borrow-request-helpers.md`](../specs/2026-07-24-issue-640-borrow-request-helpers.md)
**For agentic workers:** drive with **`jaunder-iterate`**; delegate the bulk
mechanical call-site sweeps to a subagent via **`jaunder-dispatch`** (keeps the
~290-site diff out of the driver's context). Tick checkboxes in real time.

## Review header

**Goal.** Make every test helper that currently takes `state: Arc<AppState>` by
value borrow it (`&Arc<AppState>`), and make `post_form_with_mailer` take the
mailer by generic reference — so call sites drop `Arc::clone(&state)` and the
`as Arc<dyn MailSender>` cast. Behaviour-preserving; no production change.

**Scope.**

- _In:_ the 24 by-value-`state` helper signatures (9 in
  `server/tests/helpers/mod.rs`, 14 in `server/tests/web/web_posts.rs`, 1 in
  `server/tests/web/web_media.rs`) and all their call sites across
  `server/tests/`; the generic-mailer change (`post_form_with_mailer`).
- _Out:_ `jaunder::create_router` / any production code; #635 seeding work;
  anything other than `state` + the mailer; `get_asset` (owns its own `state`);
  the two `async move` residual clones in `web_posts.rs` (spec D4 — they stay).

**Tasks (one line each).**

1. Convert the 9 shared helpers in `mod.rs` to borrow `state` (+ generic
   mailer); fix all direct call sites suite-wide.
2. Convert the 14 local wrappers in `web_posts.rs` to `&Arc`; fix their callers;
   keep the 2 `async move` residual clones.
3. Convert `web_media.rs::media_serve_get` to `&Arc`; fix its callers.
4. AC gate: run the three `rg` counts (→ 0 / 2 / 0) and `cargo xtask validate`.

**Key risks / decisions.**

- **One compilation unit.** `server/tests/` is a single `integration` binary, so
  a shared-helper signature change breaks every direct caller until all are
  updated in the same commit. Convert bottom-up (callee before caller): each of
  Tasks 1→3 ends in a compiling, passing state. Task 1 unavoidably touches the
  `web_posts.rs`/`web_media.rs` wrapper _bodies_ (their internal `post_*` calls)
  before Task 2/3 touch those wrappers' _signatures_ — a normal two-pass edit,
  called out per task below.
- **Generic mailer, not `&Arc<dyn>`** (spec D2): `Arc<CapturingMailSender>`
  won't coerce through a shared ref, so the helper is generic and coerces
  internally.
- **Purely mechanical.** No assertion/URI/body changes anywhere (spec AC5). If a
  diff hunk changes anything besides `Arc::clone(&state)`→`&state` or
  `mailer.clone() as Arc<…>`→ `&mailer` (or the forced wrapper-body
  `state`↔`&state`), it is wrong.

## Global constraints

- Behaviour-preserving refactor: **no new tests**, no changed assertions. The
  "test" for each task is the existing suite compiling and passing.
- Run from the worktree:
  `/home/mdorman/src/jaunder/.claude/worktrees/issue-640-borrow-request-helpers`.
  Gate via `devtool run -- cargo xtask …` (worktree-aware).
- Before each commit: `cargo xtask check` clean, then commit per
  **`jaunder-commit`**. **No `Co-Authored-By` trailer.**
- Prefer structured edits; delegate bulk call-site edits to a subagent (forbid
  `ctx_*` in its brief — context-mode hangs subagents).

---

## Task 1 — `mod.rs`: borrow `state` in the 9 shared helpers (+ generic mailer)

**Files:** `server/tests/helpers/mod.rs` (signatures + bodies); all
`server/tests/**` files (direct call sites of the shared helpers, incl. wrapper
bodies in `web_posts.rs` / `web_media.rs`).

**Signature changes in `mod.rs`:**

```rust
// post_inner (private): borrow state, keep owned mailer; clone once for the router.
async fn post_inner(
    state: &Arc<storage::AppState>,
    mailer: Arc<dyn MailSender>,
    uri: &str,
    body: PostBody,
    auth: Auth<'_>,
    user_agent: Option<&str>,
    secure_cookies: bool,
) -> (StatusCode, Option<String>, String) {
    // …unchanged until the router build:
    let app = jaunder::create_router(
        test_options(),
        Arc::clone(state),          // the one clone, paid here
        mailer,
        secure_cookies,
        tmp_storage_path(),
    );
    // …unchanged
}

// The six post_* wrappers: state: Arc<…> -> state: &Arc<…>; pass `state` straight to
// post_inner (already a &Arc). Bodies otherwise unchanged. e.g.:
pub async fn post_form(
    state: &Arc<storage::AppState>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let auth = cookie.map_or(Auth::None, Auth::Cookie);
    let (status, _set_cookie, body) =
        post_inner(state, noop_mailer(), uri, PostBody::Form(body.into()), auth, None, true).await;
    (status, body)
}

// post_form_with_mailer: borrow state AND take the mailer by generic ref; coerce once.
pub async fn post_form_with_mailer<M: MailSender + 'static>(
    state: &Arc<storage::AppState>,
    mailer: &Arc<M>,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let auth = cookie.map_or(Auth::None, Auth::Cookie);
    let mailer: Arc<dyn MailSender> = Arc::clone(mailer); // Arc<M> -> Arc<dyn …> coercion
    let (status, _set_cookie, body) =
        post_inner(state, mailer, uri, PostBody::Form(body.into()), auth, None, true).await;
    (status, body)
}

// post_form_with_secure_flag / post_form_with_ua / post_form_with_bearer / post_json:
//   state: Arc<…> -> &Arc<…>; pass `state` to post_inner. Bodies otherwise unchanged.

// make_app: state: Arc<…> -> &Arc<…>; clone once for create_router.
pub fn make_app(state: &Arc<storage::AppState>, storage: &TempDir) -> axum::Router {
    // …dir setup unchanged…
    jaunder::create_router(test_options(), Arc::clone(state), noop_mailer(), false, storage_path)
}

// post_multipart: state: Arc<…> -> &Arc<…>; call make_app(state, storage) (both borrow now).
pub async fn post_multipart(
    state: &Arc<storage::AppState>,
    storage: &TempDir,
    uri: &str,
    file: MultipartFile<'_>,
    cookie: Option<&str>,
) -> (StatusCode, String) { /* body: `let app = make_app(state, storage);` — rest unchanged */ }
```

**Call-site sweep (mechanical, suite-wide):**

- Every direct
  `post_form|post_json|post_form_with_secure_flag|post_form_with_ua| post_form_with_bearer|post_multipart|make_app(Arc::clone(&state), …)`
  → `(&state, …)`.
- Every
  `post_form_with_mailer(Arc::clone(&state), mailer.clone() as Arc<dyn …MailSender>, …)`
  → `(&state, &mailer, …)`.
- Inside the `web_posts.rs` local wrappers (still by-value this task), their
  internal `post_*` calls change `state`→`&state` (they own `state`). Their
  _signatures_ stay by-value until Task 2. (`web_media.rs::media_serve_get` has
  no `post_*`/`make_app` call — it builds `create_router` directly — so Task 1
  does not touch its body; only the direct `post_*`/`make_app` call sites
  elsewhere in `web_media.rs` change here. `media_serve_get`'s signature and
  body are handled wholly in Task 3.)
- The two `async move` closures (`web_posts.rs:2197,2272`): the internal call
  becomes `post_json(&state, …)`; the `let state = Arc::clone(&state);` line
  stays.

**Verify (expect PASS, and it must compile):**
`devtool run -- cargo nextest run -p jaunder --test integration` Then
`cargo xtask check` clean → commit ("test(server): borrow state in shared
request helpers (#640)").

## Task 2 — `web_posts.rs`: borrow `state` in the 14 local wrappers

**Files:** `server/tests/web/web_posts.rs` only.

**Changes:**

- Each local wrapper (`create_post_json`, `update_post_json`, `get_post_form`,
  `get_post_preview_form`, `unpublish_post_form`, `publish_post_form`,
  `list_drafts_form`, `list_user_posts_form`, `list_posts_by_tag_form`,
  `list_user_posts_by_tag_form`, `list_local_timeline_form`,
  `list_home_feed_form`, `delete_post_form`, `unauthenticated_request`):
  `state: Arc<storage::AppState>` → `&Arc<storage::AppState>`.
- Inside each wrapper, the forwarded `post_*(&state, …)` from Task 1 becomes
  `post_*(state, …)` (the param is now already `&Arc`).
- Every caller of these wrappers: `wrapper(Arc::clone(&state), …)` →
  `wrapper(&state, …)`.
- **Leave** the two `let state = Arc::clone(&state);` lines (2197, 2272) exactly
  as-is; inside those futures the call stays `post_json(&state, …)` (borrowing
  the owned clone).

**Verify:** `devtool run -- cargo nextest run -p jaunder --test integration` →
PASS; `cargo xtask check` clean → commit ("test(server): borrow state in
web_posts wrappers (#640)").

## Task 3 — `web_media.rs`: borrow `state` in `media_serve_get`

**Files:** `server/tests/web/web_media.rs` only.

**Changes:**

- `media_serve_get(state: Arc<storage::AppState>, …)` →
  `&Arc<storage::AppState>`; inside,
  `create_router(test_options(), Arc::clone(state), …)`.
- Its callers: `media_serve_get(Arc::clone(&state), …)` →
  `media_serve_get(&state, …)`.

**Verify:** `devtool run -- cargo nextest run -p jaunder --test integration` →
PASS; `cargo xtask check` clean → commit ("test(server): borrow state in
media_serve_get (#640)").

## Task 4 — Acceptance gate

**No file changes.** Confirm the spec's ACs:

- **AC1:** `rg 'state: Arc<storage::AppState>' server/tests/` → **no matches**
  (exit 1).
- **AC3:** `rg 'Arc::clone\(&state\)' server/tests/` → **exactly 2 lines**, both
  `let state = Arc::clone(&state);` in `web_posts.rs`.
- **AC4:** `rg 'as Arc<dyn.*MailSender>' server/tests/` → **no matches** (exit
  1).
- **AC6:** `devtool run -- cargo xtask validate` → green (run foreground,
  `timeout:600000`; it runs the 4 e2e combos + elisp — see the
  parallel-VM-contention note if firefox combos flake).

If all four hold, the branch is ready for **`jaunder-ship`**.
