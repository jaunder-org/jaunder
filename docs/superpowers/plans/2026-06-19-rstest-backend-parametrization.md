# rstest Backend Parametrization (Part 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the ~90 `assert_X` / `sqlite_X` / `postgres_X` test triples in `server/tests/storage.rs` into single `rstest`-parameterized tests that run on both storage backends.

**Architecture:** A `Backend` enum + `TestEnv` guard-owning struct + `setup()` produce a ready `Arc<AppState>` per backend. Three `#[template]`s (`backends`, `sqlite_only`, `postgres_only`) carry the case set; each test applies one and inlines its former `assert_X` body. The coverage run is unchanged (one nextest pass under the ephemeral PostgreSQL, from commit `8f9f71c`); the line-identity coverage gate guards against dropped cases.

**Tech Stack:** Rust, `rstest` 0.26 (new dev-dependency), `tokio::test`, `cargo nextest`, the `cargo xtask` driver.

## Global Constraints

- **`rstest = "0.26"` and `rstest_reuse = "0.7"`** in `[workspace.dependencies]`; `server` references both as `{ workspace = true }` under `[dev-dependencies]`. The `#[template]`/`#[apply]` macros live in **`rstest_reuse`**, not rstest core, and require its mandatory top-of-file imports (`use rstest_reuse;` bare + `use rstest_reuse::*;`). Test-only; never shipped in the `jaunder` binary.
- **Generated case ids are `case_N_<label>`** (e.g. `tag_normalization::case_1_sqlite`, `::case_2_postgres`) — that is how `rstest_reuse` names template cases. The labels (`sqlite`/`postgres`) are still present; nothing keys on the exact id string.
- **Approach A — `#[template]` + `#[apply]`.** Per-test overhead is exactly one `#[apply(<template>)]` attribute, one `#[case] backend: Backend` param, and one `backend.setup().await` line. Keep this ceremony visible — no bespoke wrapping macro.
- **Every case is named** (`#[case::sqlite(...)]`, `#[case::postgres(...)]`).
- **Coverage run stays one pass.** Do not touch `scripts/check-coverage` or `flake.nix`. PG cases need the ephemeral PostgreSQL that is already up for the whole run.
- **Coverage baseline (`coverage-baseline.json`) tracks `src` only** (no `tests/` keys) and is expected to stay stable. A *regression* means a real dropped case — investigate, never heal it away. Benign shrink may be auto-healed by `cargo xtask check`.
- **Out of scope for Part 1:** `commands.rs` (PostgreSQL-*command* tests, no SQLite analog) and `backup_interop.rs` (cross-backend interop tests). They do not fit the single-backend matrix. Leave them as plain `#[tokio::test]`s.
- **xtask invocation:** run `cargo xtask …` bare (no `2>&1`/pipe/`; echo`), pass/fail is the exit code. Running storage tests requires PostgreSQL: wrap with `scripts/with-ephemeral-postgres`.

---

## File Structure

- **`Cargo.toml`** (workspace root, `:23-76`) — add `rstest = "0.26"` to `[workspace.dependencies]`.
- **`server/Cargo.toml`** (`:60-69`, `[dev-dependencies]`) — add `rstest = { workspace = true }`.
- **`server/tests/storage.rs`** — add `Backend`/`TestEnv`/`setup` + the three templates near the top (after the existing `sqlite_state`/`postgres_state` at `:51-60`); convert all triples; convert the genuinely single-backend state tests to single-case templates. The existing `sqlite_state()`/`postgres_state()` stay (now called only by `Backend::setup`).

No other files change. `server/tests/storage.rs` is already a large file (~6400 lines); we are reducing it (deleting ~190 wrappers), not restructuring it.

---

## The Conversion Recipe (referenced by Tasks 3–9)

For each behavior currently expressed as a triple:

```rust
// BEFORE
async fn assert_tag_normalization(state: &AppState) { /* body */ }

#[tokio::test]
async fn sqlite_tag_normalization() {
    let (_base, state) = sqlite_state().await;
    assert_tag_normalization(&state).await;
}

#[tokio::test]          // (the #[ignore = "requires PostgreSQL"] was removed in B5)
async fn postgres_tag_normalization() {
    let state = postgres_state().await;
    assert_tag_normalization(&state).await;
}
```

```rust
// AFTER
#[apply(backends)]
#[tokio::test]
async fn tag_normalization(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    /* the former assert_tag_normalization body, verbatim */
}
```

Mechanical rule, applied per `assert_X`:

