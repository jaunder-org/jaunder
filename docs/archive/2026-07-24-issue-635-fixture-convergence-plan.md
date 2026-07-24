# Fixture Convergence (#635) Implementation Plan

> **For agentic workers:** Execute with **jaunder-iterate** (delegating the bulk
> sweep to subagents via **jaunder-dispatch**). Spec:
> [`docs/superpowers/specs/2026-07-24-issue-635-fixture-convergence.md`](../specs/2026-07-24-issue-635-fixture-convergence.md).

**Goal:** Autogenerate all seed usernames (reference only via `.username`),
enrich the fixture return types, inline mirror-variable noise, and dedup local
seed→cookie wrappers — across the whole swept test tree.

**Architecture:** The fixture-signature change (`SeedUser::new()` arg-less +
`seed() -> SeededUser`; session helpers arg-less + `SeededSession.username`)
breaks every call site simultaneously, so Task 2 is **one atomic commit** (API +
full sweep); there is no green intermediate. Mirror-inlining and wrapper-dedup
fold into the same per-file pass. Behaviour-preserving throughout.

**Tech Stack:** Rust, `cargo nextest`, both-backend `TestEnv`/`Backend`,
`cargo xtask`.

## Review header

**Scope in:** autogen usernames + `.username` field on
`SeededUser`/`SeededSession`; `seed() -> SeededUser`; session helpers drop the
username param; inline single-use mirror bindings; dedup local wrapper helpers.
**Scope out (Task 1 files them):** `seed_post` helper; `Arc<AppState>`-by-value
→ `Arc::clone` churn. Also untouched: `web_auth`/`web_email` wire-body username
payloads; the #626 bespoke carve-outs.

**Tasks:**

1. File the two out-of-scope concerns as issues.
2. The convergence — fixture API + full call-site sweep + autogen unit test (one
   commit).
3. Verification — acceptance greps + full gate + diff-scope check.

**Key risks/decisions:**

- **Atomic API change** (breaks all ~370 sites at once) → subagents do
  mechanical edits without individual compile; integration compile+fix happens
  once at 2d.
- **id+name = 2 uses → keep the struct bound** (spec §Mirror-variable): do NOT
  inline-and-drop a `SeededUser`/`SeededSession` whose name a later line reads.
- **Autogen counter correctness** rests on the fresh-DB-per-test invariant
  (documented on `SeedUser`); a violation fails loud (`UsernameTaken` panic).
- **`str::contains`** on a username needs `.as_str()` (no Deref for generic
  `Pattern`).

## Global Constraints

- No new dependencies; no `#[allow]`/`#[expect]`. Dual-backend template
  preserved.
- Per-commit gate `cargo xtask check` green (**jaunder-commit**). No
  `Co-Authored-By`.
- Newtypes in tests via `common::test_support::parse_<name>()`.
- Behaviour-preserving: only `#[cfg(test)]` + feature-gated `test_support.rs`.

---

### Task 1: File the two out-of-scope concerns

**Files:** none (tracker only).

- [ ] **Step 1: File `seed_post` fixture issue** (**jaunder-issues**,
      `--type Task --label dx`, add to Backlog). Body: ~50 repeated
      `PostCreation`/`CreatePostInput` 10-field blocks (32 in
      `atompub_posts.rs`) → a `seed_post(&state, user_id, title)` / `SeedPost`
      builder; note the existing `seed_posts` makes only generic posts, so a
      titled/published/audience-varied helper is the gap. Blocked-by #635.
- [ ] **Step 2: File the `Arc<AppState>` issue** (`--type Task --label dx`).
      Body: `post_form`/`post_json`/`post_form_with_mailer`/… take
      `Arc<AppState>` by value, forcing pervasive `Arc::clone(&state)` +
      `mailer.clone() as Arc<dyn MailSender>`; change to
      `&Arc<AppState>`/`&AppState` to erase the clones; note the last-use move
      wrinkle to check. Independent of #635.
- [ ] **Step 3:** record both issue numbers in this plan for the PR body. No
      commit.

---

### Task 2: Fixture API + full call-site sweep (one atomic commit)

**Files:**

- Modify: `storage/src/test_support.rs` (SeededUser, counter, SeedUser, autogen
  test)
