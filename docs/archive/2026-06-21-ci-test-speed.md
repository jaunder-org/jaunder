# CI Test-Speed Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut the non-e2e CI gate wall-clock by attacking instrumented build/link time (consolidate 25 integration-test binaries → 5) and the few genuinely slow tests (seed pagination posts directly; cheap test-only Argon2).

**Architecture:** Three independent workstreams, landed in order of payoff/safety: (1) move `server/tests/*.rs` into 5 domain subdirectories each with a single `main.rs` module aggregator; (2) add a `seed_posts` test helper that bypasses the HTTP/server-fn path; (3) add a `cheap-kdf` cargo feature on `common` that lowers Argon2 cost in test builds only, locked out of production by resolver-v2 dev-dep isolation plus a runtime fail-closed guard.

**Tech Stack:** Rust (edition 2021, resolver 2), cargo-nextest, cargo-llvm-cov, argon2 crate, `cargo xtask` gate, ephemeral PostgreSQL via `scripts/with-ephemeral-postgres`.

## Global Constraints

- **Branch:** Do this work on a dedicated branch off `main` (`ci-test-speed`); never commit on `main`. Never `git push` without explicit approval.
- **Verify before commit:** Run the relevant gate (`cargo xtask check --no-test` or `cargo xtask validate --no-e2e`) before each commit; one clean verified commit per task. Commit `.rs` changes *before* running `cargo xtask check` (the coverage classifier diffs `HEAD`; uncommitted line-shifting edits manufacture phantom regressions), then let `check` heal manifests and amend.
- **Stable TMPDIR for measurement:** Any profiling/test run launched via context-mode must `export TMPDIR=/var/tmp/jaunder-prof` (or similar persistent dir); the sandbox's ephemeral `/tmp/.ctx-mode-*` breaks `tempfile`-based tests.
- **Backend parity:** Storage-backed tests run on both SQLite and PostgreSQL in one nextest pass; do not drop a backend.
- **Coverage ratchet:** Never lower `coverage-baseline.json` / `crap-manifest.json` without explicit user approval. `cargo xtask check` auto-heals only improvements.
- **Security (workstream 3):** A release/production `jaunder` binary must never use cheap Argon2 params. This is enforced by feature isolation **and** a runtime guard; both must be present.
- **Test discovery invariant:** After any consolidation task, `cargo nextest list -p jaunder` must report the **same total test count** as before (no tests silently lost).
- **Run via the alias:** Invoke the gate as `cargo xtask …` (bare; no trailing `; echo`, `| tee`, `2>&1` that would mask the exit code).
- **No compound/loop Bash:** This sandbox denies `for … do … done` loops, `{ … }` blocks, `sed`, and piped `grep`/`head`/`tail`. Run each `git mv` as its own single-line command; use the Grep tool (not bash grep) to search and Edit/Write (not sed) to edit. The `for`-loops shown in some tasks below are illustrative — expand them into individual commands.

---

## Baseline (measured 2026-06-21, deps cached, stable TMPDIR)

- Coverage step ≈ **~250s instrumented build/link + ~76s execution** (1530 tests, all passing).
- Slow tests (nextest SLOW report): 4 `web_posts` pagination tests > 10s; `storage` auth tests `set_password_authenticate_with_old_returns_invalid_and_new_succeeds`, `use_invite_with_valid_code_marks_it_used`, `use_password_reset_already_used_returns_already_used` > 5s; `backup_worker_executes_scheduled_backup`, 2 `media_manager` upload tests > 5s.

Record fresh before/after numbers as you go (Task 10).

---

### Task 0: Create working branch

**Files:** none (git only).

- [ ] **Step 1: Branch off main**

```bash
git fetch origin
git switch -c ci-test-speed origin/main
```
Expected: on a new branch `ci-test-speed` with a clean tree.

- [ ] **Step 2: Confirm baseline test count**

