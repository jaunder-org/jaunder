# Session/User Test-Fixture Consolidation — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating a task to a subagent via **jaunder-dispatch**
> when useful — the file-by-file sweeps are ideal for that). Steps use checkbox
> (`- [ ]`) syntax. Spec:
> [`docs/superpowers/specs/2026-07-23-issue-626-session-test-helpers.md`](../specs/2026-07-23-issue-626-session-test-helpers.md).

**Goal:** Replace ~400 hand-rolled `create_user` / `create_session` /
`session_cookie` seeding sites across two crates with a small layered set of
test fixtures (`SeedUser` builder + `SeededSession` + session/atompub helpers),
deleting the five local copies and the `seed_user` alias.

**Architecture:** A `SeedUser` builder in `storage::test_support` is the one
home of the `create_user` parse block; server-side session helpers in
`server/tests/helpers/mod.rs` layer a session + cookie on top; an
`atompub_authed`/`atompub_xml` pair centralises AtomPub Basic-auth request
building. Each specialized helper delegates to a more general one — no seeding
block is copy-pasted. Every task is behaviour-preserving: the safety net is the
**pre-existing** test suite staying green, so sweep tasks have no red→green
cycle; their "test" step is confirming the untouched suite still passes.

**Tech Stack:** Rust, `cargo nextest`, the both-backend `TestEnv`/`Backend`
harness (ADR-0033), `cargo xtask check`/`validate`.

## Review header

