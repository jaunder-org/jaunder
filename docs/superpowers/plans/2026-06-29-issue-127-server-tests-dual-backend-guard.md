# Issue #127 — Dual-backend conversion + suite-wide guard across `server/tests` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every DB-touching test under `server/tests/**` backend-explicit, relocate/exempt the genuinely non-DB ones, and widen the `test-backend-pattern` guard to police the whole `server/tests` tree (with the parameterized-form / exemption-marker / contiguity hardening).

**Architecture:** Pure test-layer changes — convert `#[values(Backend::…)]` and backend-implicit tests to the shared `#[apply(backends)]` template routing through `backend.setup().await` → `env.state`; annotate genuinely single-backend tests `#[apply(postgres_only)]`/`#[apply(sqlite_only)]` with reasons; move unit-shaped non-DB tests into `#[cfg(test)]` unit modules and mark genuine non-DB integration tests `// guard:no-backend`. Finally rewrite the guard's `run()` from a single-file read to a `server/tests/**` directory walk and harden its scanner. No production source changes.

**Tech Stack:** Rust, `rstest`/`rstest_reuse` templates (`backends`/`sqlite_only`/`postgres_only`), `tokio::test`, `sqlx`, the `cargo xtask` driver (run via `devtool run -- cargo xtask …`).

## Global Constraints

- **Backend parity is the point.** Converted tests MUST pass on **both** backends; the Postgres case runs under the coverage pass (which sets `JAUNDER_PG_TEST_URL`). The integration suite is only ever run through the gate, which provisions PG — there is no supported no-PG run to accommodate, so just convert (drop the old `postgres_testing_enabled()` env-branch; no fallback/skip logic). This matches the 152 `#[apply(backends)]` tests #54 already landed.
- **Per-task gate = full `cargo xtask check`** (runs clippy + the Nix coverage pass incl. PostgreSQL). Iterate with `cargo xtask check --no-test` for fast clippy/dead-code; the commit gate is the full `cargo xtask check`. Run gates via `devtool run -- cargo xtask check` (worktree-aware, honest exit). Reserve `cargo xtask validate` for the final task.
- **Templates:** `backends`, `sqlite_only`, `postgres_only` exist at `storage/src/test_support.rs:156-178`, re-exported via `server/tests/helpers/mod.rs`. Task 2 adds **exactly one** new template, `backends_matrix` (a `#[values(Backend::Sqlite, Backend::Postgres)]`-based variant), because the `#[case]`-based `backends` template cannot compose with a test's own local `#[case]` axis (rstest requires every `#[case]` row to specify all `#[case]` params, so template-backend-cases collide with local scenario-cases). Spiked and confirmed (`.superpowers/sdd/task-2-spike.md`): `#[apply(backends_matrix)]` keeps the local `#[case]` matrix and yields the backend×case product. Define no other new templates.
- **Classification rule (binding):** "currently hardcodes SQLite / branches on an env var" is NOT a reason for `sqlite_only`/single-backend. A test may be tagged single-backend ONLY if, on reading it, it asserts backend-specific behavior the other backend structurally can't exhibit — recorded in its `// reason:` comment. Otherwise convert it. (Worked example: `feed_worker` is converted, not annotated; `feed_events_concurrency` is genuinely `sqlite_only` because it reproduces the SQLite #18 lock flake.)
- **The exemption marker for genuine non-DB integration tests:** a comment line `// guard:no-backend — <reason>` placed in the test's attribute block (immediately above the `#[tokio::test]`). The guard (Task 6) skips a bare test carrying it. Unit-shaped tests are MOVED out of `server/tests` entirely instead.
- **`-D dead_code`**: delete any helper/import a conversion orphans; the per-task check flags it.
- **Comment for intent**, not narration. Keep every assertion identical across a conversion — only backend plumbing changes.
- **No `Co-Authored-By` trailers.**
- **Worktree:** `.claude/worktrees/issue-127-server-tests-dual-backend-guard`, branch `worktree-issue-127-server-tests-dual-backend-guard`.

## Re-census helper (use at the start of every conversion task)

To list the bare `#[tokio::test]`s (incl. parameterized forms) in a file and their current template, run this from the worktree:

