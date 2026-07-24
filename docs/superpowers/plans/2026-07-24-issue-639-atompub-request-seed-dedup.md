# Plan — #639: AtomPub/feed request-construction + post-seeding dedup

Spec:
[`2026-07-24-issue-639-atompub-request-seed-dedup.md`](../specs/2026-07-24-issue-639-atompub-request-seed-dedup.md)
· Issue jaunder-org/jaunder#639 · **For agentic workers:** drive with
`jaunder-iterate`, delegating a task to `jaunder-dispatch` when useful.

## Review header

**Goal.** Behaviour-preserving refactor of the AtomPub/feed integration suite: a
service-layer `SeedPost` builder for post setup, a session-keyed AtomPub
request-helper family, and aggressive `#[case]` parameterization of the
CRUD-variant families. No production or runtime-behaviour change.

**Scope.**

- **In:** `storage::test_support` (add `SeedPost`);
  `server/tests/helpers/mod.rs` (add session-keyed helpers +
  `SeededSession::seed_post()`); migrate **post-seeding in `atompub_posts.rs`**
  and **AtomPub request sites** in
  `server/tests/atompub/{atompub_posts,atompub_media,atompub_service}.rs`;
  parameterize the uniform families in `atompub/*` + `feed/*`.
- **Out:** production code (untouched); `storage::test_support::seed_posts`
  (untouched); **every `create_post`-layer `CreatePostInput` seeding site** —
  the `storage`-crate contract suite _and_ the `feed/*`, `projector/*`, `web/*`,
  `misc/*` integration tests (they seed via `create_post` with explicit
  `rendered_html`/slug, not `perform_post_creation`) → **#656**
  (`blocked-by #639`), already filed & scoped; #640 (separate).

**Tasks (one line each).**

- [x] 1. `SeedPost` builder + its dual-backend unit tests in
     `storage::test_support`.
- [x] 2. `SeededSession::seed_post()` forwarder + migrate `atompub_posts.rs`
     post-seeding to `SeedPost`.
- [x] 3. Session-keyed AtomPub request helpers + migrate `atompub_posts.rs` &
     `atompub_service.rs`.
- [ ] 4. `atompub_upload` + migrate `atompub_media.rs` request sites.
- [ ] 5. Aggressive `#[case]` parameterization of the uniform CRUD-variant
     families.

**Key risks / decisions.**

- **Behaviour preservation is the whole game.** Every migrated test must assert
  the _same_ things through the new fixtures/helpers. The existing suite
  (unchanged assertions) is the safety net; `cargo xtask check` green after each
  task is the proof.
- **Dead-code discipline:** a `pub` helper in `helpers/mod.rs` with no caller
  trips clippy `-D warnings`. So each helper is introduced **in the same task
  that first uses it** (`atompub_upload` waits for Task 4), never earlier.
