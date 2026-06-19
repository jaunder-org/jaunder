# HTTP-Layer rstest (Part 2 + Part 3, merged) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the ~254 HTTP-layer integration tests in `server/tests/*.rs` run on **both** storage backends via rstest, and collapse their validation/authorization clusters into `#[case]` tables — both transforms applied per file in one pass.

**Architecture:** Reuse Part 1's exact pattern. Hoist `Backend`/`TestEnv`/templates from `storage.rs` into the shared `helpers` module, widen `TestEnv` to always carry a `base: TempDir` (the media-storage dir HTTP tests need on both backends), and convert every HTTP test to `let TestEnv { state, base } = backend.setup().await;` + `#[apply(backends)]`. Rejection/authorization clusters become a backend×value matrix (`#[values(..)] backend` + `#[case]` rows). The mailer/websub helpers — never backend-specific — become inline construction. The coverage run is unchanged (one nextest pass under ephemeral PostgreSQL); HTTP tests gain a `::postgres` case, so coverage *improves* (baseline shrinks, healed once at the end).

**Tech Stack:** Rust, `rstest` 0.26 + `rstest_reuse` 0.7 (already dev-deps from Part 1), `tokio::test`, `cargo nextest`, the `cargo xtask` driver.

## Global Constraints

- **Reuse Part 1's fixture verbatim.** `Backend` (Copy enum), `TestEnv`, `Backend::setup`, and the `backends`/`sqlite_only`/`postgres_only` templates already exist in `server/tests/storage.rs`; this plan **moves** them to `server/tests/helpers/mod.rs` (one copy) and widens `TestEnv` to `{ state: Arc<AppState>, base: TempDir }` (both `pub`, `base` always present).
- **The HTTP-test shape is `storage.rs`'s shape:** `let TestEnv { state, base } = backend.setup().await;` then the original body unchanged (it already refers to `state` and `base`). No `test_state`/harness call.
- **`rstest_reuse` import rule:** any file using `#[apply(..)]` needs `use rstest::*;`, the bare `use rstest_reuse;`, `use rstest_reuse::*;`, and crate-level `#![allow(unused_macros)]` if it imports unused templates.
- **Pinned by the Task 1 spike (use these exact forms):** import the template by **bare name** — `use helpers::{Backend, TestEnv, backends};` (add `sqlite_only`/`postgres_only` only in a file that applies them; importing an unused template name trips `-D unused-imports`) — then `#[apply(backends)]` (NOT `#[apply(helpers::backends)]`). The **matrix form** is `#[rstest]` → named `#[case::name(..)]` rows → `#[tokio::test]`, with `#[values(Backend::Sqlite, Backend::Postgres)] backend: Backend` as the **first** parameter and the `#[case]` value params after it.
- **Two per-test shapes only** (see "Recipe"): non-clustered → `#[apply(backends)]` + `#[case] backend: Backend`; clustered → `#[rstest]` + `#[values(Backend::Sqlite, Backend::Postgres)] backend` + named `#[case(..)]` value rows. Every case named.
- **Mailer/websub inline.** Replace `test_state_with_mailer`/`test_state_with_websub` call sites with `backend.setup()` plus a one-line `let mailer = Arc::new(CapturingMailSender::new());` / `let capturing = Arc::new(CapturingWebSubClient::default());` (these don't depend on the backend; they're wired into `create_router`, not `AppState`).
- **Raw-pool tests stay single-backend.** Tests using `test_sqlite_state_with_pool` (raw `SqlitePool` SQL) cannot be backend-parametrized — leave them plain `#[tokio::test]` or `#[apply(sqlite_only)]`. `test_sqlite_state_with_pool` itself stays.
- **Coverage run unchanged.** Do not touch `scripts/check-coverage` or `flake.nix`.
- **Coverage will improve, not regress.** HTTP paths now run on PostgreSQL, covering `storage/src/postgres/*` lines previously reached only by `storage.rs`. The strict gate never fails on improvement; `coverage-baseline.json` is healed once (Finalize) via `cargo xtask check`. Never hand-edit the baseline.
- **xtask invocation:** run `cargo xtask …` bare via context-mode `ctx_execute(language:"shell", …)` (no `2>&1`/pipe/`;echo`). Per-task gate is `cargo xtask check --no-test` (compile + clippy). Running HTTP tests needs PostgreSQL — deferred to Finalize's `validate`.