Run: `cargo nextest list -p jaunder 2>/dev/null | tail -1`
Expected: a line like `... <N> tests` — **record N**; every consolidation task must preserve it.

---

## Workstream 1 — Consolidate integration-test binaries (25 → 5)

**Shared mechanics (apply in every Task 1.x):**

For a group `G` containing files `f1.rs … fk.rs`:

1. `git mv server/tests/f1.rs server/tests/G/f1.rs` for each file (creates `server/tests/G/`).
2. Create `server/tests/G/main.rs`:
   ```rust
   #![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
   #[path = "../helpers/mod.rs"]
   mod helpers;

   mod f1;
   mod f2;
   // … one `mod` line per moved file (filename without .rs)
   ```
3. In each moved file, delete its own `mod helpers;` line (the aggregator owns it now), and rewrite helper references from crate-root to the aggregator path:
   - `use helpers::{…}` → `use crate::helpers::{…}`
   - any bare `helpers::X` → `crate::helpers::X`
   Leave each file's `#![allow(...)]` inner attributes intact (valid at module top).

**Why this is collision-free:** each file stays its own `mod`, so reused top-level names (`post_form`, `make_app`, `body_string`, `make_user`, `cookie_for`, …) live in distinct module paths and never clash. Do **not** flatten files into one module.

**Per-task verification (same for every group):**
- `cargo nextest list -p jaunder` total count == N (from Task 0 Step 2).
- `scripts/with-ephemeral-postgres cargo nextest run -p jaunder --no-fail-fast` → all pass.
- Commit.

### Task 1.1: `storage` binary (heaviest, isolate first)

**Files:**
- Move: `server/tests/storage.rs` → `server/tests/storage/storage.rs`
- Create: `server/tests/storage/main.rs`

- [ ] **Step 1: Move file and create aggregator**

```bash
mkdir -p server/tests/storage
git mv server/tests/storage.rs server/tests/storage/storage.rs
```
Then create `server/tests/storage/main.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
#[path = "../helpers/mod.rs"]
mod helpers;

mod storage;
```

- [ ] **Step 2: Rewire helpers in `server/tests/storage/storage.rs`**

Delete the `mod helpers;` line. Change `use helpers::{…}` → `use crate::helpers::{…}` and any bare `helpers::` → `crate::helpers::`.

- [ ] **Step 3: Verify discovery count unchanged**

Run: `cargo nextest list -p jaunder 2>/dev/null | tail -1`
Expected: total tests == N.

- [ ] **Step 4: Verify tests pass**

Run: `scripts/with-ephemeral-postgres cargo nextest run -p jaunder --no-fail-fast`
Expected: 0 failed.

- [ ] **Step 5: Commit**

```bash
git add -A server/tests
git commit -m "refactor(test): consolidate storage integration tests into one binary"
```

### Task 1.2: `web` binary (10 files)

> **Note:** `web_audiences.rs` and `web_subscriptions.rs` are NOT on `main` — they belong to the unmerged audience work on the `visibility` branch. They are intentionally excluded here. When `visibility` later merges, fold those two in: `git mv` each into `server/tests/web/` and add a `mod web_audiences;` / `mod web_subscriptions;` line to `server/tests/web/main.rs`.

**Files:**
- Move into `server/tests/web/`: `web_posts.rs`, `web_auth.rs`, `web_account.rs`, `web_backup.rs`, `web_media.rs`, `web_password_reset.rs`, `web_sessions.rs`, `web_site.rs`, `web_email.rs`, `web_tags.rs`
- Create: `server/tests/web/main.rs`

- [ ] **Step 1: Move files**

```bash
mkdir -p server/tests/web
for f in web_posts web_auth web_account web_backup web_media \
         web_password_reset web_sessions web_site web_email web_tags; do
  git mv "server/tests/$f.rs" "server/tests/web/$f.rs"
done
```

- [ ] **Step 2: Create `server/tests/web/main.rs`**