- **`SeedPost` wraps `perform_post_creation`** (service layer, auto-render +
  slug-retry). Only `atompub_posts.rs` seeds at that layer; the `feed/*`,
  `projector/*`, `web/*`, `misc/*` sites seed at the lower `create_post` layer
  (explicit `rendered_html`/slug) and are **out of scope** here (#656) —
  migrating them to `SeedPost` would silently swap their rendering/slug path.
  This is the corrected layer boundary from plan review.
- **Parameterization must not drop a case.** Task 5 is last so it parameterizes
  tests that already use the shared helpers; each `#[case]` row must map 1:1 to
  a pre-existing test (or documented merge), never silently fewer.

## Global constraints

- No `Co-Authored-By` trailer on commits.
- Before every commit: `cargo xtask check` clean (fmt + clippy + Nix
  coverage/tests); it auto-fixes fmt, so `git status --porcelain` after green
  (see `jaunder-commit`).
- Storage-crate tests are **dual-backend** (`#[apply(backends)]` /
  `backends_matrix`) — a bare `#[tokio::test]` fails the `test-backend-pattern`
  guard (`CONTRIBUTING.md`).
- Server integration tests run as:
  `cargo nextest run -p jaunder --test integration <filter>`.
- Import discipline: `use` the new helpers into each test module; drop `crate::`
  / long `storage::` prefixes at call sites where the module already imports the
  short form.
- This is a **behaviour-preserving** refactor: do not add/remove/alter any
  assertion except the mechanical helper/fixture substitution and the Task-5
  parameterization (which preserves every case).

---

## Task 1 — `SeedPost` builder in `storage::test_support`

**Files.**

- `storage/src/test_support.rs` — add `SeedPost<'a>` beside `SeedUser` /
  `seed_posts`.
- `storage/src/post_service.rs` — reuse `perform_post_creation`, `PostCreation`,
  `PostFormat` (no change; just the call target).

**Interface.** A builder mirroring `SeedUser`, wrapping `perform_post_creation`:

```rust
/// A single seeded post, built the real `perform_post_creation` way (slug-retry,
/// rendering, re-read). Aggressively defaulted: a published, public, Markdown post
/// with a non-empty body. A test deviates from a default ONLY when it asserts on
/// (or requires) that field — the `SeedUser` discipline. Distinct from `seed_posts`
/// (batch, generic) and from the `create_post`-layer builder (#656).
pub struct SeedPost<'a> {
    user_id: UserId,
    title: Option<&'a str>,
    body: PostBody,
    format: PostFormat,
    slug_override: Option<Slug>,
    published_at: Option<DateTime<Utc>>,   // Some(now) by default → published
    summary: Option<PostSummary>,
    audiences: Vec<AudienceTarget>,        // vec![Public] by default
    idempotency_key: Option<&'a str>,
}

impl<'a> SeedPost<'a> {
    #[must_use]
    pub fn new(user_id: UserId) -> Self { /* defaults per the spec table; body = a
        fixed non-empty default, e.g. "Seeded post body" */ }

    #[must_use] pub fn title(mut self, t: &'a str) -> Self { … }
    #[must_use] pub fn body(mut self, b: impl Into<PostBody>) -> Self { … }
    #[must_use] pub fn format(mut self, f: PostFormat) -> Self { … }
    #[must_use] pub fn draft(mut self) -> Self { self.published_at = None; self }
    #[must_use] pub fn published_at(mut self, at: DateTime<Utc>) -> Self { … }
    #[must_use] pub fn summary(mut self, s: PostSummary) -> Self { … }
    #[must_use] pub fn audiences(mut self, a: Vec<AudienceTarget>) -> Self { … }
    #[must_use] pub fn idempotency_key(mut self, k: &'a str) -> Self { … }

    /// Persist via `perform_post_creation` (`max_attempts = 100`, internal) and
    /// return the re-read `PostRecord` (carries `post_id` and `slug`). Panics on
    /// error — happy-path setup only, like `SeedUser::seed`.
    pub async fn seed(self, state: &Arc<AppState>) -> PostRecord { … }
}
```

Gate exactly as `SeedUser` (in-process test use). Add only the setters listed
(the fields the ~52 existing blocks vary); no speculative setters.

**Test.** `storage/src/test_support.rs` inline `#[cfg(test)]` (or the crate's
chosen storage-test file) — dual-backend, mirroring the `seed_user_builder_*`
tests:

- `seed_post_builder_defaults_create_published_public_markdown` —
  `SeedPost::new(uid).seed(&state)` yields a `PostRecord` that is published
  (`published_at.is_some()`), Public audience, Markdown, non-empty body,
  non-empty slug.
- `seed_post_bare_repeated_seeds_get_distinct_slugs` — two bare
  `SeedPost::new(uid).seed()` calls return distinct `slug`s (collision-suffix
  retry).
- `seed_post_draft_is_unpublished` — `.draft().seed()` →
  `published_at.is_none()`.

**Run.**

- `cargo nextest run -p storage seed_post` → **FAIL** (builder absent) then
  **PASS**.
- `cargo xtask check` → clean.

**Commit.** `test(storage): add SeedPost builder to test_support (#639)`.

---

## Task 2 — `SeededSession::seed_post()` + migrate `atompub_posts.rs` post-seeding

**Files.**

- `server/tests/helpers/mod.rs` — add the forwarder:
  ```rust
  impl SeededSession {
      /// A `SeedPost` pre-owned by this session's user — so authed tests never
      /// re-type `session.user_id`. `.seed(&state)` still takes state (irreducible).
      #[must_use]
      pub fn seed_post(&self) -> storage::test_support::SeedPost<'_> {
          storage::test_support::SeedPost::new(self.user_id)
      }
  }
  ```
- `server/tests/atompub/atompub_posts.rs` (~16 blocks) — migrate every routine
  `storage::perform_post_creation(state.posts.as_ref(), storage::PostCreation { … })`
  block → `session.seed_post()…seed(&state)` (every site here holds a
  `session`).

**Scope note.** Only `atompub_posts.rs` seeds at the `perform_post_creation`
service layer. The `feed/*`, `projector/*`, `web/*`, `misc/*` sites seed at the
`create_post` storage layer (explicit `rendered_html`/slug) and are **#656**,
not this task — do not touch them here.

**Rules.**

- Bare `session.seed_post().seed(&state).await` unless the test asserts on a
  varied field — then and only then chain a setter (`.title("Hello Title One")`
  for the collection-listing assertions, `.draft()` for the scheduled/draft
  cases, `.audiences(…)` for the visibility cases, `.slug_override(…)` for the
  client-supplied-slug case). This is AC6.
- **Do not** migrate the `perform_post_creation`-contract tests in
  `post_service.rs` (idempotency-key dedup, empty-post) — they exercise the
  function's own contract (AC1 carve-out). Leave a one-line comment on any
  borderline site kept as a literal.
- Update `atompub_posts.rs`'s `use crate::helpers::{…}` — `SeedPost` is reached
  via `session.seed_post()`, so no direct `SeedPost` import is needed unless a
  session-less site appears (none expected here).

**Run.**

- `cargo nextest run -p jaunder --test integration atompub::atompub_posts` →
  **PASS**
- `cargo xtask check` → clean.

**Verify (behaviour).** No assertion changed; the same posts (same
title/body/published/ audience where a test asserts on them) exist through the
fixture. Green suite = preserved.

**Commit.** `test(atompub): seed posts via SeedPost/session.seed_post (#639)`.

---

## Task 3 — session-keyed AtomPub request helpers + `atompub_posts`/`atompub_service`

**Files.**

- `server/tests/helpers/mod.rs` — add, layered on `atompub_authed`:

  ```rust
  /// Full AtomPub URI for `session`'s user: `/atompub/{username}/{suffix}`.
  fn atompub_uri(session: &SeededSession, suffix: &str) -> String { … }

  /// Chainable Basic-authed builder against the session user's `suffix` resource —
  /// the base for the extra-header cases (If-Match, Idempotency-Key, media slug).
  pub fn atompub(session: &SeededSession, method: &str, suffix: &str) -> request::Builder { … }

  /// Basic-authed builder against a **verbatim** URI (a captured `Location`), auth
  /// still from the session — so username/token are never doubled.
  pub fn atompub_at(session: &SeededSession, method: &str, uri: &str) -> request::Builder { … }

  pub fn atompub_get(session: &SeededSession, suffix: &str) -> Request<Body> { … }          // GET, empty body
  pub fn atompub_send_xml(session: &SeededSession, method: &str, suffix: &str, xml: &str) -> Request<Body> { … }
  pub fn atompub_post_xml(session: &SeededSession, suffix: &str, xml: &str) -> Request<Body> { atompub_send_xml(session, "POST", suffix, xml) }
  pub fn atompub_put_xml(session: &SeededSession, suffix: &str, xml: &str) -> Request<Body> { atompub_send_xml(session, "PUT", suffix, xml) }
  ```

- `server/tests/atompub/atompub_posts.rs` (~54 request sites) and
  `server/tests/atompub/atompub_service.rs` (~3) — migrate:
  - `atompub_xml("GET", &format!("/atompub/{}/posts…", session.username), &session.username, &session.token, None)`
    → `atompub_get(&session, "posts…")`.
  - POST/PUT create/update →
    `atompub_post_xml`/`atompub_put_xml(&session, "posts…", &xml)`.
  - If-Match / Idempotency-Key cases →
    `atompub(&session, method, suffix).header(…).body(…)`.
  - `Location`-follow-up PUT/DELETE/GET →
    `atompub_at(&session, method, &location)`.
  - Cross-user negative tests targeting a _foreign_ username keep the literal
    foreign prefix (AC4 carve-out) — a one-line comment noting why.

**Rules.** After this task, no `atompub_posts`/`atompub_service` request repeats
`session.username` twice and none hardcodes the `/atompub/{session.username}/`
own-user prefix (AC4). `atompub_authed`/`atompub_xml` may remain as the
primitives these wrap.

**Run.**

- `cargo nextest run -p jaunder --test integration atompub::atompub_posts` →
  **PASS**
- `cargo nextest run -p jaunder --test integration atompub::atompub_service` →
  **PASS**
- `cargo xtask check` → clean.

**Commit.**
`test(atompub): session-keyed request helpers for posts/service (#639)`.

---

## Task 4 — `atompub_upload` + migrate `atompub_media.rs`

**Files.**

- `server/tests/helpers/mod.rs` — add (first use is here, so no dead-code):
  ```rust
  /// Media POST for `session`'s user: `image/png` + `slug` header + `bytes` body.
  pub fn atompub_upload(session: &SeededSession, slug: &str, bytes: &'static [u8]) -> Request<Body> { … }
  ```
- `server/tests/atompub/atompub_media.rs` (~15 request sites) — migrate:
  - the `image/png` + `slug` + bytes uploads →
    `atompub_upload(&session, "pic.png", PNG)`.
  - the odd cases (`slug = ".."`, a non-`image/png` content type, cross-user
    foreign username) → `atompub(&session, "POST", "media").header(…).body(…)`
    (or the foreign literal), so `atompub_upload` stays the fixed happy path.

**Run.**

- `cargo nextest run -p jaunder --test integration atompub::atompub_media` →
  **PASS**
- `cargo xtask check` → clean.

**Commit.** `test(atompub): atompub_upload helper for media requests (#639)`.

---

## Task 5 — aggressive `#[case]` parameterization of uniform families

**Files.** `server/tests/atompub/atompub_posts.rs`, `atompub_media.rs`,
`server/tests/feed/{feed_handlers,feed_regenerate,feed_worker}.rs`.

**Approach.** For each CRUD-variant family that differs **only by data** (URI
fragment / body / expected status), collapse the copy-pasted `#[tokio::test]`
fns into one `#[apply(backends_matrix)]` test with local `#[case]` rows — one
row per pre-existing test, same name-per-case where practical. Named families
(from the spec / survey):

- the collection GET `limit`/pagination variants (`atompub_posts.rs`),
- the media upload variants (`atompub_media.rs` — content-type / slug /
  expected-status),
- the feed-format `{rss, atom, json}` triple (`feed_handlers.rs`),
- any other family the sweep surfaces meeting the data-only-varying bar.

Leave a family **unrolled** only when its variants differ structurally
(different setup, seeding, or assertions) — with a one-line comment stating why
(AC5).

**Rules.** Every `#[case]` row maps 1:1 to a case that existed before (no
dropped coverage); `#[apply(backends_matrix)]` keeps both backends via
`#[values]` × local `#[case]`. Behaviour preserved: the parameterized test
asserts exactly what the unrolled ones did.

**Run.**

- `cargo nextest run -p jaunder --test integration atompub` → **PASS** (same
  case count, modulo intended merges — eyeball the nextest list before/after).
- `cargo nextest run -p jaunder --test integration feed` → **PASS**
- `cargo xtask check` → clean.

**Commit.**
`test(atompub,feed): parameterize uniform CRUD-variant families (#639)`.

---

## Final gate

After Task 5, the full local gate before ship (`jaunder-ship` will re-run it):

- `cargo xtask validate` → green (static + clippy + coverage + e2e). This is the
  behaviour-preservation proof for the whole refactor.
- `git status --porcelain` clean (no stray fmt fixes uncommitted).

## Self-review checklist (against the spec's ACs)

- [ ] AC1 — no routine `PostCreation` literal in `atompub_posts.rs`;
      `create_post`-layer sites (→#656) and `perform_post_creation`-contract
      carve-outs left untouched.
- [ ] AC2 — `SeedPost` shape + defaults + `.seed → PostRecord`; bare form
      compiles.
- [ ] AC3 — `SeededSession::seed_post()`; authed seeding sites use it, no
      `session.user_id` threaded into `SeedPost::new`.
- [ ] AC4 — no doubled `session.username`, no own-user hardcoded prefix;
      foreign/Location carve-outs honoured.
- [ ] AC5 — uniform families parameterized; unrolled ones carry a one-line
      reason.
- [ ] AC6 — setters only where a test asserts on the varied field.
- [ ] AC7 — `cargo xtask validate` green.