**Scope in:** all `server/tests/**` and `storage/src/**` `#[cfg(test)]` seeding
sites; delete `seed_user`, `web_sessions::create_user_and_session`,
`web_site`/`web_backup::create_session_cookie`, `atompub_posts::seed_alice`;
delegate-in-place `web_password_reset::create_user_with_verified_email` &
`web_tags::seed_user_and_tagged_post`. **Scope out:** production signatures
(that's #325); `storage/src/**` non-`cfg(test)` bodies; the `test-support`
crate's production seeding logic.

**Tasks:**

1. Add the `SeedUser` builder (+ unit test); make `seed_user` delegate to it.
2. Sweep `storage/src/**` unit tests onto `SeedUser`; delete `seed_user`.
3. Add `SeededSession` + `create_session_for` + `create_user_and_session`; sweep
   `web_posts.rs`.
4. Sweep the remaining `web/*` + `feed/*` + `projector` files; delegate the two
   single-use helpers in place.
5. Add `create_operator_and_session`; sweep `web_site.rs` + `web_backup.rs`;
   delete their local `create_session_cookie`.
6. Sweep `server/tests/storage/mod.rs`.
7. Add `atompub_authed` + `atompub_xml`; sweep `atompub/*`; delete `seed_alice`.
8. Full-gate verification + acceptance-grep + doc polish.

**Key risks/decisions:**

- **Coverage gate runs on every commit** (ADR-0050) and measures test-support
  code, so each new fixture method must have a real caller or a unit test in the
  same commit. Task 1's `SeedUser` unit test covers all four builder methods up
  front, decoupling the builder from sweep timing.
- **Bespoke carve-outs must not be swept:** error-path `create_user` tests (they
  assert the error; the builder `expect()`s success) and label-asserting
  `create_session` calls stay direct. Each sweep step names them.
- **Label normalisation** `"MarsEdit"`/`"Laptop"`→`"test session"` is only safe
  where no test reads the label back — verified for atompub; NOT for the
  label-asserting storage/web_account/web_sessions tests, which stay direct.

## Global Constraints

- Rust edition/toolchain per repo; no new dependencies.
- Storage tests follow the dual-backend template (`#[apply(backends)]` +
  `#[case] backend: Backend`); do not convert a dual-backend test to
  single-backend.
- Per-commit gate: `cargo xtask check` (fmt + clippy + Nix coverage/tests) must
  be green before each commit (**jaunder-commit**). **No `Co-Authored-By`
  trailer.**
- Newtypes built in tests via `common::test_support::parse_<name>()` (fixture
  convention).
- Behaviour-preserving: no non-`cfg(test)` code path changes.

---

### Task 1: `SeedUser` builder

**Files:**

- Modify: `storage/src/test_support.rs` (add `SeedUser`; import
  `parse_display_name`; redirect `seed_user`)

**Interfaces:**

- Consumes:
  `common::test_support::{parse_username, parse_password, parse_display_name}`;
  `AppState`, `UserId`.
- Produces:

  ```rust
  pub struct SeedUser<'a> { /* private fields */ }
  impl<'a> SeedUser<'a> {
      pub fn new(username: &'a str) -> Self;          // password123, no display name, non-operator
      pub fn password(self, password: &'a str) -> Self;
      pub fn display_name(self, display_name: &'a str) -> Self;
      pub fn operator(self) -> Self;
      pub async fn seed(self, state: &Arc<AppState>) -> UserId;   // expect()s success — happy-path only
  }
  ```

- [ ] **Step 1: Write the failing test.** Add to `storage/src/test_support.rs` a
      `#[cfg(test)]` module (SQLite-only is fine — no per-backend branching,
      same rationale as the existing `create_user`/`seed` smoke tests). Pin
      defaults and every override:

```rust
#[cfg(test)]
mod seed_user_builder_tests {
    use super::*;

    #[tokio::test]
    async fn defaults_create_a_plain_non_operator_user() {
        let TestEnv { state, base: _base } = Backend::Sqlite.setup().await;
        let id = SeedUser::new("alice").seed(&state).await;
        let u = state.users.get_user(id).await.unwrap().expect("user exists");
        assert_eq!(u.username, "alice");
        assert!(!u.is_operator);
        assert!(u.display_name.is_none());
        // default password authenticates
        state.users.authenticate(&parse_username("alice"), &parse_password("password123"))
            .await.expect("default password authenticates");
    }

    #[tokio::test]
    async fn overrides_apply_password_display_name_and_operator() {
        let TestEnv { state, base: _base } = Backend::Sqlite.setup().await;
        let id = SeedUser::new("bob")
            .password("hunter2xyz")
            .display_name("Bob B")
            .operator()
            .seed(&state)
            .await;
        let u = state.users.get_user(id).await.unwrap().expect("user exists");
        assert!(u.is_operator);
        assert_eq!(u.display_name.as_ref().map(|d| d.as_ref()), Some("Bob B"));
        state.users.authenticate(&parse_username("bob"), &parse_password("hunter2xyz"))
            .await.expect("overridden password authenticates");
    }
}
```

(Verify `UserRecord`'s field/accessor names against `storage/src/users.rs` while
writing — use whatever the existing tests use for `username`/`display_name`/
`is_operator`.)

- [ ] **Step 2: Run the tests, verify they fail.** Run:
      `cargo nextest run -p storage seed_user_builder_tests` Expected: FAIL —
      `SeedUser` not defined.

- [ ] **Step 3: Implement `SeedUser`.** Add near `seed_user` in
      `storage/src/test_support.rs`, and add `parse_display_name` to the
      existing `use common::test_support::{…}` line:

```rust
/// Fixture for a seeded user, built the real `UserStorage::create_user` way.
/// Defaults: password `password123`, no display name, non-operator — chain the
/// setters to override only what a test varies. `seed()` `expect()`s success, so
/// it is happy-path setup only (error-path tests call `create_user` directly and
/// assert the error).
pub struct SeedUser<'a> {
    username: &'a str,
    password: &'a str,
    display_name: Option<&'a str>,
    is_operator: bool,
}

impl<'a> SeedUser<'a> {
    /// A non-operator user named `username`, password `password123`, no display name.
    #[must_use]
    pub fn new(username: &'a str) -> Self {
        Self { username, password: "password123", display_name: None, is_operator: false }
    }
    /// Override the password (auth/duplicate tests).
    #[must_use]
    pub fn password(mut self, password: &'a str) -> Self {
        self.password = password;
        self
    }
    /// Set a display name.
    #[must_use]
    pub fn display_name(mut self, display_name: &'a str) -> Self {
        self.display_name = Some(display_name);
        self
    }
    /// Mark the user an operator.
    #[must_use]
    pub fn operator(mut self) -> Self {
        self.is_operator = true;
        self
    }
    /// Create the user and return its id.
    ///
    /// # Panics
    /// If the username/password/display name fail to parse or the user cannot be created.
    pub async fn seed(self, state: &Arc<AppState>) -> UserId {
        let display_name = self.display_name.map(parse_display_name);
        state
            .users
            .create_user(
                &parse_username(self.username),
                &parse_password(self.password),
                display_name.as_ref(),
                self.is_operator,
            )
            .await
            .expect("seed user should be created")
    }
}
```

Then redirect the existing `seed_user` body (keep it for now — Task 2 deletes
it):

```rust
pub async fn seed_user(state: &Arc<AppState>) -> UserId {
    SeedUser::new("testuser").seed(state).await
}
```

- [ ] **Step 4: Run the tests, verify they pass.** Run:
      `cargo nextest run -p storage seed_user_builder_tests` Expected: PASS.

- [ ] **Step 5: Commit.** Run `cargo xtask check` first (green), then:

```bash
git add storage/src/test_support.rs
git commit -m "test(storage): add SeedUser fixture builder; seed_user delegates to it"
```

---

### Task 2: Sweep `storage/src/**` unit tests onto `SeedUser`; delete `seed_user`

**Files:**

- Modify:
  `storage/src/{posts,post_service,media,users,user_config,email,audiences,sessions,password,backup}.rs`,
  `storage/src/postgres/backup.rs`, `storage/src/test_support.rs` (delete
  `seed_user`), `test-support/src/lib.rs` (2 `#[cfg(test)]` call sites)

**Interfaces:**

- Consumes: `SeedUser` (Task 1).
- Produces: `seed_user` no longer exists.

_Behaviour-preserving mechanical sweep — no new tests; delegate per-file to a
subagent (**jaunder-dispatch**) if useful, briefing it: no
`ctx_\*`tools, behaviour-preserving, run`cargo nextest run -p storage` per
file.\_

- [ ] **Step 1: Replace `seed_user` callers.** Every `seed_user(&<state>)` →
      `SeedUser::new("testuser").seed(&<state>)`. In each file, drop `seed_user`
      from the `use crate::test_support::{…}` list and add `SeedUser`. Example
      (`storage/src/post_service.rs`):
  ```rust
  // before
  let user_id = seed_user(&env.state).await;
  // after
  let user_id = SeedUser::new("testuser").seed(&env.state).await;
  ```
- [ ] **Step 2: Migrate direct happy-path `create_user`/`create_session`
      setups.** In `backup.rs`, `postgres/backup.rs`, `post_service.rs`, and any
      `state.users.create_user(&parse_…(), …)` used only as _setup_, replace
      with
      `SeedUser::new(<name>)[.password(..)][.display_name(..)][.operator()].seed(&state)`.
      **Leave direct** (do not sweep): `users.rs` error-path `create_user` tests
      (`create_user_with_hash_error_returns_internal_error`,
      duplicate-username), and `sessions.rs`
      `create_session(user_id, "Test Device")` label tests — these are the
      subject under test.
- [ ] **Step 3: Migrate the two `test-support`-crate test call sites.** In
      `test-support/src/lib.rs`, the two `test_support::seed_user(&state)` in
      `seed_tests`/`create_user_tests` →
      `test_support::SeedUser::new("testuser").seed(&state)`.
- [ ] **Step 4: Delete `seed_user`** from `storage/src/test_support.rs`. This
      also removes the now-redundant in-file `seed_user_creates_a_user` test
      (`test_support.rs:740`) and drops `seed_user` from its `use super::{…}`
      import — Task 1's `seed_user_builder_tests` already covers that behaviour.
- [ ] **Step 5: Verify no caller remains.** Run:
      `rg -n 'seed_user\b' storage test-support -g '*.rs'` Expected: no match
      (the identifier is gone).
- [ ] **Step 6: Run the storage suite, verify green.** Run:
      `cargo nextest run -p storage` Expected: PASS (behaviour unchanged).
- [ ] **Step 7: Commit.** `cargo xtask check` first, then:

```bash
git add storage/src test-support/src/lib.rs
git commit -m "test(storage): route unit-test seeding through SeedUser; remove seed_user"
```

---

### Task 3: Session helpers + `web_posts.rs` sweep

**Files:**

- Modify: `server/tests/helpers/mod.rs` (add struct + helpers),
  `server/tests/web/web_posts.rs`

**Interfaces:**

- Consumes: `SeedUser` (Task 1); existing `session_cookie`.
- Produces:

  ```rust
  pub struct SeededSession { pub user_id: UserId, pub token: RawToken }
  impl SeededSession { pub fn cookie(&self) -> String; }
  pub async fn create_session_for(state: &Arc<storage::AppState>, user_id: UserId) -> SeededSession;
  pub async fn create_user_and_session(state: &Arc<storage::AppState>, username: &str) -> SeededSession;
  ```

- [ ] **Step 1: Add the helpers** to `server/tests/helpers/mod.rs` (add
      `use common::ids::UserId;` if absent):

```rust
/// A user seeded together with one authenticated web session. `token` is the raw
/// session token; `cookie()` renders the `session=<token>` header for cookie auth.
pub struct SeededSession {
    pub user_id: UserId,
    pub token: RawToken,
}

impl SeededSession {
    /// The `session=<token>` cookie header authenticating a request as this user.
    #[must_use]
    pub fn cookie(&self) -> String {
        session_cookie(&self.token)
    }
}

/// Create an authenticated `"test session"` session for an existing `user_id` —
/// the one place the default session label lives.
pub async fn create_session_for(
    state: &std::sync::Arc<storage::AppState>,
    user_id: UserId,
) -> SeededSession {
    let token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .expect("create session");
    SeededSession { user_id, token }
}

/// Seed a non-operator user and an authenticated web session — the workhorse.
pub async fn create_user_and_session(
    state: &std::sync::Arc<storage::AppState>,
    username: &str,
) -> SeededSession {
    let user_id = storage::test_support::SeedUser::new(username).seed(state).await;
    create_session_for(state, user_id).await
}
```

- [ ] **Step 2: Sweep `web_posts.rs`.** Replace each hand-rolled block. The
      three KEEP-shapes map as:
  ```rust
  // single-user (KEEP user_id + cookie)
  let s = create_user_and_session(&state, "author").await;
  // …use s.user_id and s.cookie()
  // two-user
  let author = create_user_and_session(&state, "author").await;
  let stranger = create_user_and_session(&state, "stranger").await;
  // inline cookie
  let cookie = create_user_and_session(&state, "author").await.cookie();
  ```
  Update downstream references: `user_id`→`s.user_id`, `cookie`→`s.cookie()` (or
  bind `let cookie = s.cookie();` where reused). The `"s"`-labelled inline sites
  (≈3291+) normalise to the default label — no test reads it back (grep-verify:
  `rg 'label' server/tests/web/web_posts.rs` shows no session-label assertion).
- [ ] **Step 3: Run, verify green.** Run:
      `cargo nextest run -p jaunder --test integration web::web_posts` Expected:
      PASS.
- [ ] **Step 4: Commit.** `cargo xtask check` first, then:

```bash
git add server/tests/helpers/mod.rs server/tests/web/web_posts.rs
git commit -m "test(web): add session fixtures; route web_posts seeding through them"
```

---

### Task 4: Sweep remaining `web/*`, `feed/*`, `projector`

**Files:**

- Modify:
  `server/tests/web/{web_media,web_account,web_auth,web_email,audiences,web_subscriptions,web_sessions,web_password_reset,web_tags}.rs`,
  `server/tests/misc/media_handlers.rs`,
  `server/tests/feed/{feed_events_hook,feed_handlers,feed_regenerate,feed_worker}.rs`,
  `server/tests/projector/mod.rs`

**Interfaces:**

- Consumes: `SeedUser`, `create_user_and_session`, `create_session_for` (Tasks
  1, 3).

_Mechanical, per-file; delegate to subagents (**jaunder-dispatch**), one brief
per file, restating: no `ctx_\*`, behaviour-preserving, run the file's nextest
filter, leave the enumerated bespoke sites.\_

- [ ] **Step 1: Cookie/token web files** (`web_media`, `web_account`,
      `web_auth`, `web_email`, `audiences`, `web_subscriptions`,
      `misc/media_handlers`): hand-rolled block → `create_user_and_session`;
      `.unwrap()`/`.expect()` variants both collapse. `web_auth` logout tests
      keep the token → use `s.token`. **Leave direct:** `web_account`
      `carol-session`/`dave-session` and the `display_name = Some` session tests
      (seed via `SeedUser::new(u).display_name("…").seed(&state)` then
      `create_session_for`); the multi-session revoke test may use the workhorse
      for the first session + a direct `create_session` for the second.
- [ ] **Step 2: Just-a-user files** (`feed/feed_handlers.rs`,
      `feed/feed_regenerate.rs`, `feed/feed_worker.rs`, `projector/mod.rs`,
      `web_email` no-session cases): `create_user(&…, &…, None, false)` →
      `SeedUser::new(<name>).seed(&state)`. **Exception —
      `feed/feed_events_hook.rs`** pairs `create_user` +
      `create_session(user_id, "test session")` (5 sites: ~30/78/168/247/326);
      it is the _workhorse_ pattern, so route it through
      `create_user_and_session(&state, <name>)`, not bare `SeedUser`.
- [ ] **Step 3: `web_sessions.rs`** — delete the local `create_user_and_session`
      (tuple) fn; its callers now use the shared struct
      (`.user_id`/`.token`/`.cookie()`). The extra labelled sessions
      (`"mobile"`, the list/revoke second sessions) **stay direct**.
- [ ] **Step 4: Delegate-in-place**
      `web_password_reset::create_user_with_verified_email` and
      `web_tags::seed_user_and_tagged_post` — keep the fns, rewrite bodies:
  ```rust
  async fn create_user_with_verified_email(state: &Arc<storage::AppState>, username: &str, email: &str) -> SeededSession {
      let s = create_user_and_session(state, username).await;
      state.users.set_email(s.user_id, Some(&parse_email(email)), true).await.expect("set verified email");
      s
  }
  ```
  Update its call sites to the struct (`s.user_id`; the `(user_id, _)`
  destructures become `.user_id`). `seed_user_and_tagged_post` seeds its user
  via `SeedUser::new(username).seed(state)`.
- [ ] **Step 5: Run each file green** as swept, e.g.
      `cargo nextest run -p jaunder --test integration web::web_account`, …,
      `feed::feed_worker`. Expected: PASS.
- [ ] **Step 6: Commit** (one commit; or per-file if delegating).
      `cargo xtask check` first:

```bash
git add server/tests/web server/tests/misc/media_handlers.rs server/tests/feed server/tests/projector
git commit -m "test(server): route web/feed/projector seeding through session fixtures"
```

---

### Task 5: Operator helper + `web_site`/`web_backup` sweep

**Files:**

- Modify: `server/tests/helpers/mod.rs` (add `create_operator_and_session`),
  `server/tests/web/web_site.rs`, `server/tests/web/web_backup.rs`

**Interfaces:**

- Consumes: `SeedUser::operator` (Task 1), `create_session_for` (Task 3).
- Produces:

  ```rust
  pub async fn create_operator_and_session(state: &Arc<storage::AppState>, username: &str) -> SeededSession;
  ```

- [ ] **Step 1: Add the helper** to `server/tests/helpers/mod.rs`:

```rust
/// Like [`create_user_and_session`] but the user is an operator.
pub async fn create_operator_and_session(
    state: &std::sync::Arc<storage::AppState>,
    username: &str,
) -> SeededSession {
    let user_id = storage::test_support::SeedUser::new(username).operator().seed(state).await;
    create_session_for(state, user_id).await
}
```

- [ ] **Step 2: Sweep** both files. Delete the local
      `create_session_cookie(state, username, is_operator) -> String`. Its call
      sites: `create_session_cookie(&state, "operator", true)` →
      `create_operator_and_session(&state, "operator").await.cookie()`;
      `(…, "member", false)` →
      `create_user_and_session(&state, "member").await.cookie()`.
- [ ] **Step 3: Run green.** Run:
      `cargo nextest run -p jaunder --test integration web::web_site web::web_backup`
      Expected: PASS.
- [ ] **Step 4: Commit.** `cargo xtask check` first:

```bash
git add server/tests/helpers/mod.rs server/tests/web/web_site.rs server/tests/web/web_backup.rs
git commit -m "test(web): add create_operator_and_session; route site/backup seeding through it"
```

---

### Task 6: Sweep `server/tests/storage/mod.rs`

**Files:**

- Modify: `server/tests/storage/mod.rs`

**Interfaces:**

- Consumes: `SeedUser`, `create_user_and_session`, `create_session_for`.

_Largest single file (7.7k lines, 148 `create_user` / 10 `create_session`).
Delegate to a subagent; brief it to migrate only happy-path setups and to leave
the enumerated subject-under-test sites._

- [ ] **Step 1: Migrate happy-path `create_user` setups.**
      `state.users.create_user(&username("alice"), &password("password123"), None, false)`
      → `SeedUser::new("alice").seed(&state)`; with a non-default password →
      `.password("pw1234567")`; with `Some(&display_name(..))` →
      `.display_name("…")`. (The local `username()`/`password()` parse helpers
      stay for any remaining direct calls.)
- [ ] **Step 2: Leave direct (subject under test):** error-path `create_user`
      (duplicate/hash-failure) tests; every `create_session` whose label is
      asserted (`"Laptop"`, `"test"`, `"alice-1"`/`"alice-2"`/`"bob-1"`,
      `"session 1"`/`"2"`, and the list/revoke tests). Incidental
      `"test session"` sessions (`authenticate_updates_last_used_at`, the revoke
      setup) may use `create_session_for(&state, user_id)` (use `.token`).
- [ ] **Step 3: Run green.** Run:
      `cargo nextest run -p jaunder --test integration storage` Expected: PASS.
- [ ] **Step 4: Commit.** `cargo xtask check` first:

```bash
git add server/tests/storage/mod.rs
git commit -m "test(storage): route storage integration-test seeding through SeedUser"
```

---

### Task 7: AtomPub request helpers + `atompub/*` sweep

**Files:**

- Modify: `server/tests/helpers/mod.rs` (add `atompub_authed`/`atompub_xml`),
  `server/tests/atompub/{atompub_posts,atompub_media,atompub_service,atompub_rsd}.rs`

**Interfaces:**

- Consumes: existing `basic_header`; `Request`, `Body`, `header`.
- Produces:

  ```rust
  pub fn atompub_authed(method: &str, uri: &str, username: &str, token: &RawToken) -> axum::http::request::Builder;
  pub fn atompub_xml(method: &str, uri: &str, username: &str, token: &RawToken, body: Option<&str>) -> Request<Body>;
  ```

- [ ] **Step 1: Add the helpers** to `server/tests/helpers/mod.rs`:

```rust
/// A `Request::builder()` preloaded with `method`, `uri`, and `Authorization:
/// Basic <username:token>` — the base every authenticated AtomPub request shares.
/// Callers add extra headers (`If-Match`, `slug`, `Idempotency-Key`, content-type)
/// and finish with `.body(...)`.
#[must_use]
pub fn atompub_authed(
    method: &str,
    uri: &str,
    username: &str,
    token: &RawToken,
) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, basic_header(username, token))
}