---

## File Structure

- **`server/tests/helpers/mod.rs`** — gains `pub Backend`/`pub TestEnv { state, base }`/`Backend::setup` and the three `pub` templates. The env-selected `test_state`/`test_state_with_mailer`/`test_state_with_websub` are removed in Finalize once no file calls them. `test_sqlite_state_with_pool` stays.
- **`server/tests/storage.rs`** — drop its local `Backend`/`TestEnv`/templates and orphaned `sqlite_state`/`postgres_state`; `use helpers::{Backend, TestEnv, backends, sqlite_only, postgres_only}` (+ rstest_reuse imports). Its test bodies use `&env.state` and are unaffected by `TestEnv` gaining a `base` field.
- **The 20 HTTP-layer files** — each backend-parametrized + value-clustered:
  `web_posts.rs` (64), `web_auth.rs` (29), `atompub_posts.rs` (26), `web_backup.rs` (19), `media_handlers.rs` (15), `web_account.rs` (14), `web_media.rs` (11), `feed_handlers.rs` (10), `atompub_media.rs` (9), `web_sessions.rs` (9), `web_password_reset.rs` (8), `web_site.rs` (7), `feed_events_hook.rs` (6), `web_email.rs` (6), `feed_worker.rs` (5), `web_tags.rs` (5), `feed_regenerate.rs` (4), `atompub_service.rs` (3), `atompub_rsd.rs` (2), `static_assets.rs` (2).

---

## The Recipe (referenced by every per-file task)

A typical HTTP test today:

```rust
#[tokio::test]
async fn foo() {
    let base = TempDir::new().unwrap();
    let state = test_state(&base).await;        // or _with_mailer/_with_websub
    let app = make_app(state, &base).await;
    // …requests + asserts…
}
```

**Shape A — non-clustered behavior:**

```rust
#[apply(backends)]
#[tokio::test]
async fn foo(#[case] backend: Backend) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(state, &base).await;
    // …body unchanged…
}
```

The only edit is the attribute/param line and replacing the two setup lines with the destructure. If a `_with_mailer`/`_with_websub` variant was used, add the one inline construction line and pass it where the old tuple element went.

**Shape B — a rejection/authorization cluster → backend×value matrix.** The backend axis is `#[values]` (because `#[apply]`'s `#[case]` can't coexist with value `#[case]` rows); the value axis is named `#[case]`s:

```rust
#[rstest]
#[case::empty_body(empty_form(), CreatePostError::Empty)]
#[case::missing_slug(no_slug_form(), CreatePostError::NoSlugSource)]
#[tokio::test]
async fn create_post_rejects(
    #[values(Backend::Sqlite, Backend::Postgres)] backend: Backend,
    #[case] form: PostForm,
    #[case] expected: CreatePostError,
) {
    let TestEnv { state, base } = backend.setup().await;
    let app = make_app(state, &base).await;
    // …submit `form`, assert the response maps to `expected`…
}
```