```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
#[path = "../helpers/mod.rs"]
mod helpers;

mod web_account;
mod web_auth;
mod web_backup;
mod web_email;
mod web_media;
mod web_password_reset;
mod web_posts;
mod web_sessions;
mod web_site;
mod web_tags;
```

- [ ] **Step 3: Rewire helpers in each moved file**

For every file in `server/tests/web/`, delete `mod helpers;`, and change `use helpers::{…}` → `use crate::helpers::{…}`, bare `helpers::` → `crate::helpers::`.

- [ ] **Step 4: Verify count + pass**

Run: `cargo nextest list -p jaunder 2>/dev/null | tail -1` (== N), then
`scripts/with-ephemeral-postgres cargo nextest run -p jaunder --no-fail-fast` (0 failed).

- [ ] **Step 5: Commit**

```bash
git add -A server/tests
git commit -m "refactor(test): consolidate web_* integration tests into one binary"
```

### Task 1.3: `atompub` binary

**Files:** move `atompub_posts.rs`, `atompub_media.rs`, `atompub_rsd.rs`, `atompub_service.rs` → `server/tests/atompub/`; create `server/tests/atompub/main.rs`.

- [ ] **Step 1: Move + aggregator**

```bash
mkdir -p server/tests/atompub
git mv server/tests/atompub_posts.rs   server/tests/atompub/atompub_posts.rs
git mv server/tests/atompub_media.rs   server/tests/atompub/atompub_media.rs
git mv server/tests/atompub_rsd.rs     server/tests/atompub/atompub_rsd.rs
git mv server/tests/atompub_service.rs server/tests/atompub/atompub_service.rs
```
(Run each `git mv` as a separate command — no `for`-loop.)
`server/tests/atompub/main.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
#[path = "../helpers/mod.rs"]
mod helpers;

mod atompub_media;
mod atompub_posts;
mod atompub_rsd;
mod atompub_service;
```

- [ ] **Step 2: Rewire helpers** in each moved file (as in Task 1.2 Step 3).

- [ ] **Step 3: Verify count + pass** (`== N`; 0 failed via ephemeral-postgres run).

- [ ] **Step 4: Commit**

```bash
git add -A server/tests
git commit -m "refactor(test): consolidate atompub integration tests into one binary"
```

### Task 1.4: `feed` binary

**Files:** move `feed_worker.rs`, `feed_events_hook.rs`, `feed_handlers.rs`, `feed_regenerate.rs` → `server/tests/feed/`; create `server/tests/feed/main.rs`.

- [ ] **Step 1: Move + aggregator**

```bash
mkdir -p server/tests/feed
git mv server/tests/feed_worker.rs      server/tests/feed/feed_worker.rs
git mv server/tests/feed_events_hook.rs server/tests/feed/feed_events_hook.rs
git mv server/tests/feed_handlers.rs    server/tests/feed/feed_handlers.rs
git mv server/tests/feed_regenerate.rs  server/tests/feed/feed_regenerate.rs
```
(Run each `git mv` as a separate command — no `for`-loop.)
`server/tests/feed/main.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
#[path = "../helpers/mod.rs"]
mod helpers;

mod feed_events_hook;
mod feed_handlers;
mod feed_regenerate;
mod feed_worker;
```

- [ ] **Step 2: Rewire helpers** in each moved file.

- [ ] **Step 3: Verify count + pass** (`== N`; 0 failed).

- [ ] **Step 4: Commit**

```bash
git add -A server/tests
git commit -m "refactor(test): consolidate feed integration tests into one binary"
```

### Task 1.5: `misc` binary

**Files:** move `commands.rs`, `backup_interop.rs`, `media_handlers.rs`, `static_assets.rs` → `server/tests/misc/`; create `server/tests/misc/main.rs`.

- [ ] **Step 1: Move + aggregator**