- Modify: `server/tests/helpers/mod.rs` (SeededSession.username, session
  helpers)
- Modify (sweep): all
  `server/tests/{web,atompub,feed,misc,projector,storage}/**` + `storage/src/**`
  `#[cfg(test)]` seeding sites.

**Interfaces — Produces:**

```rust
// storage::test_support
pub struct SeededUser { pub user_id: UserId, pub username: Username }
pub struct SeedUser<'a> { /* password, display_name, is_operator — no username */ }
impl<'a> SeedUser<'a> {
    pub fn new() -> Self;                                  // Default; autogen name
    pub fn password(self, p: &'a str) -> Self;
    pub fn display_name(self, d: &'a str) -> Self;
    pub fn operator(self) -> Self;
    pub async fn seed(self, state: &Arc<AppState>) -> SeededUser;
}
// server/tests/helpers
pub struct SeededSession { pub user_id: UserId, pub username: Username, pub token: RawToken }
impl SeededSession { pub fn cookie(&self) -> String; }
pub async fn create_user_and_session(state: &Arc<AppState>) -> SeededSession;
pub async fn create_operator_and_session(state: &Arc<AppState>) -> SeededSession;
pub async fn create_session_for(state: &Arc<AppState>, user_id: UserId) -> SeededSession;
```

- [ ] **Step 2a: Autogen unit test (write first).** In `test_support.rs`'s
      `mod tests`, dual-backend (`#[apply(backends)]`), assert two
      `SeedUser::new()` seeds in one test produce **distinct** usernames and
      both are retrievable:

```rust
#[apply(backends)]
#[tokio::test]
async fn seed_user_autogenerates_distinct_usernames(#[case] backend: Backend) {
    let env = backend.setup().await;
    let a = SeedUser::new().seed(&env.state).await;
    let b = SeedUser::new().seed(&env.state).await;
    assert_ne!(a.username, b.username, "each seed gets a fresh name");
    for u in [&a, &b] {
        let rec = env.state.users.get_user(u.user_id).await.unwrap().expect("exists");
        assert_eq!(rec.username, u.username);
    }
}
```

Update the existing `seed_user_builder_*` tests to the new API (`new()`
arg-less, `.seed()` returns `SeededUser` → read `.user_id`/`.username`).

- [ ] **Step 2b: Change the fixtures.**
  - `test_support.rs`: add `static SEED_SEQ: AtomicU64 = AtomicU64::new(0);` and
    `use std::sync::atomic::{AtomicU64, Ordering};`. Add `SeededUser`.
    `SeedUser` fields become `password`/`display_name`/`is_operator` (drop
    `username`). **Do NOT `#[derive(Default)]`** — `&str: Default` is `""`,
    which would silently wipe the `"password123"` default and break the
    default-password tests; hand-write
    `pub fn new() -> Self { Self { password: "password123", display_name: None, is_operator: false } }`.
    `seed()` computes
    `let username = { let n = SEED_SEQ.fetch_add(1, Ordering::Relaxed); parse_username(&format!("user{n}")) };`,
    calls `create_user`, returns `SeededUser { user_id, username }`. Doc-comment
    the fresh-DB-per-test invariant + loud-failure property (spec §Correctness
    invariant).
  - `helpers/mod.rs`: add `username: Username` to `SeededSession` (+
    `use common::username::Username;`). `create_user_and_session`: seed via
    `SeedUser::new().seed(state)` → build
    `SeededSession { user_id: u.user_id, username: u.username, token }`
    **directly** (no `create_session_for`). `create_operator_and_session`: same
    via `.operator()`. `create_session_for`: after `create_session`,
    `let username = state.users.get_user(user_id).await .expect("user").expect("exists").username;`
    → `SeededSession { user_id, username, token }`.