> The Shape B type/helper names (`empty_form()`, `CreatePostError::Empty`, `PostForm`) are **illustrative** — use the file's actual form builders and error/status types. What's fixed is the attribute structure (pinned by Task 1's spike).

Rules:
1. **Backend-parametrize every test** whose body builds an app via `test_state`/`_with_mailer`/`_with_websub`. Add the rstest_reuse imports to the file.
2. **Collapse only genuine clusters:** identical setup + assertion *structure*, differing only by input or endpoint. Each row a named `#[case::name(..)]`. If merging needs `if`-branching on the case, keep them separate (Shape A each). A file may have zero clusters — then all Shape A.
3. **Raw-pool / SQLite-internal tests** stay single-backend (plain `#[tokio::test]` or `#[apply(sqlite_only)]`); note each in the commit.
4. Per-file gate: `cargo xtask check --no-test` green. Bodies move verbatim; the only failure mode is rstest wiring.

---

## Task 1: Foundation — hoist + widen the fixture, spike cross-module apply + the matrix idiom

**Files:** Modify `server/tests/helpers/mod.rs`, `server/tests/storage.rs`. Spike on `server/tests/atompub_rsd.rs` (2 tests).

**Produces:** `helpers::Backend`, `helpers::TestEnv { state: Arc<AppState>, base: TempDir }`, `Backend::setup`, `helpers::{backends, sqlite_only, postgres_only}`, and the pinned idioms every later task copies.

- [ ] **Step 1: Move + widen the fixture.** Cut `Backend`, `TestEnv`, `Backend::setup`, and the three `#[template]`s from `storage.rs` into `helpers/mod.rs` (`pub`). Change `TestEnv` to `pub struct TestEnv { pub state: std::sync::Arc<AppState>, pub base: TempDir }` (was `_guard: Option<TempDir>`). Rewrite `setup` to always produce a `base`, using `helpers`' own primitives:

```rust
impl Backend {
    pub async fn setup(self) -> TestEnv {
        let base = TempDir::new().unwrap();
        let state = match self {
            Backend::Sqlite => open_database(&sqlite_url(&base)).await.unwrap(),
            Backend::Postgres => open_existing_database(&template_postgres_url().await).await.unwrap(),
        };
        TestEnv { state, base }
    }
}
```

Add `use rstest::*; use rstest_reuse; use rstest_reuse::*;` to `helpers/mod.rs` and `#![allow(unused_macros)]` at its top (and `#![allow(dead_code)]` if not already tolerated — `helpers` is included into every test binary and not every item is used by each).

- [ ] **Step 2: Re-point `storage.rs`.** Replace its removed local defs with `use helpers::{Backend, TestEnv, backends, sqlite_only, postgres_only};` (keep its existing rstest_reuse imports). Delete `storage.rs`'s now-orphaned `sqlite_state`/`postgres_state` (only `setup` called them — clippy confirms). `storage.rs` test bodies use `&env.state` only, so `TestEnv` gaining `base` is transparent.

- [ ] **Step 3: SPIKE cross-module `#[apply]`.** In `atompub_rsd.rs`, add the rstest_reuse imports and convert ONE test to `#[apply(backends)]` (template now in `helpers`) with the Shape-A `let TestEnv { state, base } = backend.setup().await;`. Determine how `#[apply]` resolves a template from another module — try in order: (a) `use helpers::backends;` then `#[apply(backends)]`; (b) `#[apply(helpers::backends)]`; (c) `pub use` re-export in helpers. Record the working form — canonical for all files.

- [ ] **Step 4: SPIKE the backend×value matrix.** Convert `atompub_rsd.rs`'s other test (or a scratch test) to Shape B: `#[rstest]` + `#[values(Backend::Sqlite, Backend::Postgres)] backend` + two `#[case]` rows. Confirm it generates `rows × 2` cases. Record the exact attribute ordering.

- [ ] **Step 5:** `cargo xtask check --no-test` green.

- [ ] **Step 6: Commit** `test(http): hoist+widen Backend fixture to helpers; finish atompub_rsd; pin cross-module apply + matrix (spike)`.

---

## Tasks 2–11: Per-file conversion

Apply the Recipe to each file: backend-parametrize every app-building test (Shape A), collapse clusters (Shape B), leave raw-pool/SQLite-internal tests single-backend. Each task ends green on `cargo xtask check --no-test` and commits. Grouped by size/harness:

- [ ] **Task 2: `web_posts.rs`** (64 tests, ~34 cluster candidates — the densest). Expect several Shape-B tables: `create_post_rejects_*` → one `create_post_rejects` matrix; `update_post_rejects_*` → one; `*_rejects_unauthenticated` across endpoints → one (parameterize the request builder); `list_*_rejects_invalid_cursor_inputs` → one. Everything else Shape A. Commit `test(http): backend-parametrize + cluster web_posts`.

- [ ] **Task 3: `atompub_posts.rs`** (26, ~12). Clusters: `*_forbids_other_user` (×4) → one matrix over the request/resource; `*_with_no_title_or_content_returns_400` (×2); the cursor accept/reject pair. Rest Shape A. Commit.

- [ ] **Task 4: `web_auth.rs`** (29, ~5). Mostly Shape A; collapse any login/register rejection cluster. Commit.

- [ ] **Task 5: `web_backup.rs`** (19; **2 use `test_sqlite_state_with_pool`** → leave single-backend). Backend-parametrize the other 17; collapse the ~6 rejection tests if they share shape. Commit.

- [ ] **Task 6: `media_handlers.rs` (15) + `web_media.rs` (11) + `atompub_media.rs` (9).** Media trio; collapse rejection clusters (~5/5/6). Commit `test(http): backend-parametrize + cluster media handlers`.

- [ ] **Task 7: `web_account.rs` (14) + `web_sessions.rs` (9) + `web_site.rs` (7) + `web_tags.rs` (5).** Mostly Shape A. Commit.

- [ ] **Task 8: `feed_handlers.rs` (10) + `feed_events_hook.rs` (6) + `feed_regenerate.rs` (4).** Shape A + any small clusters. Commit.

- [ ] **Task 9: `feed_worker.rs` (5).** Was `test_state_with_websub` — convert to `backend.setup()` + inline `CapturingWebSubClient::default()`; this is its first PostgreSQL run, so watch for a PG-specific websub/feed-event issue. Commit.

- [ ] **Task 10: `web_password_reset.rs` (8) + `web_email.rs` (6).** Were `test_state_with_mailer` — convert to `backend.setup()` + inline `CapturingMailSender::new()`; verify captured-mail assertions hold on both backends. Commit.

- [ ] **Task 11: `atompub_service.rs` (3) + `static_assets.rs` (2).** Small tail. `static_assets` may be backend-irrelevant (static file serving) — if a test never touches storage, leave it plain and note why. Commit. (`atompub_rsd.rs` was finished in Task 1.)

---

## Task 12: Finalize — remove old harness, heal baseline, full gate, docs

**Files:** `server/tests/helpers/mod.rs`, `coverage-baseline.json` (regenerated), `CONTRIBUTING.md`.

- [ ] **Step 1: Delete the now-unused env-selected harness.** With every file migrated to `backend.setup()`, `test_state`, `test_state_with_mailer`, `test_state_with_websub` (and `postgres_testing_enabled`, if nothing else uses it) are dead — remove them from `helpers/mod.rs`. Keep `test_sqlite_state_with_pool`. `cargo xtask check --no-test` confirms nothing still calls them.

- [ ] **Step 2: Heal the improved baseline.** Run `cargo xtask check` (Fix mode) — HTTP paths now run on PostgreSQL, so previously-uncovered `storage/src/postgres/*` lines are covered; Fix shrinks `coverage-baseline.json`. This is an improvement, expected; eyeball that the baseline diff is shrink-only.

- [ ] **Step 3: Full gate.** Run `cargo xtask validate --no-e2e`. Expect green, coverage clean. Any `new_uncovered`/regression means a test was dropped or a body altered — investigate; never hand-edit the baseline.

- [ ] **Step 4: Update CONTRIBUTING.** Note the HTTP-layer integration tests are now backend-parametric too (`#[apply(backends)]`, or the `#[values]`+`#[case]` matrix for clustered tests) via the same `Backend::setup()` fixture, so the whole integration suite runs on both backends per run.

- [ ] **Step 5: Commit** `test(http): remove env-selected harness; heal baseline; backend-parametric integration suite complete` (the healed `coverage-baseline.json` travels in this commit).

---

## Verification

- Per file: `cargo xtask check --no-test` (compile + clippy) green before commit.
- Finalize: `cargo xtask validate --no-e2e` green, coverage clean after the heal.
- Runtime parity is proven by the green instrumented run (every converted test executes `::sqlite` and `::postgres`).

## Sequencing

Task 1 (foundation + spike) → Tasks 2–11 (per file, `web_posts` first as the richest exemplar; otherwise any order) → Task 12 (finalize). Tasks 2–11 are independent once the fixture lands; the old harness stays until Finalize so each file stays green.

## Risks

- **Cross-module `#[apply]` / template resolution** — the central unknown; Task 1 spike resolves it before bulk work. Fallback if templates can't apply cross-module: define the three templates once per file (a few lines) or via a shared `macro_rules!`.
- **`#[values]` + `#[case]` matrix** — pinned by Task 1 spike; if it won't compose, clustered tests fall back to Shape A and value-clustering degrades gracefully to backend-only.
- **`feed_worker` PostgreSQL path** — newly exercised; a PG-specific feed-event/websub issue could surface (a real find, not a defect of this work). Same for any handler path never before run on PG.
- **Runtime growth** — ~254 HTTP tests gain a PostgreSQL case; the coverage run does more PG work. Acceptable; monitor `validate` duration.
- **Coverage heal size** — the baseline shrink may be large (many postgres lines newly covered). Confirm the diff is shrink-only.
