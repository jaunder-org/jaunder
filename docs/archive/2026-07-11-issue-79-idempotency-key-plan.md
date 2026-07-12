# Plan — atompub: idempotency key for post create (#79)

**Spec:**
[`2026-07-11-issue-79-idempotency-key-spec.md`](./2026-07-11-issue-79-idempotency-key-spec.md).
Behavior, dedup model, and AC-S1–6 / AC-C1–6 live there; this plan is **how**.

## Review header

**Goal.** Dedup post creates on a client `Idempotency-Key` (atomic, in the
create transaction), and grow the emacs client an ephemeral per-call key +
auto-retry so the key is actually resent on a lost-response retry.

**Scope — in:** `storage/` (migration 0023, create-path threading, dedup +
lookup), `server/src/atompub/` (`PostServices` extractor, `collection_post`,
`member_put`), `elisp/jaunder-publish.el` + `elisp/test/jaunder-test.el`.
**Out:** buffer-persisted / content-bound keys; PUT retry/key; 409-on-reuse; key
GC (follow-on, Task 0).

**Tasks (one commit each unless noted):**

0. **File the GC/TTL follow-on issue** (`jaunder-issues`) — no commit.
1. **`PostServices` extractor** — bundle the four post-handler storage
   Extensions; apply to `collection_post` + `member_put`, dropping
   `member_put`'s `#[expect(too_many_arguments)]`. Pure refactor, no behavior
   change.
2. **Storage dedup** — migration 0023 (both dialects); thread `idempotency_key`
   through the create chain; register it in the post transaction;
   `IdempotencyConflict`; `post_id_for_idempotency_key`. Storage tests.
3. **Handler dedup** — `collection_post` reads `Idempotency-Key`, passes it
   through, and on `IdempotencyConflict` returns the original post as `200`
   (skipping `apply_categories`). Handler integration tests.
4. **Client** — ephemeral key + auto-retry on the create POST. ERT tests.

**Key risks / decisions:**

- **Type split:** `PostCreation<'a>` takes `Option<&'a str>`; `CreatePostInput`
  / `RenderedPostContent` are owned → `Option<String>`. The chain is
  `PostCreation` → `RenderedPostContent` → `render_post_input` →
  `CreatePostInput` → `write_post_in_tx`; every other builder
  (`seed_post_input`, all tests) sets the field `None`.
- **Violation attribution is by statement, not constraint name:** the
  `idempotency_keys` INSERT has its own
  `.map_err(is_unique_violation → IdempotencyConflict)`, separate from the
  post-insert's `→ SlugConflict`. Verified sound in review.
- **Dedup 200 skips `apply_categories`** (original already tagged). Only **one**
  `From` impl needs a new arm: `post_service.rs`'s
  `From<PerformCreationError> for InternalError` is an explicit no-wildcard
  match. The `atompub/mod.rs` `From … for HandlerError` has a `_` wildcard that
  already absorbs `IdempotencyConflict` — **leave it untouched** (adding an
  explicit arm there creates a fresh uncovered region → coverage-gate fail). The
  new `InternalError` arm is unreachable (handler intercepts the conflict), so
  it needs a trivial covering unit test (see Task 2 tests).
- **No lint suppression** — the arg-count problem is solved by `PostServices`,
  not `#[expect]`.
- **Client key is self-contained** (`md5` of `random`/`float-time`/`emacs-pid`)
  — `org-id` is not required by the client.

## Global constraints

- **Rust:** complete code; `CONTRIBUTING.md` (backend parity, dialect files,
  coverage). No `#[allow]`/`#[expect]` without approval (this cycle adds none —
  it removes one).
- **Per-commit gate:** `cargo xtask check` (git-enforced). Storage/handler tests
  dual-backend via `#[apply(backends)]`; Postgres cases need the Nix gate (local
  `cargo nextest` runs SQLite; Postgres via the gate). Elisp tests run under the
  `ert` step; iterate with `emacs --batch -Q -l elisp/scripts/run-tests.el`.
- **No placeholders.**