```
rg -nN -B3 '#\[tokio::test' server/tests/<path>.rs
```

A test is "bare" if no `#[apply(backends|sqlite_only|postgres_only)]` appears in the contiguous attribute block above its `#[tokio::test...]` line. Audit line numbers in this plan are approximate — match tests by name.

---

### Task 1: `misc/commands.rs` — backend-parameterize the helpers + convert 24 CLI tests + tag 3 PG-only

**Files:**
- Modify: `server/tests/misc/commands.rs` (helpers near L36–53; 27 bare tests across L143–779).

**Interfaces:**
- Consumes: `backends`/`postgres_only` templates, `Backend`, `sqlite_url`, `unique_postgres_url`, `nonexistent_postgres_url` (from `crate::helpers`), the `cmd_*` fns, `StorageArgs`.
- Produces: a backend-parameterized `storage_args(backend: Backend, base: &TempDir) -> StorageArgs` and `uninitialized_storage_args(backend: Backend, base: &TempDir) -> StorageArgs`; 24 `#[apply(backends)]` tests; 3 `#[apply(postgres_only)]` tests.

- [x] **Step 1: Re-census** `server/tests/misc/commands.rs` per the helper above; confirm the 29 bare tests and identify the 3 `cmd_create_pg_db_*` PG-only ones (near L189/L201/L266) vs the 26 others (24 DB + the 2 `cmd_create_pg_db_rejects_non_postgres_urls`-style — treat all 3 `cmd_create_pg_db_*` as `postgres_only`). NOTE: census found **26** helper-using DB tests (not 24); all 26 converted.

- [x] **Step 2: Backend-parameterize the helpers.** Change the env-branch to a backend match:

```rust
async fn storage_args(backend: Backend, base: &TempDir) -> StorageArgs {
    let storage_path = base.path().join("storage");
    let db = match backend {
        Backend::Sqlite => sqlite_url(base),
        Backend::Postgres => unique_postgres_url().await,
    };
    StorageArgs { storage_path, db }
}

fn uninitialized_storage_args(backend: Backend, base: &TempDir) -> StorageArgs {
    let storage_path = base.path().join("storage");
    let db = match backend {
        Backend::Sqlite => sqlite_url(base),
        Backend::Postgres => nonexistent_postgres_url(),
    };
    StorageArgs { storage_path, db }
}
```

Drop the now-unused `postgres_testing_enabled` import if it becomes orphaned.

- [x] **Step 3: Convert the 24 DB tests.** For each, add `#[apply(backends)]` above `#[tokio::test]`, add `#[case] backend: Backend` as a parameter, and pass `backend` into the helper call (`storage_args(backend, &base).await` / `uninitialized_storage_args(backend, &base)`). Bodies and assertions unchanged. Example:

```rust
#[apply(backends)]
#[tokio::test]
async fn cmd_init_on_fresh_dir_creates_structure_and_valid_db(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let args = storage_args(backend, &base).await;
    cmd_init(&args, false).await.unwrap();
    assert!(args.storage_path.is_dir());
    assert!(args.storage_path.join("media").is_dir());
    assert!(args.storage_path.join("backups").is_dir());
    open_database(&args.db).await.unwrap();
}
```

- [x] **Step 4: Tag the 3 `cmd_create_pg_db_*` tests** `#[apply(postgres_only)]` + `#[case] backend: Backend` + `let _ = backend;` + `// reason: provisions a Postgres role/database (needs PG admin/bootstrap); intrinsically Postgres.` Bodies unchanged.

- [x] **Step 5: Fast feedback** — `devtool run -- cargo xtask check --no-test`. Fix clippy/dead_code (delete any orphaned import). PASSED (exit 0); `postgres_testing_enabled` import dropped.

- [ ] **Step 6: Full per-task gate** — `devtool run -- cargo xtask check`. DEFERRED to controller (runs the PG/coverage passes).

- [ ] **Step 7: Commit** — DEFERRED to controller.

```bash
git add server/tests/misc/commands.rs
git commit -m "test(issue-127): run CLI command tests on both backends"
```

---