1. **Delete** `sqlite_X` and `postgres_X`.
2. **Rename** `assert_X` → `X`, change its signature to `(#[case] backend: Backend)`, prepend `#[apply(<template>)]` + `#[tokio::test]`, and insert `let env = backend.setup().await;` + `let state = &env.state;` as the first two body lines. The rest of the body is unchanged (it already binds `state: &AppState`).
3. **Template choice** is determined by which wrappers existed:
   - both `sqlite_X` and `postgres_X` → `#[apply(backends)]`
   - only `postgres_X` → `#[apply(postgres_only)]`
   - only `sqlite_X` → `#[apply(sqlite_only)]`
4. **Helpers with multiple real callers stay helpers.** If `assert_X` is called by more than one wrapper-pair (e.g. the `*_parity_suite` sequences call several `assert_*`), keep it as a helper and convert only the wrapper that invokes it. A `*_app_state_parity_suite` becomes a single `app_state_parity_suite(#[case] backend)` that calls the same `assert_*` helpers against `&env.state`.
5. **Bodies that differ between `sqlite_X` and `postgres_X`** (not thin wrappers over a shared `assert_X`) are converted by hand: move the shared logic into the test, gate any genuinely backend-specific assertion on `matches!(backend, Backend::Postgres)`. Flag any such case in the commit message.
6. **Do not touch standalone tests** that don't follow the triple naming (e.g. `second_open_on_migrated_database_succeeds`, `set_then_get_roundtrips`, `open_pool`-based storage-internal tests) — they exercise SQLite storage internals directly and are not part of the parametric family. Task 9 handles the genuinely backend-specific *open/migration* tests.

After each conversion task: `cargo xtask check --no-test` must be green (compile + clippy). Bodies are moved verbatim, so the only failure mode is rstest wiring — which compilation catches.

---

## Task 1: Add `rstest`, the `Backend` fixture, and the templates; prove the idiom (spike)

**Files:**
- Modify: `Cargo.toml:23-76`
- Modify: `server/Cargo.toml:60-69`
- Modify: `server/tests/storage.rs:51-60` (add below the state helpers)

**Produces:** `Backend` (Copy enum `Sqlite`/`Postgres`), `TestEnv { state: Arc<AppState>, _guard: Option<TempDir> }`, `Backend::setup(self) -> TestEnv`, and `#[template]`s `backends`, `sqlite_only`, `postgres_only`. These are consumed by every later task.

- [ ] **Step 1: Add the workspace dependency**

In `Cargo.toml`, under `[workspace.dependencies]` (alongside `tempfile = "3"` etc.):

```toml
rstest = "0.26"
rstest_reuse = "0.7"
```

- [ ] **Step 2: Add the dev-dependency**

In `server/Cargo.toml`, under `[dev-dependencies]`:

```toml
rstest = { workspace = true }
rstest_reuse = { workspace = true }
```

- [ ] **Step 3: Add the fixture and templates to `storage.rs`**

Add the rstest imports and a crate-level allow to the top of `server/tests/storage.rs`:

```rust
#![allow(unused_macros)] // single-case templates expand to name-mangled macro_rules! a per-item allow can't reach
use rstest::*;
use rstest_reuse; // bare import REQUIRED by rstest_reuse — the glob alone is insufficient
use rstest_reuse::*;
```

Then, immediately after `sqlite_state`/`postgres_state` (`:60`):

```rust
use storage::AppState;

#[derive(Copy, Clone)]
enum Backend {
    Sqlite,
    Postgres,
}

struct TestEnv {
    state: std::sync::Arc<AppState>,
    _guard: Option<TempDir>,
}

impl Backend {
    async fn setup(self) -> TestEnv {
        match self {
            Backend::Sqlite => {
                let (base, state) = sqlite_state().await;
                TestEnv { state, _guard: Some(base) }
            }
            Backend::Postgres => TestEnv {
                state: postgres_state().await,
                _guard: None,
            },
        }
    }
}

#[template]
#[rstest]
#[case::sqlite(Backend::Sqlite)]
fn sqlite_only(#[case] backend: Backend) {}

#[template]
#[rstest]
#[case::postgres(Backend::Postgres)]
fn postgres_only(#[case] backend: Backend) {}

#[template]
#[rstest]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
fn backends(#[case] backend: Backend) {}
```

(`AppState` may already be imported transitively; if `use storage::AppState;` conflicts, drop the duplicate.)

- [ ] **Step 4: Convert one real test as the spike**

Pick `tag_normalization`. Apply the Conversion Recipe to `assert_tag_normalization` / `sqlite_tag_normalization` / `postgres_tag_normalization` (`storage.rs` ~`:3690-3755`), producing the `#[apply(backends)]` form shown in the recipe.

- [ ] **Step 5: Compile + clippy**

Run: `cargo xtask check --no-test`
Expected: green. If it fails on attribute ordering, this is the spike's job — try, in order: (a) `#[apply(backends)]` above `#[tokio::test]` (the documented form); (b) swap to `#[tokio::test]` above `#[apply(backends)]`; (c) add `#[awt]` and `#[future]` per rstest async docs. Record the working ordering in the commit message; it is the canonical ordering all later tasks use.