```bash
mkdir -p server/tests/misc
git mv server/tests/commands.rs       server/tests/misc/commands.rs
git mv server/tests/backup_interop.rs server/tests/misc/backup_interop.rs
git mv server/tests/media_handlers.rs server/tests/misc/media_handlers.rs
git mv server/tests/static_assets.rs  server/tests/misc/static_assets.rs
```
(Run each `git mv` as a separate command — no `for`-loop.)
`server/tests/misc/main.rs`:
```rust
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]
#[path = "../helpers/mod.rs"]
mod helpers;

mod backup_interop;
mod commands;
mod media_handlers;
mod static_assets;
```

- [ ] **Step 2: Rewire helpers** in each moved file. Note: `commands.rs` and `backup_interop.rs` both define `populate_backup_fixture`/`assert_backup_fixture_restored` — fine, they stay in separate modules.

- [ ] **Step 3: Confirm `server/tests/` now contains only the 5 dirs + `helpers/`**

Run: `ls server/tests`
Expected: `atompub  feed  helpers  misc  storage  web` (no stray `*.rs`).

- [ ] **Step 4: Verify count + pass** (`== N`; 0 failed).

- [ ] **Step 5: Commit**

```bash
git add -A server/tests
git commit -m "refactor(test): consolidate remaining integration tests into misc binary"
```

### Task 1.6: Gate + measure consolidation win

- [ ] **Step 1: Full non-e2e gate**

Run: `cargo xtask check --no-test`
Then commit any healed manifests if `check` reports improvements:
```bash
git commit -am "chore(coverage): heal manifests after test consolidation" || true
```

- [ ] **Step 2: Record build-time delta** (see Task 10 harness). Expect the ~250s instrumented build to drop materially (5 links vs 25).

---

## Workstream 2 — Seed pagination posts directly

### Task 2: `seed_posts` helper + refactor the 4 slow pagination tests

**Files:**
- Modify: `server/tests/helpers/mod.rs` (add helper)
- Modify: `server/tests/web/web_posts.rs` (4 tests: `get_post_finds_author_draft_across_multiple_pages`, `list_home_feed_returns_authenticated_users_published_posts_only`, `list_local_timeline_returns_published_posts_with_cursor_pagination`, `list_user_posts_returns_published_posts_with_cursor_pagination`)

**Interfaces:**
- Consumes: `storage::post_service::create_rendered_post(storage: &dyn PostStorage, user_id: i64, title: Option<String>, slug: Slug, body: String, format: PostFormat, published_at: Option<DateTime<Utc>>, summary: Option<String>, audiences: Vec<AudienceTarget>) -> Result<i64, CreatePostError>`
- Produces: `pub async fn seed_posts(state: &Arc<storage::AppState>, user_id: i64, count: usize, published: bool) -> Vec<i64>`

- [ ] **Step 1: Confirm the `&dyn PostStorage` accessor**

Read `web/src/posts/server.rs` (the `CreatePost` handler) and `storage/src/lib.rs` (`AppState`) to see exactly how the production handler obtains a `&dyn PostStorage` from `AppState` (e.g. `&*state.posts` or `state.posts.storage()`), and the exact `Slug`/`AudienceTarget`/`PostFormat` import paths. Use that same accessor in Step 2.
Expected: a one-line accessor expression and the type import paths.

- [ ] **Step 2: Add `seed_posts` to `server/tests/helpers/mod.rs`**

```rust
/// Seeds `count` posts for `user_id` directly through the storage service,
/// bypassing the HTTP/server-fn path (markdown render of trivial bodies is
/// negligible; the cost we avoid is axum routing + server_fn per call).
/// `published == true` sets `published_at = now` so list/timeline endpoints
/// return them; `false` leaves them as drafts. Returns ids in creation order.
pub async fn seed_posts(
    state: &std::sync::Arc<storage::AppState>,
    user_id: i64,
    count: usize,
    published: bool,
) -> Vec<i64> {
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let published_at = if published { Some(chrono::Utc::now()) } else { None };
        let id = storage::post_service::create_rendered_post(
            /* &dyn PostStorage from state — accessor confirmed in Step 1 */,
            user_id,
            None,
            format!("seed-{i}").parse().expect("valid slug"),
            format!("# Post {i}\n\nbody"),
            storage::PostFormat::Markdown,
            published_at,
            None,
            Vec::new(),
        )
        .await
        .expect("seed post should be created");
        ids.push(id);
    }
    ids
}
```