### Task 2: Add the `backends_matrix` template + standardize the 17 backend×matrix `#[values]` tests onto `#[apply(backends_matrix)]`

These 17 tests are NOT simple single-axis `#[values]` tests — each combines the backend axis with a LOCAL `#[case]` (or tuple-`#[case]`) matrix, so they legitimately use `#[values(Backend::Sqlite, Backend::Postgres)]` (the cartesian-product operator) and cannot use the `#[case]`-based `backends` template. The fix (spiked, `.superpowers/sdd/task-2-spike.md`): add one `#[values]`-based template and standardize them onto it, preserving their local matrices.

**Files (modify):** `storage/src/test_support.rs` (+the new template), `server/tests/helpers/mod.rs` (+re-export), then the 17 tests across `server/tests/web/web_posts.rs`, `web/web_media.rs`, `web/web_backup.rs`, `web/web_auth.rs`, `atompub/atompub_posts.rs`, `atompub/atompub_media.rs`, `atompub/atompub_rsd.rs`, `feed/feed_events_hook.rs`, `feed/feed_handlers.rs`, `misc/media_handlers.rs`. Re-census each file (`rg -nN -B6 '#\[tokio::test' …`) to find its `#[values(Backend::Sqlite, Backend::Postgres)]` tests.

**Interfaces:**
- Produces: `pub fn backends_matrix(#[values(Backend::Sqlite, Backend::Postgres)] backend: Backend) {}` (a `#[template]`), re-exported as `backends_matrix`; each of the 17 tests tagged `#[apply(backends_matrix)]`.
- Consumed by: Task 6's guard, which MUST add `#[apply(backends_matrix)]` to its accepted set.

- [ ] **Step 1: Define the template** in `storage/src/test_support.rs`, immediately after the `backends` template (~L173-178):