---

## Task 0 — file GC follow-on (no commit)

Via `jaunder-issues`: "atompub: GC/TTL for idempotency keys" — the
`idempotency_keys` table grows unbounded; add a prune (lazy delete-on-read of
stale rows, or a swept job). Milestone #4, references #79. Capture up front so
it isn't blocked behind this cycle.

---

## Task 1 — `PostServices` extractor (Commit 1, pure refactor)

### Files / interfaces

**`server/src/atompub/mod.rs`** (or a small `extract.rs`) — a bundling extractor
(this is the first `FromRequestParts` impl in `server/src`; add
`use axum::{extract::FromRequestParts, http::request::Parts, Extension};` and
`use axum::extract::rejection::ExtensionRejection;`). The four Extensions are
layered by the parent app router (today's handlers already resolve them
identically):

```rust
pub(crate) struct PostServices {
    pub posts: Arc<dyn PostStorage>,
    pub subscriptions: Arc<dyn SubscriptionStorage>,
    pub user_config: Arc<dyn UserConfigStorage>,
    pub site_config: Arc<dyn SiteConfigStorage>,
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for PostServices {
    type Rejection = axum::extract::rejection::ExtensionRejection;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self {
            posts: Extension::<Arc<dyn PostStorage>>::from_request_parts(parts, state).await?.0,
            subscriptions: Extension::<Arc<dyn SubscriptionStorage>>::from_request_parts(parts, state).await?.0,
            user_config: Extension::<Arc<dyn UserConfigStorage>>::from_request_parts(parts, state).await?.0,
            site_config: Extension::<Arc<dyn SiteConfigStorage>>::from_request_parts(parts, state).await?.0,
        })
    }
}
```

**`server/src/atompub/posts.rs`** — change `collection_post` and `member_put` to
take `services: PostServices` in place of the four `Extension(...)` args
(reference `services.posts.as_ref()` etc.). Remove `member_put`'s
`#[expect(clippy::too_many_arguments, …)]`. No behavior change.

### Test

No new tests — the existing `atompub_posts.rs` suite exercises both handlers
(create, update, If-Match). Run: `cargo nextest run -p jaunder atompub` (SQLite)
green; the extractor pulls the same Extensions the router already layers, so
behavior is identical.

### Commit

`cargo xtask check` clean →
`atompub: bundle post-handler storage extractors into PostServices (#79)`.

---

## Task 2 — storage dedup (Commit 2)

### Files / interfaces

**Migration
`storage/migrations/{sqlite,postgres}/0023_create_idempotency_keys.sql`** —
mirror `0007_*` (password_resets) per-dialect shape:

- sqlite: `idempotency_key_id INTEGER PRIMARY KEY AUTOINCREMENT`,
  `user_id INTEGER NOT NULL REFERENCES users(user_id)`, `key TEXT NOT NULL`,
  `post_id INTEGER NOT NULL REFERENCES posts(post_id)`,
  `created_at TEXT NOT NULL DEFAULT (…)`, `UNIQUE(user_id, key)`.