- [ ] **Step 3: Refactor `get_post_finds_author_draft_across_multiple_pages`**

Replace the `for i in 0..55 { create_post_json(...) }` loop with:
```rust
let ids = crate::helpers::seed_posts(&state, author_id, 55, false).await;
let first_post_id = ids[0];
```
(Keep the subsequent `get_post_by_id` / `get_post_form` assertions unchanged.)

- [ ] **Step 4: Refactor the 3 timeline/list tests**

In `list_home_feed_returns_authenticated_users_published_posts_only`, `list_local_timeline_returns_published_posts_with_cursor_pagination`, and `list_user_posts_returns_published_posts_with_cursor_pagination`, replace each `for i in 0..N { create_post_json(..., publish=true, ...) }` seeding loop with `crate::helpers::seed_posts(&state, <author_id>, N, true).await;`, preserving each test's existing post count `N` and its list/pagination assertions.

- [ ] **Step 5: Verify the 4 tests pass and are faster**

Run: `scripts/with-ephemeral-postgres cargo nextest run -p jaunder web_posts -- --no-fail-fast`
Expected: all pass; the 4 previously-SLOW tests no longer appear in the `SLOW [>5s]` report.

- [ ] **Step 6: Commit**

```bash
git add -A server/tests
git commit -m "test(web_posts): seed pagination posts via storage, not the HTTP path"
```

---

## Workstream 3 — Test-only cheap Argon2 (Path A), locked out of production

### Task 3: Add the `cheap-kdf` feature and a build-truth constant

**Files:**
- Modify: `common/Cargo.toml`
- Modify: `common/src/lib.rs`

**Interfaces:**
- Produces: `common::CHEAP_KDF_ENABLED: bool` (compile-time truth of the feature for downstream runtime guards).

- [ ] **Step 1: Add the feature (non-default; implied by `test-utils`)**

In `common/Cargo.toml`, change the `[features]` block to:
```toml
[features]
test-utils = ["cheap-kdf"]
cheap-kdf = []
metrics = ["dep:opentelemetry"]
```

- [ ] **Step 2: Export the build-truth constant + release tripwire**

In `common/src/lib.rs`, add near the top:
```rust
/// True only when the test-only cheap Argon2 parameters are compiled in.
/// Production builds (no `cheap-kdf`) leave this `false`; downstream binaries
/// assert on it at startup as a fail-closed guard.
pub const CHEAP_KDF_ENABLED: bool = cfg!(feature = "cheap-kdf");

// A release/optimized build must never carry the cheap KDF params. Test builds
// (debug_assertions on) are unaffected; an optimized build with the feature on
// fails to compile here rather than producing a weak-hashing artifact.
#[cfg(all(feature = "cheap-kdf", not(debug_assertions)))]
compile_error!("cheap-kdf must not be enabled in a release/optimized build");
```

- [ ] **Step 3: Build both ways to prove the gate**

Run: `cargo build -p common` (Expected: OK, feature off)
Run: `cargo build -p common --features cheap-kdf` (Expected: OK — debug build allows it)
Run: `cargo build -p common --features cheap-kdf --release` (Expected: FAIL with `cheap-kdf must not be enabled in a release/optimized build`)

- [ ] **Step 4: Commit**

```bash
git add common/Cargo.toml common/src/lib.rs
git commit -m "feat(common): add test-only cheap-kdf feature with release tripwire"
```

### Task 4: Lower Argon2 cost under `cheap-kdf`, preserving a production-params test

**Files:**
- Modify: `common/src/password.rs`