```rust
/// Dual-backend matrix template: a `#[values]`-based backend axis that composes
/// with a test's own local `#[case]`/`#[values]` matrix (the `#[case]`-based
/// `backends` template cannot — its case rows collide with local case rows).
#[template]
#[export]
#[rstest]
pub fn backends_matrix(#[values(Backend::Sqlite, Backend::Postgres)] backend: Backend) {}
```

- [ ] **Step 2: Re-export it** in `server/tests/helpers/mod.rs` — add `backends_matrix` to the `pub use storage::test_support::{ … }` list alongside `backends`.

- [ ] **Step 3: Convert the 17 tests.** For each, remove the `#[values(Backend::Sqlite, Backend::Postgres)]` attribute from the `backend: Backend` parameter (leaving a plain `backend: Backend,` param) and add `#[apply(backends_matrix)]` immediately above `#[tokio::test]`. Keep the local `#[case]`/`#[rstest]` rows and the body unchanged. Before/after:

```rust
// before
#[case::list_drafts(UnauthEndpoint::ListDrafts)]
// … more #[case] rows …
#[tokio::test]
async fn endpoint_rejects_unauthenticated(
    #[values(Backend::Sqlite, Backend::Postgres)] backend: Backend,
    #[case] endpoint: UnauthEndpoint,
) { … }

// after
#[apply(backends_matrix)]
#[case::list_drafts(UnauthEndpoint::ListDrafts)]
// … more #[case] rows …
#[tokio::test]
async fn endpoint_rejects_unauthenticated(
    backend: Backend,
    #[case] endpoint: UnauthEndpoint,
) { … }
```

(If a bare test in these files has NO `#[values(Backend…)]` arg and is single-axis, it should already be `#[apply(backends)]`; if it's bare and single-axis, it belongs to Task 3/4/5 — stop and report it.)

- [ ] **Step 4: Fast feedback** — `cargo xtask check --no-test` (bare, via Bash, worktree-aware). Confirm it compiles; optionally `cargo nextest list -p jaunder --test web 2>/dev/null | rg endpoint_rejects_unauthenticated` to confirm the backend×case product is preserved.

- [ ] **Step 5: Full per-task gate** — `cargo xtask check`. Expected: green; each converted test expands to backend × its local cases.

- [ ] **Step 6: Commit**

```bash
git add storage/src/test_support.rs server/tests/helpers/mod.rs server/tests/web server/tests/atompub server/tests/feed server/tests/misc/media_handlers.rs
git commit -m "test(issue-127): add backends_matrix template; standardize backend-matrix tests onto it"
```

---

### Task 3: Convert `feed_worker::worker_applies_backoff_on_ping_failure` (incidental SQLite → dual-backend)

**Files:**
- Modify: `server/tests/feed/feed_worker.rs` (the test near L258, plus its hand-built `AppState` preamble L260–307).

**Interfaces:**
- Consumes: `backends` template, `Backend`, `backend.setup().await` → `env.state`.
- Produces: the test as `#[apply(backends)]`, routing through `env.state` instead of a hand-built `Sqlite*` `AppState`.

- [ ] **Step 1: Read the test** (L258–end) to capture the `FailingWebSubClient`, the worker invocation, and the backoff assertions.

- [ ] **Step 2: Replace the preamble.** Delete the `TempDir` + `SqlitePool::connect_with` + `sqlx::migrate!` + hand-built `storage::AppState { … Sqlite*Storage … }` block, and substitute:

```rust
#[apply(backends)]
#[tokio::test]
async fn worker_applies_backoff_on_ping_failure(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = env.state.clone();
    // … FailingWebSubClient + worker run + backoff assertions, unchanged,
    //    using `state` (Arc<AppState>) where the old `state` local was used …
}
```

Keep the `FailingWebSubClient` impl and every assertion identical. If the body used the raw `pool` directly (not just `state`), route that raw SQL through the dual-backend `raw_exec`/`raw_try_exec` helpers; if the body only used `state`, no further change.

- [ ] **Step 3: Fast feedback** — `devtool run -- cargo xtask check --no-test`. Delete any now-orphaned `Sqlite*`/`SqlitePool`/`sqlx::migrate` imports the conversion leaves unused.

- [ ] **Step 4: Full per-task gate** — `devtool run -- cargo xtask check`. Expected: green on both backends — this closes the feed-worker backoff PG coverage hole.

- [ ] **Step 5: Commit**

```bash
git add server/tests/feed/feed_worker.rs
git commit -m "test(issue-127): run feed-worker ping-failure backoff on both backends"
```

---

### Task 4: Annotate the genuinely single-backend tests (verify each by reading)

**Files:**
- Modify: `server/tests/misc/backup_interop.rs` (×2, near L127/L160), `server/tests/misc/pg_teardown.rs` (×1, near L34), `server/tests/feed/feed_events_concurrency.rs` (×1, near L18).

**Interfaces:**
- Consumes: `postgres_only`/`sqlite_only` templates.
- Produces: each test tagged with the correct single-backend template + a `// reason:`.

- [ ] **Step 1: Read each test and confirm the intentional reason** (per the classification rule — do NOT tag on "hardcodes X"):
  - `backup_interop` ×2 (`sqlite_backup_restores_into_postgres`, `postgres_backup_restores_into_sqlite`): cross-backend backup/restore exercises BOTH engines in one test → `postgres_only` (requires live PG). Reason: cross-backend backup interop, needs both engines.
  - `pg_teardown::per_test_database_is_dropped_on_teardown`: asserts Postgres per-test-database teardown → `postgres_only`. Reason: PG-specific per-test DB drop.
  - `feed_events_concurrency::claim_pending_batch_no_lock_contention`: reproduces the SQLite #18 `claim_pending_batch` lock flake (reserved-lock upgrade under `busy_timeout`) → `sqlite_only`. Reason as below. (Postgres MVCC can't exhibit it.)

- [ ] **Step 2: Tag them.** Add the template + `#[case] backend: Backend` (consume with `let _ = backend;` only where the body doesn't already use it) + a `// reason:` line. For `feed_events_concurrency`, **preserve** its `#[tokio::test(flavor = "multi_thread", worker_threads = 4)]` and `#[ignore = …]` attributes; the result is:

```rust
#[apply(sqlite_only)]
// reason: reproduces the SQLite-specific issue #18 claim_pending_batch lock flake
// (reserved-lock upgrade under busy_timeout); Postgres MVCC cannot exhibit it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "timing-based #18 reproduction; run manually with --ignored"]
async fn claim_pending_batch_no_lock_contention(#[case] backend: Backend) {
    let _ = backend; // sqlite_only template supplies Backend::Sqlite
    let env = Backend::Sqlite.setup().await; // unchanged body …
}
```

(The `sqlite_only` template's case is `Backend::Sqlite`; the body may keep `Backend::Sqlite.setup()` explicitly — leave the existing body as-is and just add the attributes + `let _ = backend;`.)

- [ ] **Step 3: Fast feedback** — `devtool run -- cargo xtask check --no-test`.

- [ ] **Step 4: Full per-task gate** — `devtool run -- cargo xtask check`. Expected: green (the `#[ignore]`d test still compiles; the `postgres_only` tests run their PG case).

- [ ] **Step 5: Commit**

```bash
git add server/tests/misc/backup_interop.rs server/tests/misc/pg_teardown.rs server/tests/feed/feed_events_concurrency.rs
git commit -m "test(issue-127): annotate genuinely single-backend server tests with reasons"
```

---

### Task 5: Non-DB tests — relocate unit-shaped ones, exempt genuine non-DB integration ones

**Files:**
- Read then act on: `server/tests/misc/static_assets.rs` (×2, L47/L58), `server/tests/web/web_auth.rs` (the `auth_user_extraction_fails_without_session_storage_extension` test, ~L918).
- Possibly create/modify: a `#[cfg(test)]` module in the crate that owns the code under test (e.g. `server/src/…` or `web/src/…`) for any relocated unit test.

**Interfaces:**
- Produces: unit-shaped non-DB tests moved OUT of `server/tests` into a `#[cfg(test)] mod tests` in the owning crate; genuine non-DB integration tests left in place with a `// guard:no-backend — <reason>` marker.

- [ ] **Step 1: Read each of the 3 tests and decide relocate-vs-exempt** per the policy:
  - If the test calls a pure function / extractor directly with no router/app wiring → **unit-shaped → relocate.**
  - If it exercises integration wiring (an axum route, the asset router) but touches no DB → **genuine non-DB integration → exempt in place.**

- [ ] **Step 2a (relocate):** For each unit-shaped test, move it verbatim into a `#[cfg(test)] mod tests { … }` in the source file that owns the function it tests (adjust `use` paths from `crate::`-test-style to the in-crate module paths; the assertion body is unchanged). Delete it from `server/tests`. If a whole `server/tests/<file>.rs` becomes empty, delete the file and drop its `mod` line from the test entrypoint (e.g. `server/tests/misc.rs`).

- [ ] **Step 2b (exempt):** For each genuine non-DB integration test, add the marker line in its attribute block:

```rust
// guard:no-backend — serves an embedded static asset; exercises no database.
#[tokio::test]
async fn test_jaunder_css_served() { … }
```

- [ ] **Step 3: Fast feedback** — `devtool run -- cargo xtask check --no-test`. Fix imports/`mod` wiring from any relocation.

- [ ] **Step 4: Full per-task gate** — `devtool run -- cargo xtask check`. Expected: green; relocated tests run as unit tests, exempted ones remain.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(issue-127): relocate unit-shaped non-DB tests; mark genuine non-DB integration tests guard:no-backend"
```

---

### Task 6: Widen + harden the `test-backend-pattern` guard; document the marker; final validate

**Files:**
- Modify: `xtask/src/steps/test_pattern_check.rs` (the scanner + `run()` + fixture tests).
- Modify: `CONTRIBUTING.md` (testing section — document the suite-wide guard + the `// guard:no-backend` marker + the single-backend `// reason:` requirement).

**Interfaces:**
- Consumes: `CommandResult`/`StepResult`.
- Produces: a guard that walks `server/tests/**/*.rs`, matches parameterized tokio forms, honors the exemption marker, tolerates attribute-block contiguity, and hard-fails on a missing root.

- [ ] **Step 1: Write the failing fixture unit tests** in `test_pattern_check.rs` covering the new behaviors. Add to the existing `#[cfg(test)] mod tests`:

```rust
const PARAM_BARE: &str = "\
#[tokio::test(flavor = \"multi_thread\")]
async fn bad_param() {}
";
const PARAM_TAGGED: &str = "\
#[apply(sqlite_only)]
#[tokio::test(flavor = \"multi_thread\")]
async fn good_param(#[case] backend: Backend) {}
";
const EXEMPT: &str = "\
// guard:no-backend — serves a static asset; no DB.
#[tokio::test]
async fn no_db() {}
";
const DOC_GAP: &str = "\
#[apply(backends)]
/// doc comment between the template and the test
#[tokio::test]
async fn good_with_doc(#[case] backend: Backend) {}
";
const MATRIX_TAGGED: &str = "\
#[apply(backends_matrix)]
#[case::a(1)]
#[tokio::test]
async fn good_matrix(backend: Backend, #[case] n: i32) {}
";

#[test] fn parameterized_bare_is_flagged() { assert_eq!(violations(PARAM_BARE), vec![1]); }
#[test] fn parameterized_tagged_is_clean() { assert!(violations(PARAM_TAGGED).is_empty()); }
#[test] fn no_backend_marker_exempts() { assert!(violations(EXEMPT).is_empty()); }
#[test] fn doc_comment_between_template_and_test_is_clean() { assert!(violations(DOC_GAP).is_empty()); }
#[test] fn backends_matrix_apply_is_clean() { assert!(violations(MATRIX_TAGGED).is_empty()); }
```

- [ ] **Step 2: Run them, verify they fail** — `devtool run -- cargo test --manifest-path xtask/Cargo.toml test_pattern_check`. Expected: the 4 new tests FAIL (current scanner matches only exact `#[tokio::test]`, has no marker/contiguity logic).

- [ ] **Step 3: Harden the scanner.** Rewrite `violations` so that: a line is a tokio test if its trimmed text is `#[tokio::test]` OR starts with `#[tokio::test(`; the attribute-block walk (up and down) steps over lines that are `#[…]` attributes OR blank OR `//`/`///` comments (stopping at the first other code line / the `fn`); the test is satisfied if the block contains an accepted `#[apply(…)]` OR a `// guard:no-backend` comment; otherwise it is a violation at the tokio line. **Extend `is_backend_apply` to also accept `#[apply(backends_matrix)]`** (the `#[values]`-based dual-backend template from Task 2 — note `#[apply(backends_matrix)]` is NOT a substring of `#[apply(backends)]`, so it must be listed explicitly):

```rust
fn is_backend_apply(trimmed: &str) -> bool {
    trimmed.contains("#[apply(backends)]")
        || trimmed.contains("#[apply(backends_matrix)]")
        || trimmed.contains("#[apply(sqlite_only)]")
        || trimmed.contains("#[apply(postgres_only)]")
}
fn is_attr_or_skippable(trimmed: &str) -> bool {
    trimmed.is_empty() || trimmed.starts_with("#[") || trimmed.starts_with("//")
}
fn is_tokio_test(trimmed: &str) -> bool {
    trimmed == "#[tokio::test]" || trimmed.starts_with("#[tokio::test(")
}
fn is_exempt_or_tagged(trimmed: &str) -> bool {
    is_backend_apply(trimmed) || trimmed.starts_with("// guard:no-backend")
}
fn violations(source: &str) -> Vec<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !is_tokio_test(line.trim()) { continue; }
        let mut ok = false;
        // walk up across the contiguous attribute/comment/blank block
        let mut j = i;
        while j > 0 && is_attr_or_skippable(lines[j - 1].trim()) {
            j -= 1;
            if is_exempt_or_tagged(lines[j].trim()) { ok = true; break; }
        }
        if !ok {
            // walk down across attributes until the fn
            let mut k = i + 1;
            while k < lines.len() && is_attr_or_skippable(lines[k].trim()) {
                if is_exempt_or_tagged(lines[k].trim()) { ok = true; break; }
                k += 1;
            }
        }
        if !ok { out.push(i + 1); }
    }
    out
}
```

(Keep the existing `is_backend_apply` helper. Note `is_attr_or_skippable` now also bounds the walk on blanks/comments — verify the existing `annotated_tokio_test_is_clean`/`bare_tokio_test_is_flagged_at_its_line`/`sync_unit_test_is_exempt` tests still pass.)

- [ ] **Step 4: Run unit tests, verify they pass** — `devtool run -- cargo test --manifest-path xtask/Cargo.toml test_pattern_check`. Expected: all (old + 4 new) PASS.

- [ ] **Step 5: Widen `run()` to a directory walk.** Replace the `const SCANNED: &[&str]` single-file read with a recursive walk of `server/tests`, hard-failing if the root is absent:

```rust
const TEST_ROOT: &str = "server/tests";

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_dir() { rust_files(&p, out)?; }
        else if p.extension().is_some_and(|e| e == "rs") { out.push(p); }
    }
    Ok(())
}

pub fn run(result: &mut CommandResult) {
    let mut files = Vec::new();
    let step = match rust_files(Path::new(TEST_ROOT), &mut files) {
        Err(e) => StepResult::fail("test-backend-pattern")
            .detail(format!("cannot scan {TEST_ROOT}: {e}")),
        Ok(()) => {
            let scanned: Vec<(String, String)> = files
                .iter()
                .filter_map(|p| {
                    std::fs::read_to_string(p)
                        .ok()
                        .map(|s| (p.display().to_string(), s))
                })
                .collect();
            match problems(&scanned) {
                None => StepResult::ok("test-backend-pattern"),
                Some(detail) => StepResult::fail("test-backend-pattern").detail(detail),
            }
        }
    };
    result.push(step);
}
```

- [ ] **Step 6: Verify the guard passes against the now-clean tree** — `devtool run -- cargo xtask check --no-test`. Expected: `test-backend-pattern` is `ok` (Tasks 1–5 left zero unannotated bare/parameterized tokio tests in `server/tests`). If it fails, it prints the offending `file:line` — fix the straggler (it was missed by an earlier task) before proceeding.

- [ ] **Step 7: Document the convention** in `CONTRIBUTING.md`'s testing section: every DB-touching `server/tests` test carries `#[apply(backends|sqlite_only|postgres_only)]` (single-backend requires a `// reason:`); genuine non-DB integration tests carry `// guard:no-backend — <reason>`; unit-shaped tests live in `#[cfg(test)]` modules, not `server/tests`. The `test-backend-pattern` guard enforces this across `server/tests`.

- [ ] **Step 8: Final full local gate** — `devtool run -- cargo xtask validate`. Expected: green, incl. both backends across the suite and the e2e matrix.

- [ ] **Step 9: Commit**

```bash
git add xtask/src/steps/test_pattern_check.rs CONTRIBUTING.md
git commit -m "feat(issue-127): widen test-backend-pattern guard to all of server/tests + harden scanner"
```

---

## Notes for the implementer

- **Read before converting:** match tests by name; line numbers are approximate and shift as files are edited.
- **No new separable concerns to file:** the sibling work (storage-crate guard + dialect tests, backup-on-PG) is already tracked as #135 / #136.
- **If a "convertible" test turns out to assert backend-specific behavior on reading** (the reverse of the feed_worker case), annotate it single-backend with a `// reason:` and note it — the classification rule cuts both ways.

## Self-review

- **Spec coverage:** Part A (20 `#[values]`) → Task 2; the feed_worker conversion → Task 3; Part B (commands.rs 24+3) → Task 1; Part C (single-backend annotations) → Task 4; Part D (non-DB relocate/exempt) → Task 5; Part E (guard widen+harden+CONTRIBUTING) → Task 6; the classification rule is in Global Constraints and exercised in Tasks 3/4. ✓
- **Placeholder scan:** none — the guard rewrite and helper changes carry complete code; conversions name exact tests/templates and show the transform. Per-test bodies are read-then-transform (a conversion), so they are not pre-pasted. ✓
- **Type consistency:** `violations(source) -> Vec<usize>`, `problems(&[(String,String)]) -> Option<String>`, helpers `is_attr_or_skippable`/`is_tokio_test`/`is_exempt_or_tagged`/`is_backend_apply`, `rust_files(dir, out)`, step name `test-backend-pattern`, `TEST_ROOT="server/tests"`; helper `storage_args(backend, base)`/`uninitialized_storage_args(backend, base)`. Consistent across tasks. ✓