- [ ] **Step 2c: Sweep all call sites** (delegate per file-group to subagents,
      **jaunder-dispatch** — each brief carries the transformation rules; they
      edit mechanically, no per-file compile since the tree is broken until all
      land):
  - **Drop the username literal:** `SeedUser::new("x")` → `SeedUser::new()`;
    `create_user_and_session(&state, "x")` → `create_user_and_session(&state)`;
    same for `create_operator_and_session`.
  - **`seed()` now returns `SeededUser`:**
    `let user_id = SeedUser::new("x") .seed(&state).await;` → if only the id is
    used, `let user_id = SeedUser::new() .seed(&state).await.user_id;`; if the
    name is ALSO used, bind the struct:
    `let user = SeedUser::new().seed(&state).await;` then `user.user_id` /
    `user.username`.
  - **Name-referenced sites → `.username`:**
    `/~alice/`→`format!("/~{}/…", s.username)`;
    `get_user_by_username(&username("alice"))` →
    `get_user_by_username(&s.username)`; `assert_eq!(rec.username, "alice")` →
    `assert_eq!(rec.username, s.username)`; a substring check uses
    `s.username.as_str()`. AtomPub: thread one `let name = &session.username;`
    when a test issues 2+ requests (seed + URI +
    `atompub_authed(.., name, ..)`).
  - **Mirror bindings:** inline single-use
    `let x = <fixture>.user_id/.token/ .cookie();`. **Keep bound** when the
    struct feeds 2+ reads (id+name), is used 3+ times, or wraps a side-effect.
  - **Wrapper dedup:** delete `web_account::operator_cookie` (→
    `create_operator_and_session(&state).cookie()` at its 7 callers),
    `web_media::authed_cookie` + the un-helpered duplicate in
    `media_handlers.rs`, `audiences`/`web_subscriptions`
    `make_user`/`cookie_for` → shared fixtures.
  - **Leave literal (carve-outs):** `atompub_service.rs` `"mallory"`,
    `feed_handlers.rs` `"charlie"` (non-seeded mismatch names); the #626 bespoke
    error-path/invite/label tests; `web_auth`/`web_email` wire-body payloads;
    the 3 raw-SQL `WHERE username='testuser'` sites in `storage/src/users.rs`
    (or `format!` the seeded `.username` if seeded via `SeedUser`).

- [ ] **Step 2d: Integrate + compile.**
      `devtool run -- cargo xtask check --no-test`. Fix stragglers (missed
      sites, `.user_id`/`.username` typing, unused imports). Iterate to green.
      Then `rg 'SeedUser::new\("|create_user_and_session\(&?\w+,\s*"'` to
      confirm no seed passes a literal (allowlist per spec AC1).

- [ ] **Step 2e: Full gate + commit.** `devtool run -- cargo xtask check` green,
      then commit:

```bash
git add -A
git commit -m "test: autogenerate seed usernames; expose .username; inline fixture noise (#635)"
```

---

### Task 3: Verification

**Files:** any straggler surfaced.

- [ ] **Step 1: Acceptance greps** (spec §Acceptance):
  - `rg -n 'SeedUser::new\("' server storage -g '*.rs'` → empty (arg-less).
  - `rg -n 'create_user_and_session\(&\w+,|create_operator_and_session\(&\w+,' server -g '*.rs'`
    → empty.
  - Surviving username string literals only in the enumerated allowlist
    (mallory, charlie, bespoke carve-outs, wire bodies, raw-SQL).
  - `rg -n 'let \w+ = \w+\.(user_id|token|cookie\(\));' server -g '*.rs'` →
    manually confirm each remaining is multi-use / id+name, not single-use
    noise.
- [ ] **Step 2: Full gate.** `devtool run -- cargo xtask validate --no-e2e`
      (test-only change can't reach the e2e surface; CI runs the e2e matrix).
      Green.
- [ ] **Step 3: Diff scope.** `git diff wt-base-issue-635..HEAD --stat` — only
      `#[cfg(test)]` modules + `test_support.rs`; spot-check no non-test path
      changed.
- [ ] **Step 4:** commit any straggler fix.

## Self-review

- Spec coverage: AC1→2c/3; AC2→2b/2c; AC3→2c mirror rules; AC4→2c wrapper dedup;
  AC5→3; AC6→2a docs + coverage gate. Out-of-scope→Task 1.
- Type consistency: `SeededUser{user_id,username}` /
  `SeededSession{user_id, username,token}` /
  `SeedUser::{new,password,display_name,operator,seed}` used identically across
  2a/2b/2c.