**Interfaces:**
- Consumes: `common::CHEAP_KDF_ENABLED` (indirectly, via cfg).
- Produces: unchanged public API — `Password::hash()` / `Password::verify()` keep their signatures.

- [ ] **Step 1: Write the failing production-params test**

Add to `common/src/password.rs` `mod tests`:
```rust
#[test]
fn production_params_roundtrip_regardless_of_feature() {
    // Guards prod-strength Argon2 even when the workspace test build turns on
    // cheap-kdf: hash with explicit production params and verify.
    use argon2::{password_hash::{rand_core::OsRng, SaltString}, Argon2, PasswordHasher};
    let p: Password = "a".repeat(10).parse().unwrap();
    let salt = SaltString::generate(&mut OsRng);
    let prod_hash = Argon2::default()
        .hash_password(p.as_str().as_bytes(), &salt)
        .unwrap()
        .to_string();
    assert!(prod_hash.contains("m=19456"), "prod params must be Argon2 default");
    assert!(p.verify(&prod_hash).unwrap());
}
```

- [ ] **Step 2: Run it (passes today; locks current behavior before refactor)**

Run: `cargo test -p common production_params_roundtrip_regardless_of_feature`
Expected: PASS. (This test must keep passing through the refactor.)

- [ ] **Step 3: Refactor `hash()` to a params seam**

Replace `Password::hash` in `common/src/password.rs` with:
```rust
pub fn hash(&self) -> Result<String, PasswordError> {
    use argon2::{password_hash::{rand_core::OsRng, SaltString}, PasswordHasher};
    let salt = SaltString::generate(&mut OsRng);
    Self::hasher()
        .hash_password(self.0.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| PasswordError::HashingFailed(e.to_string()))
}

/// Argon2 configuration for hashing. Production uses the crate defaults
/// (m=19456, t=2). Under `cheap-kdf` (test builds only) it uses the minimum
/// memory cost so the suite is not dominated by KDF time. `verify()` derives
/// cost from the stored hash, so it needs no branch.
fn hasher() -> argon2::Argon2<'static> {
    #[cfg(feature = "cheap-kdf")]
    {
        use argon2::{Algorithm, Argon2, Params, Version};
        let params = Params::new(Params::MIN_M_COST, 1, 1, None)
            .expect("valid cheap argon2 params");
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
    }
    #[cfg(not(feature = "cheap-kdf"))]
    {
        argon2::Argon2::default()
    }
}
```
Leave `verify()` unchanged (cost comes from the PHC hash string).

- [ ] **Step 4: Verify both the prod-params test and the roundtrip tests pass, two ways**

Run: `cargo test -p common` (Expected: PASS — default, prod params)
Run: `cargo test -p common --features cheap-kdf` (Expected: PASS — `production_params_roundtrip…` still uses explicit defaults; `hash_and_verify_roundtrip` now cheap)

- [ ] **Step 5: Commit**

```bash
git add common/src/password.rs
git commit -m "feat(common): use cheap Argon2 params under cheap-kdf in test builds"
```

### Task 5: Runtime fail-closed guard in the server binary

**Files:**
- Modify: `server/src/main.rs`

- [ ] **Step 1: Add the guard as the first action in `main()`**

In `server/src/main.rs`, at the very top of `async fn main()` (before `Cli::parse()`):
```rust
// Fail-closed: a production binary must never link a `common` compiled with
// cheap test KDF params. Feature isolation (resolver 2, dev-deps only) keeps
// this false in production; if it is ever true, refuse to start rather than
// hash passwords weakly. main() is never run by the integration tests, so this
// does not affect the test build.
if common::CHEAP_KDF_ENABLED {
    eprintln!("FATAL: jaunder built with cheap-kdf (test-only password hashing); refusing to start");
    std::process::exit(1);
}
```

- [ ] **Step 2: Verify production build is unaffected and guard compiles**