- postgres: the `BIGINT`/`TIMESTAMPTZ`/`GENERATED … IDENTITY` equivalents (copy
  0007's postgres idioms exactly).

**`storage/src/posts.rs`:**

- `CreatePostInput` gains `idempotency_key: Option<String>`; `CreatePostError`
  gains `IdempotencyConflict`.
- `write_post_in_tx`: after the post `INSERT … RETURNING post_id`, if
  `input.idempotency_key` is `Some(k)`, run
  `INSERT INTO idempotency_keys (user_id, key, post_id) VALUES ($1,$2,$3)` with
  its own
  `.map_err(|e| if DB::is_unique_violation(&e) { IdempotencyConflict } else { … })`.
- `post_id_for_idempotency_key(&self, user_id, key) -> Result<Option<i64>, sqlx::Error>`
  on the `PostStorage` trait + the generic `PostStore<DB>` impl
  (`SELECT post_id FROM idempotency_keys WHERE user_id = $1 AND key = $2`, `$n`
  placeholders — no dialect method) + `MockPostStorage`.

**`storage/src/post_service.rs`:**

- `PostCreation<'a>` gains `idempotency_key: Option<&'a str>`; thread it through
  `RenderedPostContent` (owned `Option<String>`), `render_post_input`,
  `create_rendered_post` into `CreatePostInput`. `seed_post_input` (batch) sets
  `None`.
- `PerformCreationError` gains `IdempotencyConflict`; `perform_post_creation`'s
  `match` on `CreatePostError` gets a new arm:
  `IdempotencyConflict → return Err(Perform…::IdempotencyConflict)` (no slug
  retry). Add the matching arm **only** to the explicit
  `From<PerformCreationError> for InternalError` impl (`post_service.rs`); the
  `atompub/mod.rs → HandlerError` impl's `_` wildcard already covers it — do not
  touch it.

### Test (`storage/src/post_service.rs` `mod tests`, `#[apply(backends)]`)

- `perform_post_creation_dedups_on_idempotency_key` (AC-S5): create with key
  `"k"` → ok; second `perform_post_creation` with the same `user_id`+`"k"` →
  `IdempotencyConflict`; assert the post count for the user is 1 (no second
  row).
- `post_id_for_idempotency_key_maps` : returns the first post's id for `"k"`,
  `None` for an unknown key.
- `idempotency_key_is_per_user` (AC-S4): user A `"k"` and user B `"k"` both
  create; each `post_id_for_idempotency_key` returns that user's own post.
- `idempotency_conflict_converts_to_internal_error` (plain `#[test]`, no
  backend): construct `PerformCreationError::IdempotencyConflict`, `.into()` an
  `InternalError`, assert its kind — covers the otherwise-unreachable new `From`
  arm so the CRAP gate stays green.

Run: `cargo nextest run -p storage idempotency` — FAIL before, PASS after
(SQLite; Postgres via the gate).

### Commit

`cargo xtask check` clean →
`storage: idempotency-key dedup for post create (#79)`.

---

## Task 3 — handler dedup (Commit 3)

### Files / interfaces

**`server/src/atompub/posts.rs` `collection_post`** (now
`services: PostServices, auth_user, Path(username), headers: HeaderMap, body: String`):

- Read
  `let idem = headers.get("idempotency-key").and_then(|v| v.to_str().ok()).map(str::trim).filter(|s| !s.is_empty());`
- Pass `idempotency_key: idem` into `PostCreation`.
- Match `perform_post_creation`:
  - `Ok(created)` → `apply_categories` + re-fetch + **201** (as today).
  - `Err(Perform…::IdempotencyConflict)` → look up the key (a conflict implies
    `idem` is `Some`, but avoid a bare `unwrap`:
    `idem.ok_or(HandlerError::Internal)?`), then
    `post_id_for_idempotency_key(auth_user.user_id, key).await?.ok_or(HandlerError::Internal)?`,
    re-fetch via `get_post_by_id` (owner viewer, as the 201 path does) → build
    the **200** tuple (`StatusCode::OK`, CONTENT_TYPE/LOCATION/ETAG, xml). **No
    `apply_categories`.** Factor the shared Location/ETag/xml builder from the
    201 path (posts.rs:361-370, which includes LOCATION — not `member_put`'s
    200, which omits it).
  - other errors → `?` as today.

Factor the 200/201 response building to avoid duplicating the Location/ETag/xml
construction.

### Test (`server/tests/atompub/atompub_posts.rs`, `#[apply(backends)]`)

- `create_with_same_idempotency_key_dedups` (AC-S1): POST with
  `Idempotency-Key: k` → 201 + capture Location; second POST same key + same
  body → **200** + **same** Location; a GET of that Location is the one post.
- `create_with_fresh_idempotency_key_is_201` (AC-S2): two POSTs with different
  keys → both 201, two distinct Locations.
- `create_without_idempotency_key_is_201` (AC-S3): POST, no header → 201
  (unchanged).
- (AC-S4 covered at storage; optionally a two-user handler test.)

Run: `cargo nextest run -p jaunder atompub` (SQLite) → new tests PASS; existing
create tests green.

### Commit

`cargo xtask check` clean →
`atompub: dedup post create on Idempotency-Key (#79)`.

---

## Task 4 — client key + auto-retry (Commit 4)

### Files / interfaces

**`elisp/jaunder-config.el`** (or `jaunder-transport.el`): a generator + retry
policy consts.

```elisp
(defun jaunder--idempotency-key ()
  "Return a fresh opaque idempotency key (self-contained; no org-id dependency)."
  (md5 (format "%s%s%s" (random) (float-time) (emacs-pid))))
```

**`elisp/jaunder-publish.el`** — wrap the **create** POST (the `id`-absent
branch) in a retry that reuses one key:

```elisp
(defun jaunder--create-with-retry (url xml)
  "POST XML to URL as a create, with an Idempotency-Key and transient-failure retry.
Returns the `jaunder--http-request' response plist.  Retries a signalled transport
error or a 5xx status up to 3 attempts total, resending the same key; a 4xx or 2xx
returns immediately."
  (let ((key (jaunder--idempotency-key))
        (delays '(1 2))                 ; backoff before retries 2 and 3
        (attempt 0) resp)
    (while (null resp)
      (setq attempt (1+ attempt))
      (let ((r (condition-case err
                   (jaunder--http-request "POST" url xml jaunder--entry-content-type
                                          (list (cons "Idempotency-Key" key)))
                 (error (if (< attempt 3) 'retry (signal (car err) (cdr err)))))))
        (cond
         ((eq r 'retry) (sleep-for (pop delays)))
         ((and (integerp (plist-get r :status))
               (<= 500 (plist-get r :status) 599)
               (< attempt 3))
          (sleep-for (pop delays)))
         (t (setq resp r)))))
    resp))
```

Replace the create-branch `jaunder--http-request "POST" …` call in
`jaunder-publish` with `(jaunder--create-with-retry url xml)`. Confirm the exact
transport signature (5th arg = extra-headers alist, as the PUT/If-Match path
uses) at implementation.

### Test (`elisp/test/jaunder-test.el`, ERT)

Stub `jaunder--http-request` (stateful via a captured counter) and `sleep-for`
(no-op):

- `jaunder-create-retry-sends-key-and-retries-5xx` (AC-C2/C3/C4): first call
  returns `(:status 503 …)`, second returns `(:status 201 …)`; assert the
  response is the 201, both calls carried the **same** non-empty
  `Idempotency-Key`, exactly 2 calls.
- `jaunder-create-retry-no-retry-on-4xx` (AC-C3): stub returns
  `(:status 400 …)`; assert one call, returns the 400 plist (caller handles as
  today).
- `jaunder-create-retry-exhausts` (AC-C5): stub always signals a transport
  error; assert it errors after 3 calls.
- `jaunder-idempotency-key-fresh` (AC-C1/C6): two `jaunder--idempotency-key`
  calls are non-empty and differ.
- (AC-C1 no-property): the create flow writes no `JAUNDER_IDEMPOTENCY_KEY` —
  assert via the publish path or by inspection that no such
  `jaunder--set-property` call exists.

Run: `emacs --batch -Q -l elisp/scripts/run-tests.el` — FAIL before, PASS after.

### Commit

`cargo xtask check` clean (runs `ert` + `byte-compile` + `elisp-fmt`) →
`emacs: idempotency key + auto-retry on publish create (#79)`.

## Self-review

- Every AC maps to a test: AC-S1/2/3 → Task 3 handler tests; AC-S4/S5 + lookup →
  Task 2 storage tests; AC-S6 `#[apply(backends)]` on all; AC-C1–C6 → Task 4 ERT
  tests.
- Commits are ordered so each gates green: refactor (1) → storage (2) → handler
  consuming storage (3) → client (4). Task 0 files the one separable concern
  (GC) up front.
- No lint suppression added; one removed. No migration/back-compat risk (new
  table only).