- [ ] **Step 6: Run the spike test on both backends**

Run: `scripts/with-ephemeral-postgres cargo nextest run -p jaunder tag_normalization`
Expected: two cases pass — `tag_normalization::sqlite` and `tag_normalization::postgres`.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml server/Cargo.toml server/tests/storage.rs
git commit -m "test(storage): add rstest Backend fixture + templates; convert tag_normalization (spike)"
```

---

## Tasks 2–8: Convert the `storage.rs` triple families

Each task applies the Conversion Recipe to one feature family, deletes the wrappers, runs `cargo xtask check --no-test` (green), and commits. Families (function stems visible in the file; convert the `assert_`/`sqlite_`/`postgres_` set for each):

- [ ] **Task 2: site-config / user / session / invite / email-verification / password-reset + parity suite.** `app_state_parity_suite`, `site_config_set_then_get_roundtrips`, `authenticate_with_corrupted_hash_returns_internal_error`, `create_user_duplicate_and_authenticate_work`, `session_lifecycle_works`, `feed_events_marks_run`, `invite_and_atomic_registration_work`, `email_verification_and_password_reset_work`. (`storage.rs` ~`:360-490`.) Per the recipe, the `*_parity_suite` becomes one `app_state_parity_suite(#[case] backend)` calling the existing `assert_*` helpers.
  - Verify: `cargo xtask check --no-test` green. Commit: `test(storage): parametrize user/session/invite/email family over backends`.

- [ ] **Task 3: posts — create/get/update/delete/list/soft-delete.** `post_create_and_get_by_id_works`, `post_slug_conflict_returns_slug_conflict`, `post_update_writes_revision_and_updates_record`, `post_update_not_found_returns_error`, `soft_delete_excludes_post_from_lists`, `list_published_by_user_returns_only_user_posts`, `list_published_returns_published_non_deleted_posts`, `list_published_in_window_applies_hybrid_rule_across_surfaces`, `list_drafts_by_user_returns_only_drafts`. (~`:1930-2010`.)
  - Verify green. Commit: `test(storage): parametrize posts family over backends`.

- [ ] **Task 4: tags — part 1 (lifecycle & errors).** `tag_creation_and_retrieval`, `tag_normalization` (already done in spike — skip), `untag_post`, `duplicate_tag_error`, `list_posts_by_tag`, `list_user_posts_by_tag`, `tag_not_found_error`, `soft_deleted_posts_excluded_from_tag_list`, `draft_posts_excluded_from_tag_list`, `tag_post_nonexistent_post_error`, `untag_nonexistent_tag_error`. (~`:3740-3810`.)
  - Verify green. Commit: `test(storage): parametrize tag lifecycle family over backends`.

- [ ] **Task 5: tags — part 2 (variants & pagination).** `multiple_tags_on_single_post`, `empty_tag_list`, `tag_case_preservation_variants`, `invalid_tag_input`, `tag_list_pagination`, `list_user_posts_by_tag_excludes_other_users`, `selective_untag`, `numeric_tag`, `retag_same_post_with_same_tag_fails`, `untag_nonexistent_post`, `get_tags_nonexistent_post`, `list_posts_by_nonexistent_tag`, `list_user_posts_by_nonexistent_tag`, `many_tags_many_posts`, `tag_all_numeric`, `tag_hyphen_boundaries`, `tag_with_long_display`, `tag_list_ordering`, `tags_for_multiple_posts`, `tag_mixed_alphanumeric`, `simple_tag_lifecycle`. (~`:3860-4070`.)
  - Verify green. Commit: `test(storage): parametrize tag variants family over backends`.

- [ ] **Task 6: post cursors & edge cases.** `post_update_invalid_slug`, `list_published_cursor_boundary`, `list_drafts_cursor_boundary`, `list_user_posts_by_tag_cursor`, `list_posts_by_tag_cursor`, `soft_delete_then_operations`, `tag_post_multiple_attempts`, `list_published_by_user_no_posts`, `get_by_permalink_soft_deleted`, `update_soft_deleted_post`, `tag_edge_case_formats`, `get_post_by_id_nonexistent`, `list_published_with_cursor_same_timestamp`, `post_revisions_created`, `tag_display_preservation`, `untag_preserves_other_tags`. (~`:4510-5195`.)
  - Verify green. Commit: `test(storage): parametrize post cursor/edge family over backends`.

- [ ] **Task 7: site-config/session/invite operations + rendered posts.** `site_config_operations`, `session_list_operations`, `invite_list_operations`, `create_rendered_post_markdown_renders_and_stores`, `create_rendered_post_org_renders_and_stores`, `create_rendered_post_slug_conflict_returns_storage_error`, `update_rendered_post_markdown_renders_and_updates`, `update_rendered_post_org_renders_and_updates`, `update_rendered_post_not_found_returns_storage_error`. (~`:5340-5665`.)
  - Verify green. Commit: `test(storage): parametrize operations + rendered-post family over backends`.

- [ ] **Task 8: media / user-config / list-tags / post-record.** `create_and_get_media`, `duplicate_media_returns_already_exists`, `delete_media_removes_record`, `delete_nonexistent_returns_not_found`, `list_media_returns_records_for_user`, `list_media_filtered_by_source`, `get_user_upload_usage_returns_zero_initially`, `get_user_upload_usage_sums_uploads_only`, `find_by_hash_returns_any_match`, `user_config_get_returns_none_when_unset`, `user_config_set_and_get`, `user_config_overwrite`, `user_config_delete_removes_key`, `user_config_delete_nonexistent_is_ok`, `list_tags_returns_alphabetical_with_prefix`, `post_record_carries_tags`. (~`:6150-6390`.)
  - Verify green. Commit: `test(storage): parametrize media/user-config/list-tags family over backends`.

---

## Task 9: Backend-specific state tests → single-case templates

**Files:** Modify `server/tests/storage.rs`.

Some tests are genuinely one-backend. Apply the single-case templates so they read like every other test:

- [ ] **Step 1: Identify the remaining `postgres_*` / `sqlite_*` tests** that were NOT part of a triple (no opposite-backend twin and not yet converted). Confirm with: `cargo nextest list -p jaunder --tests` (no remaining `assert_*`/`sqlite_*`/`postgres_*`-prefixed names except intended helpers).

- [ ] **Step 2: Convert state-fixture single-backend tests.** Any test that uses `sqlite_state()`/`postgres_state()` for a single backend with no twin → apply `#[apply(sqlite_only)]` or `#[apply(postgres_only)]` per the recipe.

- [ ] **Step 3: Leave the open/migration tests as plain tests.** `open_database_succeeds_on_postgres_test_vm`, `open_database_runs_postgres_migrations_on_existing_empty_db`, `open_existing_database_runs_postgres_migrations_on_unmigrated_db` exercise the *open path itself* (they call `open_database`/`open_existing_database` directly, not a prebuilt `AppState`), so they do not use the `Backend::setup` fixture. Leave them as `#[tokio::test]`. Likewise the parse tests (`postgres_url_is_accepted_at_parse_time`, `unsupported_url_is_rejected_at_parse_time`) and SQLite storage-internal tests (`second_open_on_migrated_database_succeeds`, `set_then_get_roundtrips`, `get_missing_key_returns_none`, `set_overwrites_existing_value`).

- [ ] **Step 4: Compile + clippy**

Run: `cargo xtask check --no-test`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add server/tests/storage.rs
git commit -m "test(storage): single-case templates for backend-specific state tests"
```

---

## Task 10: Full gate, baseline, and docs

**Files:** Modify `CONTRIBUTING.md` (the "PostgreSQL-backed Rust tests" passage, ~`:160-162`).

- [ ] **Step 1: Run the full pre-push gate**

Run: `cargo xtask validate --no-e2e`
Expected: exit 0; coverage **clean** (the conversion is test-code-only; the `src`-only baseline should be unchanged). The build log goes to the JSON sidecar; read `.xtask/last-result.json` `steps[]` only if it fails.

- [ ] **Step 2: Handle any coverage delta.** If the gate reports `new_uncovered`/regression, a real case was dropped during conversion — find the missing behavior and restore it; do **not** edit the baseline to pass. If it reports only structural/improvement shrink, run `cargo xtask check` once to auto-heal and re-run `cargo xtask validate --no-e2e` to confirm clean.

- [ ] **Step 3: Update CONTRIBUTING.** Replace the description of the `sqlite_*`/`postgres_*` paired-wrapper pattern with the rstest reality: backend-parametric tests are written once and applied to the `backends` template (`#[apply(backends)]`), expanding into a `::sqlite` and a `::postgres` case in the single nextest pass; single-backend tests use `sqlite_only`/`postgres_only`. Keep it to the surrounding style.

- [ ] **Step 4: Commit**

```bash
git add server/tests/storage.rs CONTRIBUTING.md
git commit -m "test(storage): rstest backend parametrization complete; update CONTRIBUTING"
```

---

## Done criteria

- `server/tests/storage.rs` has no `assert_*`/`sqlite_*`/`postgres_*`-prefixed test triples; backend-parametric tests use `#[apply(backends)]`, single-backend ones use `#[apply(sqlite_only|postgres_only)]`.
- `cargo xtask validate --no-e2e` is green with coverage clean.
- `rstest` is a workspace dev-dependency; `cargo deny` (run inside the static checks) stays green.
- ~190 wrapper functions removed; each behavior defined once.