/// The dominant AtomPub case: Basic auth + optional `application/atom+xml` body.
/// `Some(xml)` sends the entry body; `None` sends an empty body (GET/DELETE).
#[must_use]
pub fn atompub_xml(
    method: &str,
    uri: &str,
    username: &str,
    token: &RawToken,
    body: Option<&str>,
) -> Request<Body> {
    let builder = atompub_authed(method, uri, username, token);
    match body {
        Some(xml) => builder
            .header(header::CONTENT_TYPE, "application/atom+xml")
            .body(Body::from(xml.to_owned())),
        None => builder.body(Body::empty()),
    }
    .expect("failed to build atompub request")
}
```

- [ ] **Step 2: Delete both local `seed_alice` fns.** There are **two**:
      `atompub_posts.rs:467` (`-> (UserId, RawToken)`) and
      `atompub_media.rs:278` (`-> RawToken`). Replace with
      `create_user_and_session(&state, "alice")` — callers of the posts one use
      `.user_id`/`.token`, callers of the media one use `.token`.
      `"MarsEdit"`→`"test session"` is safe
      (`rg '\.label' server/tests/atompub/` is empty).
- [ ] **Step 3: Sweep the request builders.**
  - GET/DELETE (empty body):
    `app.oneshot(atompub_xml("GET", &uri, "alice", &token, None)).await`
  - POST/PUT XML:
    `atompub_xml("POST", "/atompub/alice/posts", "alice", &token, Some(&xml))`
  - **Media uploads** (`atompub_media.rs`) and **`If-Match`**
    (`atompub_posts.rs`) use the base + explicit headers:
    ```rust
    atompub_authed("POST", &uri, "alice", &token)
        .header(header::CONTENT_TYPE, "image/png").header("slug", "pic.png")
        .body(Body::from(PNG)).unwrap()
    atompub_authed("PUT", &uri, "alice", &token)
        .header(header::IF_MATCH, etag).header(header::CONTENT_TYPE, "application/atom+xml")
        .body(Body::from(xml)).unwrap()
    ```
    Leave any unauthenticated negative-test `Request::builder()` (no
    `basic_header`) direct.
- [ ] **Step 4: Run green.** Run:
      `cargo nextest run -p jaunder --test integration atompub` Expected: PASS.
- [ ] **Step 5: Commit.** `cargo xtask check` first:

```bash
git add server/tests/helpers/mod.rs server/tests/atompub
git commit -m "test(atompub): centralise Basic-auth request building; drop seed_alice"
```

---

### Task 8: Full-gate verification, acceptance grep, doc polish

**Files:**

- Modify (if grep surfaces stragglers): any test file with a missed site.

- [ ] **Step 1: Acceptance greps** (spec §Acceptance). Record the residue:
  - `rg -n 'fn seed_user\b|seed_user\(' storage test-support server -g '*.rs'` →
    **empty** (AC5).
  - `rg -n 'create_user\(&' server/tests -g '*.rs'` → only the enumerated
    error-path `create_user` tests (`users.rs`/`storage/mod.rs` duplicate/hash);
    all happy-path seeding goes through `SeedUser` (AC2). (Note:
    `backup_fixture.rs` wraps its `create_user(` arg onto the next line, so it
    never matches this single-line pattern.)
  - `rg -n '\.create_session\(' server/tests -g '*.rs'` → only `helpers/mod.rs`
    (1, `create_session_for`) + enumerated label-asserting sites in
    `storage/mod.rs`/`web_account.rs`/`web_sessions.rs` (AC3).
  - `rg -n 'basic_header' server/tests/atompub` → only inside `atompub_authed`
    calls + enumerated unauth negatives (AC4).
  - `rg -n 'create_user_and_session|create_session_cookie|seed_alice' server/tests`
    → the shared helper + no surviving local defs (AC1).
- [ ] **Step 2: Fix any straggler** the greps surface (missed hand-rolled
      block), commit as a fixup into the relevant task's commit if still local,
      else a small follow-up commit.
- [ ] **Step 3: Full local gate.** Run (foreground, long):
      `cargo xtask validate` (static + clippy + coverage + e2e). Expected: green
      — proves behaviour preserved and coverage of every fixture branch (AC6).
- [ ] **Step 4: Verify no non-test source changed.** Run:
      `git diff --stat wt-base-issue-626..HEAD -- ':!**/tests/**' ':!**/test_support.rs'`
      Expected: empty (AC6 — only `#[cfg(test)]` + `test_support.rs`; note
      `storage/src/*.rs` entries appear but their diffs must be
      `#[cfg(test)]`-only — spot-check).
- [ ] **Step 5: Commit** any doc/comment polish. `cargo xtask check` first:

```bash
git add -A
git commit -m "test: finalize session/user fixture consolidation (#626)"
```

## Self-review notes

- **Spec coverage:** AC1→Tasks 2/3/5/7 (local helpers gone); AC2→Tasks 1/2/6
  (`create_user` centralised); AC3→Tasks 3–7 (`create_session` residue);
  AC4→Task 7; AC5→Task 2; AC6→Task 8; AC7 (docs)→doc comments in Tasks 1/3/5/7.
  All covered.
- **Type consistency:** `SeededSession { user_id, token }` + `.cookie()`,
  `SeedUser::{new,password,display_name,operator,seed}` used identically across
  Tasks 1/3/5/7. `create_session_for`/`create_user_and_session`/
  `create_operator_and_session` signatures stable.
- **Coverage-per-commit:** every fixture method has a same-commit caller —
  builder methods via Task 1's unit test; session/operator/atompub helpers via
  their sweep.