Run: `cargo build -p jaunder` (Expected: OK)
Run: `cargo build -p jaunder --features common/cheap-kdf` (Expected: OK in debug; the binary would exit at startup if run)

- [ ] **Step 3: Commit**

```bash
git add server/src/main.rs
git commit -m "feat(server): fail-closed guard against cheap-kdf in the binary"
```

### Task 6: Confirm the slow auth tests now run cheap; scan for param assertions

**Files:**
- Inspect: `server/tests/storage/storage.rs`, `storage/src/helpers.rs`

- [ ] **Step 1: Scan for any test asserting literal default params**

Run: `cargo nextest list -p jaunder >/dev/null` then search the test tree:
```bash
git grep -n "19456" -- 'server/tests' 'storage/src' 'common/src'
```
Expected: hits only in (a) `common/src/password.rs` `production_params_roundtrip…` (intended) and (b) `storage/src/helpers.rs` `DUMMY_PASSWORD_HASH_FALLBACK` (a fallback constant, never asserted against under cheap-kdf because `dummy_password_hash()` recomputes via `Password::hash()`). If any *other* test asserts `m=19456`, adjust it to derive the expected params from `Password::hash()` instead of a literal.

- [ ] **Step 2: Verify the named slow auth tests pass and are no longer slow**

Run: `scripts/with-ephemeral-postgres cargo nextest run -p jaunder set_password_authenticate_with_old use_invite_with_valid_code use_password_reset_already_used`
Expected: all pass; none appear in `SLOW [>5s]`.

- [ ] **Step 3: Verify the §2.1 timing-parity tests still pass**

Run: `cargo nextest run -p storage dummy_password_hash`
Expected: `dummy_password_hash_is_a_valid_verifiable_hash` and `dummy_password_hash_matches_real_hash_parameters` PASS (both real and dummy hashes now use cheap params, so parity holds).

- [ ] **Step 4: Commit (if Step 1 required edits; else skip)**

```bash
git add -A && git commit -m "test: derive expected argon2 params instead of hardcoding under cheap-kdf"
```

---

## Task 10: Re-measure and full gate

**Files:** none (verification).

- [ ] **Step 1: Re-profile the coverage step**

Using a stable TMPDIR, time the instrumented build and execution separately (mirror the 2026-06-21 method: `cargo llvm-cov clean --workspace`; time `cargo llvm-cov nextest --no-run`; then `cargo llvm-cov clean --profraw-only` and time `scripts/with-ephemeral-postgres cargo llvm-cov --no-report nextest --show-progress none`; read the nextest `Summary [..s]` line for execution wall-clock).
Expected vs baseline: build materially below ~250s; execution below ~76s with the named SLOW tests gone.

- [ ] **Step 2: Full CI-faithful gate**

Run: `cargo xtask validate`
Expected: exit 0 (`xtask-done: … ok=true`). Read `.xtask/last-result.json` `steps[]` to confirm `coverage` is `clean` and e2e passed.

- [ ] **Step 3: Record results in the spec**

Append measured before/after numbers to `docs/superpowers/specs/2026-06-21-ci-test-speed-design.md` (Verification section).

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "docs: record ci-test-speed measured results"
```

---

## Self-review notes (addressed)

- **Spec coverage:** consolidation → Tasks 1.1–1.6; seed-direct → Task 2; KDF Path A (feature, params, prod lock, preserved real-KDF test, §2.1 parity, param-assertion pre-check) → Tasks 3–6. Non-goals (overlap dedup, e2e instrumentation, baseline lowering) untouched.
- **One confirmed-at-implementation detail:** the `&dyn PostStorage` accessor from `AppState` (Task 2 Step 1) — a targeted read, not a placeholder.
- **Type consistency:** `seed_posts(&Arc<storage::AppState>, i64, usize, bool) -> Vec<i64>` used identically in helper and call sites; `common::CHEAP_KDF_ENABLED: bool` defined in Task 3, consumed in Task 5; `Password::hash` signature unchanged across Task 4.
