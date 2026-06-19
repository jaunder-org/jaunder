# rstest Test Parameterization — Design

**Status:** approved (design phase)
**Date:** 2026-06-19
**Follows:** the testing/coverage orchestration redesign (epic `jaunder-1bhw`) and its
final piece, the one-batch coverage run with the PostgreSQL `#[ignore]` gating removed
(commit `8f9f71c`). This spec builds directly on that: the test environment is the Nix
coverage check, which runs the whole suite once under an ephemeral PostgreSQL.

## Goal

Eliminate the per-test backend boilerplate in the integration suite by adopting
[`rstest`](https://docs.rs/rstest) parameterization, in two independently-shippable parts:

1. **Part 1 — backend parametrization.** Collapse the ~90 `assert_X` / `sqlite_X` /
   `postgres_X` triples in `storage.rs` (plus 2 in `commands.rs`, 1 in `backup_interop.rs`)
   into single tests parameterized over the storage backend.
2. **Part 2 — value parametrization.** Collapse clusters of near-duplicate single-backend
   tests (validation-rejection and authorization tables) in the web/atompub/handler test
   files into `#[case]` tables.

## Motivation

Today every backend-parametric behavior is **three functions**: `assert_X` (the real body)
plus `sqlite_X` and `postgres_X` — two near-identical wrappers differing only in
`sqlite_state()` vs `postgres_state()`. That is ~190 wrapper functions carrying no
information, and a standing maintenance hazard: adding `sqlite_foo` without its
`postgres_foo` twin silently drops PostgreSQL parity for that behavior.

The single-backend web/atompub files have a different repetition: clusters of tests that
share one assertion shape and vary only by input (`create_post_rejects_*`, ≈8 variants) or
by endpoint (`*_rejects_unauthenticated`, `*_forbids_other_user`).

`rstest` addresses both: a parameterized test defined once, expanded into one case per
backend (Part 1) or per input row (Part 2). Write-once means a new behavior automatically
covers both backends, and a reader learns a single standard idiom.

## Global Constraints

- **`rstest` is a workspace dev-dependency** (test-only; never shipped in the `jaunder`
  binary). `cargo deny` must stay green (its `syn`/`quote`/`proc-macro2` deps are already
  in-tree) — verified, not assumed.
- **Approach A — `#[template]` + `#[apply]`.** A single source of truth for each case set;
  per-test overhead is one `#[apply(...)]` attribute, one `#[case] backend: Backend` param,
  and one `backend.setup().await` line. The visible rstest ceremony is kept (no bespoke
  wrapping macro); revisit only if it becomes obtrusive.
- **Every case is named** (`#[case::sqlite(...)]`, `#[case::empty_body(...)]`) so a failure
  still points at one identifiable scenario.
- **The coverage run stays one pass — B5 is untouched.** rstest expands all cases into the
  single nextest run already executing under the ephemeral PostgreSQL. `scripts/check-coverage`
  and `flake.nix` need no changes.
- **The line-identity coverage gate is the safety net.** Both parts are test-code-only; the
  `coverage-baseline.json` tracks `src` files only (no `tests/` keys), so the baseline is
  expected to stay stable. A *regression* means a real dropped case — investigate, never heal
  it away. Benign shrink may be auto-healed by `cargo xtask check` (Fix mode).

---

## Part 1 — Backend Parametrization

### The `Backend` fixture

`sqlite_state()` returns `(TempDir, Arc<AppState>)` — the temp dir is a guard that must
outlive the test — while `postgres_state()` returns `Arc<AppState>`. The unified setup owns
the optional guard:

```rust
#[derive(Copy, Clone)]
enum Backend { Sqlite, Postgres }

struct TestEnv { state: Arc<AppState>, _guard: Option<TempDir> }

impl Backend {
    async fn setup(self) -> TestEnv {
        match self {
            Backend::Sqlite   => { let (g, s) = sqlite_state().await; TestEnv { state: s, _guard: Some(g) } }
            Backend::Postgres => { let s = postgres_state().await;    TestEnv { state: s, _guard: None } }
        }
    }
}
```

### The three templates

```rust
#[template] #[rstest] #[case::sqlite(Backend::Sqlite)]     fn sqlite_only(#[case] backend: Backend) {}
#[template] #[rstest] #[case::postgres(Backend::Postgres)] fn postgres_only(#[case] backend: Backend) {}
#[template] #[rstest]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
fn backends(#[case] backend: Backend) {}
```

Every storage test uses the same shape — `#[apply(<template>)]` + `#[case] backend: Backend`
+ `backend.setup()`. The template name documents intent (`postgres_only` says *why* a test is
single-backend). A reader learns exactly one pattern.

### Conversion

Each `(assert_X, sqlite_X, postgres_X)` triple collapses to one test: the `assert_X` body
becomes the test body, the two wrappers are deleted.

```rust
#[apply(backends)]
#[tokio::test]
async fn tag_normalization(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    // …former assert_tag_normalization body, unchanged…
}
```

The `let state = &env.state;` line is a deliberate convenience so the moved body needs no
identifier edits.

Refinements, handled per-test (not blindly):

- **Shared `assert_*` helpers with multiple real callers** (e.g. the `*_app_state_parity_suite`
  sequences) stay as helpers; only the 1:1 wrappers collapse.
- **Triples whose sqlite/postgres bodies diverge** (not thin wrappers over a shared assert)
  are converted individually.
- **Genuinely backend-specific tests** (the PG-only `open_database_*` / migration tests; any
  SQLite-only ones) use `#[apply(postgres_only)]` / `#[apply(sqlite_only)]` — the same shape,
  one case. They are not part of the dual matrix. The PG-only ones need a live PostgreSQL
  (they run in the coverage pass; a bare no-PG `cargo nextest` fails them — consistent with
  the post-B5 model).

### Files

- `server/tests/storage.rs` — ~90 triples.
- `server/tests/commands.rs` — 2.
- `server/tests/backup_interop.rs` — 1 (`postgres_backup_restores_into_sqlite`, plus a
  `postgres_storage_args` helper; assess whether it fits the matrix or stays bespoke).

### De-risking

The first task of the Part 1 plan is a **spike**: convert one real test and prove
`#[apply(template)]` + `#[tokio::test]` + `async` compose, pinning the exact attribute
ordering. Everything downstream depends on that composition, so it is confirmed before the
mass conversion begins.

---

## Part 2 — Value Parametrization

Different shape from Part 1: a **judgment-based, per-file** transform, not a uniform one.
Find clusters of tests that share one assertion shape and differ only by data, and collapse
each into a named `#[case]` table.

### Cluster types (from the survey)

- **Input → expected-rejection.** `create_post_rejects_*` (≈8 in `web_posts.rs`) → one test
  with `#[case::empty_body(...)]`, `#[case::missing_slug(...)]`, … each row a
  `(form, expected_error)`. Same for `update_post_rejects_*`.
- **Endpoint → shared policy.** `*_rejects_unauthenticated` (≈6 endpoints), `*_rejects_non_author`
  (≈4), `list_*_rejects_invalid_cursor_inputs` (×4), atompub's `*_forbids_other_user` (×4) →
  one test parameterized over the endpoint / request-builder.

### Rules

- **Collapse only when setup and assertion *structure* are identical and just the data varies.**
  If merging would need `if`-branching on the case inside the body, leave the tests separate.
- **Every case is named**, so failure output still identifies one scenario.
- **Per-file tasks.** Candidates: `web_posts.rs`, `atompub_posts.rs`, `web_auth.rs`,
  `media_handlers.rs`, with a scan of the rest (`web_account`, `web_backup`, `feed_*`, …).
  Not every file has clusters; the plan enumerates what it finds.

### Safety net

The line-identity coverage gate guards the whole refactor: if a parameterization silently
drops a real case, coverage of that branch falls and the ratchet fails.

---

## Coverage, Dependency, Verification

- **Dependency:** `rstest` in `[workspace.dependencies]`, referenced from `server`'s
  `[dev-dependencies]`. Confirm `cargo deny check` stays green.
- **Coverage:** unchanged — one nextest pass under the ephemeral PostgreSQL (B5). PG cases
  need that PG, which is already up.
- **Baseline:** `src`-only, expected stable. Heal benign shrink with `cargo xtask check`
  (Fix); investigate any regression as a dropped case.
- **Verification ladder:** convert in chunks; after each chunk `cargo xtask check --no-test`
  (fast compile + clippy, catches rstest macro mistakes immediately); a full
  `cargo xtask validate --no-e2e` for the green gate at the end of each plan.

## Sequencing

Two implementation plans, Part 1 first.

- **Part 1 plan:** spike → add `rstest` + `Backend`/`TestEnv` + the three templates →
  convert `storage.rs` in section-sized chunks (its natural sections: site-config/user/session/invite,
  posts, tags, cursors, media, user-config, rendered) → `commands.rs` + `backup_interop.rs` →
  backend-specific tests via the single-case templates → final `validate`.
- **Part 2 plan:** per-file cluster conversion, one file per task, coverage gate guarding each.
  Runs after Part 1, lands incrementally, and is safely deferrable — it is independent of
  Part 1's backend machinery.

## Risks

- **rstest × tokio × template composition** — mitigated by the spike (Part 1 task 0).
- **Diverging sqlite/postgres bodies** — handled individually during conversion, not assumed
  uniform.
- **Large-file churn** in `storage.rs` (~6400 lines) — convert in chunks, each verified to
  compile before moving on.
- **Baseline shifts** — guarded by the coverage gate; regressions investigated, never healed
  away.
