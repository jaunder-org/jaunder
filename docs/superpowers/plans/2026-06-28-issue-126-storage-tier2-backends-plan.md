# Storage `test_support` + Dual-Backend Contract Tests — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use `- [ ]` checkboxes.

**Goal:** Relocate the test harness from the standalone `db-test-harness` crate into `storage` as a feature-gated `test_support` module (deleting `db-test-harness` entirely), migrate `server` onto it, then convert the 45 storage-crate contract tests to dual-backend in place.

**Architecture:** Hosting the harness inside `storage` means `storage`'s own `#[cfg(test)]` tests get `Backend::setup()`/`TestEnv` from the *same crate instance* as the code under test — eliminating the dev-dependency-cycle's two-`storage`-instances `E0308`. `server` (a separate test crate) consumes it via `storage`'s `test-support` feature.

**Tech Stack:** Rust 2021, `rstest`/`rstest_reuse`, `sqlx`, feature-gated test-support module.

## Global Constraints

- No `Co-Authored-By`. Commit style `type(scope): subject`, scope `issue-126`.
- Per-task gate: `cargo xtask check --no-test`. Final: `cargo xtask validate`.
- The relocation (Task 1) is **behavior-preserving**: the full `server` suite stays green on both backends, with no committed `server` test-body changes beyond the `helpers` import swap.
- **No trace of `db-test-harness`** in *code + Cargo manifests* when done: `rg db.test.harness -g '!docs/**' -g '!*.json'` returns nothing (docs that *explain* the supersession — ADR-0033, the archived #125 docs, these planning docs — legitimately still name it; that's expected); no workspace member; no `Cargo.lock` entry; `cargo metadata` lists no such package. The generated `crap-manifest.json` still lists the deleted crate's functions until the coverage gate **heals** it (drops them) on the Task 1 commit — stage the healed manifest.
- The `::postgres` cases run only under the coverage pass; verify with `cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p <pkg> <filter>`.
- Branch `worktree-issue-126-storage-tier2-backends`. Follow-ups #135 / #136 already filed.

---

### Task 1: Relocate the harness into `storage::test_support` (atomic; behavior-preserving)

The workspace only compiles once the move is complete (the old crate gone, `server` rewired, the module present), so this is one commit.

**Files:**
- Create: `storage/src/test_support.rs` (the moved harness)
- Modify: `storage/src/lib.rs` (declare the module), `storage/Cargo.toml` (feature + deps)
- Modify: `server/Cargo.toml` (dev-deps), `server/tests/helpers/mod.rs` (re-export source)
- Delete: `db-test-harness/` (whole dir), `Cargo.toml` workspace member, `Cargo.lock` entry

- [ ] **Step 1: Move the harness file**

`git mv db-test-harness/src/lib.rs storage/src/test_support.rs`. Then transform it:
- Remove the crate-level `//!` doc and the `#![allow(clippy::unwrap_used, clippy::expect_used)]` *crate* attribute; re-add the allow as a **module-level inner attribute** at the top of `test_support.rs`: `#![allow(clippy::unwrap_used, clippy::expect_used)]` (the user has approved these two for the harness). Keep the existing `// cov:ignore` markers and the `#[cfg(test)] mod tests` (pure-fn tests).
- Replace every `storage::` path with `crate::` (the harness now lives *in* `storage`): `use storage::{…}` → `use crate::{…}`; `storage::create_rendered_post` → `crate::create_rendered_post`; `storage::PostFormat` → `crate::PostFormat`.
- Add `seed_user` (it was never committed; add it here) next to `seed_posts`:

```rust
/// Creates a throwaway user and returns its id, for tests that need a user to
/// exist before exercising a per-user handle (replaces raw `INSERT INTO users`).
///
/// # Panics
///
/// If the username/password fail to parse or the user cannot be created.
pub async fn seed_user(state: &std::sync::Arc<crate::AppState>) -> i64 {
    state
        .users
        .create_user(
            &"testuser".parse().expect("valid username"),
            &"password123".parse().expect("valid password"),
            None,
            false,
        )
        .await
        .expect("seed user should be created")
}
```

and a smoke test in the module's `mod tests`:

```rust
#[tokio::test]
async fn seed_user_creates_a_user() {
    let env = super::Backend::Sqlite.setup().await;
    let id = super::seed_user(&env.state).await;
    assert!(id > 0);
}
```

- [ ] **Step 2: Declare the module in `storage/src/lib.rs`**

```rust
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
```

(`any(test, feature)` so `storage`'s own tests see it via `cfg(test)` and `server` via the feature. The module-level unwrap/expect allow covers the harness; `clippy.toml`'s `allow-unwrap-in-tests` covers `storage`'s own test build. If the gate still denies `unwrap_used` in the feature-but-not-test path, fall back to splitting into `#[cfg(test)] mod test_support;` + `#[cfg(all(not(test), feature = "test-support"))] pub mod test_support;` — but try the single form first.)

- [ ] **Step 3: `storage/Cargo.toml` — feature + deps**

Add `test-support` as a **new** feature **alongside** the existing `test-utils`. **Do NOT remove `test-utils`** — it is load-bearing: `storage/src/helpers.rs:347` gates the ADR-0026 `hash_password` fault-injection hook on `#[cfg(any(test, feature = "test-utils"))]`, and `web/Cargo.toml:58` + `server/Cargo.toml:65` enable it. **Do NOT touch `web/`.**

```toml
[dependencies]
# … existing …
tempfile = { workspace = true, optional = true }
rstest_reuse = { workspace = true, optional = true }

[dev-dependencies]
tempfile.workspace = true        # already present
rstest.workspace = true          # for the Task 3 converted tests (apply site)
rstest_reuse.workspace = true
# (existing) tokio = { workspace = true, features = ["macros", "rt"] }  — unchanged

[features]
test-utils = []                                      # KEEP — gates the ADR-0026 hook
test-support = ["dep:tempfile", "dep:rstest_reuse"]  # the harness module's optional deps
```

Notes: `rstest` is **not** a feature dep — `test_support` only *defines* `#[template]`s (which need `rstest_reuse`, not `rstest`); `rstest` is needed only at the *apply* site (the converted tests), so it stays a plain dev-dep. The existing `tokio` regular dep is `["full"]` via the workspace, so the macros/rt are already present (no new tokio dev-dep). `sqlx`/`common`/`chrono` are already deps.

- [ ] **Step 4: Migrate `server`**

`server/Cargo.toml` dev-deps: drop `db-test-harness = …`; change `storage = { workspace = true, features = ["test-utils"] }` → `features = ["test-utils", "test-support"]` (KEEP `test-utils` — dropping it de-gates the ADR-0026 hook server's hash-error tests need).
`server/tests/helpers/mod.rs`: change the re-export `pub use db_test_harness::{ … };` → `pub use storage::test_support::{ … };` (same item list). Everything else in `helpers` is unchanged.

- [ ] **Step 5: Delete the crate**

```bash
git rm -r db-test-harness
```
Remove the `"db-test-harness",` line from `Cargo.toml` `members`. (Cargo.lock updates on the next build.)

- [ ] **Step 6: Verify the relocation builds + server suite green on both backends**

```bash
cargo build --workspace
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p jaunder
```
Expected: workspace builds; full `jaunder` suite passes on both backends (same counts as before — behavior-preserving). Also run the `storage` `test_support` unit tests: they appear under `-p storage`.

- [ ] **Step 7: Confirm no trace + gate + commit**

```bash
rg db.test.harness -g '!docs/**' -g '!*.json'   # expect: no matches (code/manifests clean)
cargo xtask check --no-test                      # expect PASSED
git add -A
git commit -m "refactor(issue-126): host test harness in storage::test_support, remove db-test-harness crate"
```
The pre-commit gate heals `crap-manifest.json` (drops the deleted crate's coverage entries) and fail-and-restages; re-add it and re-commit (the usual generated-manifest dance).

---

### Task 2: Amend ADR-0033

**Files:** Modify `docs/adr/0033-shared-db-test-harness-crate.md`, `docs/README.md`.

- [ ] **Step 1:** Rewrite ADR-0033's Decision to "feature-gated `test_support` module in `storage`," recording the dev-dependency-cycle blocker (two `storage` instances → `E0308`) as why a separate crate cannot serve `storage`'s own tests. Note `server` consumes it via the `test-support` feature. Retitle to e.g. `0033. In-`storage`Test-Support Module for Both-Backend Test Parametrization` and update the `docs/README.md` table row.

- [ ] **Step 2: Commit**

```bash
git add docs/adr/0033-shared-db-test-harness-crate.md docs/README.md
git commit -m "docs(issue-126): amend ADR-0033 — test_support module in storage, not a separate crate"
```

---

### Task 3: Convert the 45 storage contract tests in place

Now `crate::test_support` is same-instance, so the previously-blocked modules convert cleanly. For each module: add `use crate::test_support::{backends, Backend};` (+ `seed_user` where needed) and `use rstest::*; use rstest_reuse::*;` to its `mod tests`; convert each bucket-A test; remove the dead per-module DB helper.

Conversion pattern:
```rust
#[apply(backends)]
#[tokio::test]
async fn the_test(#[case] backend: Backend) {
    let env = backend.setup().await;
    let storage = &*env.state.<handle>;   // site_config / users / posts / user_config
    // …assertions unchanged (substitute the seeded user_id for hardcoded 1)…
}
```

- [ ] **Step 1: `storage/src/site_config.rs` (25 tests → both backends).** All via `&*env.state.site_config`; delete `test_pool`, `use crate::sqlite::SqliteSiteConfigStorage;`, `use sqlx::SqlitePool;`, and `use super::SiteConfigStorage;` (the `&dyn` calls don't need the trait import once concrete-type construction is gone — drop it only if clippy flags it unused).
- [ ] **Step 2: `storage/src/auth.rs` (4).** `&*env.state.site_config`; `load_registration_policy(store)` (no `&`). Delete `in_memory_store` + `use crate::SqliteSiteConfigStorage;`.
- [ ] **Step 3: `storage/src/user_config.rs` (4).** `seed_user(&env.state)` for the id; `&*env.state.user_config`. Delete `use crate::sqlite::SqliteUserConfigStorage;`.
- [ ] **Step 4: `storage/src/post_service.rs` (12).** `seed_user` + `&*env.state.posts`. `perform_post_creation`/`perform_post_update` take `(&dyn PostStorage, PostCreation<'_>/PostUpdate<'_>)` — substitute the seeded id into the struct's `user_id`/`editor_user_id` fields; pass `storage` (drop the `&`). Delete `setup_test_db`.
- [ ] **Step 5: `storage/src/posts.rs` (1).** `create_post_persists_summary`: `seed_user` + `&*env.state.posts`.
- [ ] **Step 6: Verify both backends, gate, commit (one commit for the conversion).**

```bash
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p storage
cargo xtask check --no-test
git add storage/src/*.rs
git commit -m "test(issue-126): run storage contract tests on both backends"
```
Expected: every converted `storage::…::tests::*` shows `::case_1_sqlite` + `::case_2_postgres`, all PASS; the bucket-B pure `#[test]`s untouched.

---

### Task 4: Final gate + no-trace verification

- [ ] **Step 1:** `cargo xtask validate --no-e2e` → `PASSED`, coverage clean (no lowering — the production paths are now exercised on Postgres too; `test_support`'s own coverage is its pure-fn tests + `// cov:ignore` as before).
- [ ] **Step 2:** `rg db.test.harness -g '!docs/**' -g '!*.json'` → no matches; `cargo metadata --format-version 1 | rg -o '"name":"db-test-harness"'` → no matches; confirm `crap-manifest.json` no longer lists db-test-harness *functions* (healed in Task 1); `git log --oneline wt-base-issue-126..HEAD`.

---

## Notes for the executor
- Task 1 is the delicate one (feature mechanics, lint gating, server rewire). Do it inline with tight gate checks; the conversions (Task 3) are mechanical and can be delegated.
- If `unwrap_used` denies in `test_support` under the feature build, use the dual-cfg split (Step 2 note).
- Out of scope (filed): #135 (ADR-0019/annotation/guard), #136 (backup-on-Postgres).
