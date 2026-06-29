# Issue #54 — Dual-backend storage test coverage + uniform-pattern guard — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert the backend-agnostic SQLite-only tests in `server/tests/storage/storage.rs` to run on both backends, annotate the genuinely single-backend ones via the standard rstest mechanism, and add a configurable xtask guard that fails CI if a new `#[tokio::test]` lacks a backend template.

**Architecture:** Each agnostic `#[tokio::test]` is rewritten to `#[apply(backends)] … (#[case] backend: Backend)`, obtaining `let env = backend.setup().await; let state = &env.state;` and routing through the shared `Arc<AppState>` handles instead of direct `Sqlite*` types. Orphaned SQLite-typed helpers are deleted (enforced by `-D dead_code`). The guard is a new `xtask/src/steps/test_pattern_check.rs` modeled exactly on `xtask/src/steps/sequence_check.rs`: a pure, fixture-tested `problems()` function plus a `run()` that scans a configurable path set and pushes a `StepResult`.

**Tech Stack:** Rust, `rstest` / `rstest_reuse` templates (`backends`/`sqlite_only`/`postgres_only`), `tokio::test`, `sqlx`, the `cargo xtask` dev/CI driver.

## Global Constraints

- **Backend parity is the point of this issue.** Converted tests MUST pass on **both** SQLite and Postgres. The Postgres case runs only when `JAUNDER_PG_TEST_URL` is set — the `cargo xtask check` Nix coverage pass sets it, so the per-task gate genuinely exercises Postgres.
- **Per-task gate = full `cargo xtask check`** (NOT `--no-test`). It runs clippy + the Nix coverage pass (instrumented tests incl. PostgreSQL + the coverage gate). Iterate with `cargo xtask check --no-test` for fast clippy/dead-code feedback, but the commit gate is the full `cargo xtask check`. Reserve `cargo xtask validate` for the final task.
- **`-D dead_code` is in force for tests.** When a conversion removes the last caller of a SQLite-typed helper, that helper becomes a hard build error. **Standing rule for every conversion task:** after converting a cluster, if the per-task check reports a now-unused helper fn, delete that fn in the same commit and re-run until clean. The compiler enforces completeness — trust it.
- **Existing templates only** — `backends`, `sqlite_only`, `postgres_only` already exist at `storage/src/test_support.rs:156-178` and are re-exported via `server/tests/helpers/mod.rs`. Do NOT define new templates.
- **The shared handles** on `state: &Arc<AppState>`: `state.atomic` (AtomicOps), `state.users` (UserStorage), `state.sessions` (SessionStorage), `state.invites` (InviteStorage), `state.email_verifications` (EmailVerificationStorage), `state.password_resets` (PasswordResetStorage), `state.site_config` (SiteConfigStorage).
- **Raw-SQL escape hatch for corrupt-data setups:** the file already has dual-backend `raw_exec(backend, env, sql)` and `raw_try_exec(backend, env, sql)` helpers (used by retained `#[apply(backends)]` tests). Use these for tests that must corrupt/seed rows directly. If a corrupt-data setup genuinely cannot be expressed portably across both dialects, annotate that single test `#[apply(sqlite_only)]` with a `// reason:` comment rather than forcing it — and note it in the task's commit message.
- **No `Co-Authored-By` trailers** in any commit.
- **Worktree:** all work happens in `.claude/worktrees/issue-54-storage-dual-backend-coverage` on branch `worktree-issue-54-storage-dual-backend-coverage`.

## Worked transform (the pattern every conversion follows)

Before (SQLite-only, direct instantiation):

```rust
#[tokio::test]
async fn create_user_with_invite_expired_returns_invite_expired() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;
    let invites = SqliteInviteStorage::new(pool.clone());
    let code = invites.create_invite(/* already-expired */).await.unwrap();
    let err = SqliteAtomicOps::new(pool.clone())
        .create_user_with_invite(&username("dave"), &password("password123"), None, false, &code)
        .await
        .unwrap_err();
    assert!(matches!(err, AtomicError::InviteExpired));
    // rollback assertion: no user row, invite still unused …
}
```

After (dual-backend, shared handles):

```rust
#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_expired_returns_invite_expired(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let code = state.invites.create_invite(/* already-expired */).await.unwrap();
    let err = state
        .atomic
        .create_user_with_invite(&username("dave"), &password("password123"), None, false, &code)
        .await
        .unwrap_err();
    assert!(matches!(err, AtomicError::InviteExpired));
    // rollback assertion via state.users / raw_exec(backend, &env, …) …
}
```

Mechanical rules:
1. Add `#[apply(backends)]` immediately above `#[tokio::test]`; add `#[case] backend: Backend` as the sole fn parameter.
2. Replace the `TempDir`/`open_pool`/`Sqlite*::new` preamble with `let env = backend.setup().await; let state = &env.state;`.
3. Replace each `SqliteXStorage`/`SqliteAtomicOps` call site with the matching `state.<handle>` call.
4. Replace direct-pool raw SQL with `raw_exec(backend, &env, sql)` / `raw_try_exec(backend, &env, sql)`.
5. Keep every assertion identical — behavior is unchanged; only the backend plumbing changes.

---

### Task 1: Convert `AtomicOps::create_user_with_invite` error/rollback cluster (the headline hole)

**Files:**
- Modify: `server/tests/storage/storage.rs` (tests near L1294–1451): `create_user_with_invite_creates_user_and_marks_invite_used`, `create_user_with_invite_second_call_returns_already_used`, `create_user_with_invite_expired_returns_invite_expired`, `create_user_with_invite_unknown_code_returns_not_found`, `create_user_with_invite_duplicate_username_returns_username_taken`.

**Interfaces:**
- Consumes: existing `backends` template, `state.atomic.create_user_with_invite`, `state.invites.create_invite`, `state.users.*`, `username()`/`password()`, `raw_exec`/`raw_try_exec`.
- Produces: 5 dual-backend tests; possibly the first reduction in `open_pool`/`SqliteAtomicOps`/`SqliteInviteStorage` use counts (do not delete yet unless count hits zero).

- [x] **Step 1: Read the five current test bodies** at the line ranges above to capture each one's exact setup + assertions.

- [x] **Step 2: Convert each test** following the worked transform. For the `duplicate_username` test, pre-create the colliding user via `state.users.create_user(...)`. For the rollback assertions ("no user created", "invite left unused"), assert via `state.users.get_user(...)` / `state.invites.list_invites(...)` (or `raw_exec` count) — same assertion, dual-backend handle.

- [x] **Step 3: Fast feedback** — `cargo xtask check --no-test` (clippy + dead_code). If it flags a now-unused helper, delete it (per the standing rule) and re-run. _(Removed now-unused `AtomicOps` and `SqliteAtomicOps` imports; re-ran clean.)_

- [ ] **Step 4: Full per-task gate** — `cargo xtask check`. _(Deferred per dispatch Execution Note: controller runs the full gate; this dispatch's gate is `cargo xtask check --no-test` only.)_

- [ ] **Step 5: Commit** _(Deferred per dispatch Execution Note: controller commits.)_

```bash
git add server/tests/storage/storage.rs
git commit -m "test(issue-54): run create_user_with_invite error/rollback paths on both backends"
```

---

### Task 2: Convert `EmailVerificationStorage::use_email_verification` cluster

**Files:**
- Modify: `server/tests/storage/storage.rs` (tests near L1537–1660): `create_email_verification_and_use_returns_user_id_and_email`, `second_email_verification_supersedes_first`, `use_email_verification_already_used_returns_already_used`, `use_email_verification_expired_returns_expired`, `use_email_verification_unknown_token_returns_not_found`, `use_email_verification_with_corrupt_stored_email_returns_internal`.

**Interfaces:**
- Consumes: `state.email_verifications.*`, `state.users.*`, `raw_exec`/`raw_try_exec`.
- Produces: 6 dual-backend (or `sqlite_only`-annotated, see note) tests; likely exhausts `email_verification_storage` helper → delete it this task.

- [ ] **Step 1: Read the six current test bodies** (note the `email_verification_storage(base)` helper they share).

- [ ] **Step 2: Convert each** via the worked transform. For `use_email_verification_with_corrupt_stored_email_returns_internal`, corrupt the stored row with `raw_exec(backend, &env, "UPDATE … SET … ")` using portable SQL. **If** the corruption cannot be expressed portably, annotate just that one test `#[apply(sqlite_only)]` with `// reason: corrupts stored ciphertext via SQLite-specific bytes` and note it in the commit message.

- [ ] **Step 3: Delete the now-orphaned `email_verification_storage` helper** (it was used only by this cluster). Confirm with `cargo xtask check --no-test` — dead_code must be clean.

- [ ] **Step 4: Full per-task gate** — `cargo xtask check`. Expected: green on both backends.

- [ ] **Step 5: Commit**

```bash
git add server/tests/storage/storage.rs
git commit -m "test(issue-54): run email-verification storage paths on both backends"
```

---

### Task 3: Convert `PasswordResetStorage` create/use cluster

**Files:**
- Modify: `server/tests/storage/storage.rs` (tests near L1729–1788): `create_password_reset_and_use_returns_user_id`, `use_password_reset_already_used_returns_already_used`, `use_password_reset_expired_returns_expired`, `use_password_reset_unknown_token_returns_not_found`.

**Interfaces:**
- Consumes: `state.password_resets.*`, `state.users.*`, `raw_exec`.
- Produces: 4 dual-backend tests; likely exhausts `password_reset_storage` helper → delete it this task.

- [ ] **Step 1: Read the four current test bodies** (shared `password_reset_storage(base)` helper).

- [ ] **Step 2: Convert each** via the worked transform, routing through `state.password_resets` and `state.users`.

- [ ] **Step 3: Delete the now-orphaned `password_reset_storage` helper.** Confirm with `cargo xtask check --no-test`.

- [ ] **Step 4: Full per-task gate** — `cargo xtask check`. Expected: green on both backends.

- [ ] **Step 5: Commit**

```bash
git add server/tests/storage/storage.rs
git commit -m "test(issue-54): run password-reset storage paths on both backends"
```

---

### Task 4: Convert `InviteStorage` lifecycle cluster

**Files:**
- Modify: `server/tests/storage/storage.rs` (tests near L1203–1290): `create_invite_and_list_invites_includes_it`, `use_invite_with_valid_code_marks_it_used`, `use_invite_with_unknown_code_returns_not_found`, `use_invite_with_expired_code_returns_expired`, `use_invite_on_already_used_code_returns_already_used`.

**Interfaces:**
- Consumes: `state.invites.*`, `raw_exec`.
- Produces: 5 dual-backend tests; likely exhausts `invite_storage_triple` helper → delete it this task.

- [ ] **Step 1: Read the five current test bodies** (shared `invite_storage_triple(base)` helper).

- [ ] **Step 2: Convert each** via the worked transform, routing through `state.invites`. For expired-code setup, create with an in-the-past expiry through the same handle.

- [ ] **Step 3: Delete the now-orphaned `invite_storage_triple` helper.** Confirm with `cargo xtask check --no-test`.

- [ ] **Step 4: Full per-task gate** — `cargo xtask check`. Expected: green on both backends.

- [ ] **Step 5: Commit**

```bash
git add server/tests/storage/storage.rs
git commit -m "test(issue-54): run invite-storage lifecycle on both backends"
```

---

### Task 5: Convert `UserStorage` detail cluster

**Files:**
- Modify: `server/tests/storage/storage.rs` (tests near L965–1093 and L1483–1531): `create_user_succeeds_and_get_by_username_returns_record`, `duplicate_username_returns_username_taken`, `authenticate_correct_password_returns_record_and_sets_last_authenticated_at`, `authenticate_wrong_password_returns_invalid_credentials`, `authenticate_unknown_username_returns_invalid_credentials`, `update_profile_persists_changes`, `get_user_unknown_id_returns_none`, `set_email_persists_and_get_user_reflects_it`, `set_email_clears_previously_set_email`.

**Interfaces:**
- Consumes: `state.users.*`, `raw_exec`.
- Produces: 9 dual-backend tests; reduces `user_storage`/`SqliteUserStorage` counts (delete `user_storage` only if this exhausts it — `storage_pair` may still use it, in which case delete in Task 6).

- [ ] **Step 1: Read the nine current test bodies** (shared `user_storage(base)` helper).

- [ ] **Step 2: Convert each** via the worked transform, routing through `state.users`.

- [ ] **Step 3: Fast feedback + helper cleanup** — `cargo xtask check --no-test`; delete any helper the compiler now reports unused.

- [ ] **Step 4: Full per-task gate** — `cargo xtask check`. Expected: green on both backends.

- [ ] **Step 5: Commit**

```bash
git add server/tests/storage/storage.rs
git commit -m "test(issue-54): run user-storage detail paths on both backends"
```

---

### Task 6: Convert `SessionStorage` detail + `SiteConfig` get/set + `build_mailer`

**Files:**
- Modify: `server/tests/storage/storage.rs`: session tests near L1104–1174 (`create_session_then_authenticate_returns_correct_record`, `revoke_session_then_authenticate_returns_session_not_found`, `list_sessions_returns_only_sessions_for_given_user`); SiteConfig tests near L617–644 (`get_missing_key_returns_none`, `set_overwrites_existing_value`, `second_open_on_migrated_database_succeeds`); `build_mailer_returns_noop_when_smtp_not_configured` (near L1461).

**Interfaces:**
- Consumes: `state.sessions.*`, `state.users.*`, `state.site_config.*`, `jaunder::mailer::build_mailer(state.site_config.as_ref())`.
- Produces: 7 dual-backend tests; exhausts `storage_pair` and `user_storage` (if not already gone) → delete this task. For `build_mailer`, the body becomes `let env = backend.setup().await; let mailer = jaunder::mailer::build_mailer(env.state.site_config.as_ref()).await; …` — same assertion.

- [ ] **Step 1: Read the seven current test bodies.** Note `second_open_on_migrated_database_succeeds` re-opens the SAME database; express via `backend.setup()` then a second `open_*` on `env.base` (per `Backend::setup` plumbing). If a second open needs the backend URL, get it from `env.base` rather than a hardcoded SQLite path.

- [ ] **Step 2: Convert each** via the worked transform. `build_mailer` routes through `env.state.site_config`.

- [ ] **Step 3: Delete now-orphaned `storage_pair` and `user_storage` helpers.** Confirm with `cargo xtask check --no-test` — dead_code clean.

- [ ] **Step 4: Full per-task gate** — `cargo xtask check`. Expected: green on both backends.

- [ ] **Step 5: Commit**

```bash
git add server/tests/storage/storage.rs
git commit -m "test(issue-54): run session/site-config/mailer paths on both backends"
```

---

### Task 7: Annotate the genuinely single-backend tests + remove the last orphaned helpers

**Files:**
- Modify: `server/tests/storage/storage.rs`: the Postgres-init/migration tests `open_database_succeeds_on_postgres_test_vm`, `open_database_runs_postgres_migrations_on_existing_empty_db`, `open_existing_database_runs_postgres_migrations_on_unmigrated_db` (near L886–) and any remaining bare `#[tokio::test]`.

**Interfaces:**
- Consumes: `postgres_only` template, `env.base` for the backend URL.
- Produces: the file's end state — every `#[tokio::test]` carries one of the three templates; `open_pool` and `open_pg_pool` (and any remaining `Sqlite*`/`Pg*` helper) deleted.

- [ ] **Step 1: Enumerate remaining bare `#[tokio::test]`** in the file:

```bash
rg -nN '#\[tokio::test\]' server/tests/storage/storage.rs
```

For each hit, the immediately-preceding line must be an `#[apply(...)]`. List any that are not yet annotated — these are the genuinely single-backend stragglers (the PG-init/migration tests) plus anything missed in Tasks 1–6.

- [ ] **Step 2: Annotate the PG-specific tests** `#[apply(postgres_only)]` with `// reason: exercises Postgres migration/open path specifically` and route their body through `backend.setup()` / `env.base` (drop the hardcoded `open_pg_pool()` preamble). If any test is truly SQLite-specific, use `#[apply(sqlite_only)]` + a `// reason:` instead. Convert any straggler caught here per its nature (agnostic → `backends`).

- [ ] **Step 3: Delete the final orphaned helpers** `open_pool`, `open_pg_pool`, and any remaining `Sqlite*`/`Pg*`-typed helper. Run `cargo xtask check --no-test`; iterate until dead_code is clean (the compiler lists every remaining orphan).

- [ ] **Step 4: Confirm zero bare `#[tokio::test]`** — re-run the Step-1 `rg`; every hit must now be preceded by an `#[apply(...)]`.

- [ ] **Step 5: Full per-task gate** — `cargo xtask check`. Expected: green on both backends.

- [ ] **Step 6: Commit**

```bash
git add server/tests/storage/storage.rs
git commit -m "test(issue-54): annotate single-backend storage tests + drop SQLite-only helpers"
```

---

### Task 8: Add the uniform-pattern guard (new xtask check) + final validate

**Files:**
- Create: `xtask/src/steps/test_pattern_check.rs`
- Modify: `xtask/src/steps/mod.rs` (register the new module — match how `sequence_check` is declared)
- Modify: `xtask/src/lib.rs` (push `test_pattern_check::run(&mut result)` in both the `Check` and `Validate` flows, immediately after `steps::sequence_check::run(...)` at the spots near `lib.rs:171-208`)

**Interfaces:**
- Consumes: `CommandResult`, `StepResult` from `crate::result` (same imports `sequence_check.rs` uses).
- Produces: a step named `test-backend-pattern`; a `const SCANNED: &[&str] = &["server/tests/storage/storage.rs"];` path set that #127 later widens.

- [ ] **Step 1: Write the failing fixture unit tests** in `test_pattern_check.rs`. Model on `sequence_check.rs:75-112`. The pure function under test is `fn violations(path: &str, source: &str) -> Vec<usize>` (1-based line numbers of offending `#[tokio::test]`s) and `fn problems(scanned: &[(String, String)]) -> Option<String>`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const ANNOTATED: &str = "\
#[apply(backends)]
#[tokio::test]
async fn good(#[case] backend: Backend) {}
";
    const BARE: &str = "\
#[tokio::test]
async fn bad() {}
";
    const SYNC_UNIT: &str = "\
#[test]
fn pure_logic() {}
";

    #[test]
    fn annotated_tokio_test_is_clean() {
        assert!(violations("f.rs", ANNOTATED).is_empty());
    }

    #[test]
    fn bare_tokio_test_is_flagged_at_its_line() {
        assert_eq!(violations("f.rs", BARE), vec![1]);
    }

    #[test]
    fn sync_unit_test_is_exempt() {
        assert!(violations("f.rs", SYNC_UNIT).is_empty());
    }

    #[test]
    fn problem_detail_names_file_line_and_recovery() {
        let detail = problems(&[("storage.rs".into(), BARE.into())]).expect("a problem");
        assert!(detail.contains("storage.rs:1"));
        assert!(detail.contains("#[apply(backends|sqlite_only|postgres_only)]"));
    }

    #[test]
    fn clean_set_reports_no_problems() {
        assert_eq!(problems(&[("f.rs".into(), ANNOTATED.into())]), None);
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail** — `cargo test -p xtask test_pattern_check`. Expected: FAIL (`violations`/`problems` not defined).

- [ ] **Step 3: Implement the scanner.** Algorithm (line-based, mirroring the coverage classifier's line discipline): walk the source lines; for each line whose trimmed text is `#[tokio::test]`, inspect the contiguous block of attribute lines (trimmed text starting with `#[`) immediately **above and below** it up to the `fn`/`async fn` line; the test is a violation if that attribute block contains no `#[apply(backends)]` / `#[apply(sqlite_only)]` / `#[apply(postgres_only)]`. Record the 1-based line of the `#[tokio::test]`. Pure `#[test]` lines have no `#[tokio::test]` and are never inspected. Then:

```rust
use crate::result::{CommandResult, StepResult};
use std::path::Path;

const SCANNED: &[&str] = &["server/tests/storage/storage.rs"];

fn violations(_path: &str, source: &str) -> Vec<usize> {
    // … line-based scan as described; returns 1-based line numbers …
}

fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        for ln in violations(path, source) {
            lines.push(format!("{path}:{ln}: #[tokio::test] without a backend template"));
        }
    }
    if !lines.is_empty() {
        lines.push(
            "  recovery: annotate the test #[apply(backends|sqlite_only|postgres_only)] \
             (a single-backend test must carry a // reason: comment)"
                .to_string(),
        );
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

pub fn run(result: &mut CommandResult) {
    let scanned: Vec<(String, String)> = SCANNED
        .iter()
        .filter_map(|p| std::fs::read_to_string(Path::new(p)).ok().map(|s| (p.to_string(), s)))
        .collect();
    let step = match problems(&scanned) {
        None => StepResult::ok("test-backend-pattern"),
        Some(detail) => StepResult::fail("test-backend-pattern").detail(detail),
    };
    result.push(step);
}
```

- [ ] **Step 4: Run the unit tests, verify they pass** — `cargo test -p xtask test_pattern_check`. Expected: PASS.

- [ ] **Step 5: Register + wire the module.** Add `pub mod test_pattern_check;` to `xtask/src/steps/mod.rs`; in `xtask/src/lib.rs`, add `steps::test_pattern_check::run(&mut result);` right after each `steps::sequence_check::run(&mut result);` in the `Check` and `Validate` arms.

- [ ] **Step 6: Verify the guard passes against the now-clean storage.rs** — `cargo xtask check --no-test`. Expected: the `test-backend-pattern` step is `ok` (Tasks 1–7 left zero bare `#[tokio::test]`). If it fails, a straggler was missed — fix it before proceeding.

- [ ] **Step 7: Final full local gate** — `cargo xtask validate`. Expected: green, including both backends across the storage suite and the e2e matrix.

- [ ] **Step 8: Commit**

```bash
git add xtask/src/steps/test_pattern_check.rs xtask/src/steps/mod.rs xtask/src/lib.rs
git commit -m "feat(issue-54): xtask guard rejects #[tokio::test] without a backend template"
```

---

## Notes for the implementer

- **Reading before converting:** always read the actual current test body — line numbers in this plan are audit-time approximations and shift as tests are edited; match by test-name, not by line.
- **No separable concerns to file:** the sibling work (other `server/tests` files, storage-crate dialect reconciliation, backup-on-PG) is already tracked as #127 / #135 / #136. This plan files no new issues.
- **If a divergent error path cannot be reproduced portably** (a corruption/forced-failure setup that only works on SQLite), the correct outcome is an annotated `#[apply(sqlite_only)]` test with a stated reason — the acceptance criteria explicitly allow intentional, annotated single-backend tests. Do not fabricate a passing Postgres case.

## Self-review

- **Spec coverage:** Part A (convert ~36 agnostic) → Tasks 1–6; Part B (annotate single-backend via standard mechanism) → Task 7; Part C (xtask guard, path-configurable) → Task 8; orphaned-helper removal → folded into Tasks 2–7 under the `-D dead_code` standing rule; `confirm_password_reset` already dual (audit only) → covered by the Task-7 `rg` sweep confirming no bare straggler remains. No ADR (matches spec). ✓
- **Placeholders:** none — the guard code is complete; conversion tasks name exact tests, handles, and the worked transform. The per-test bodies are read-then-transform (a conversion, not green-field), which is why full per-test code is not pre-pasted. ✓
- **Type consistency:** `violations(path, source) -> Vec<usize>`, `problems(&[(String, String)]) -> Option<String>`, step name `test-backend-pattern`, const `SCANNED` — used consistently across Task 8 steps. Handle names (`state.atomic`/`state.users`/`state.sessions`/`state.invites`/`state.email_verifications`/`state.password_resets`/`state.site_config`) match the Global Constraints list. ✓
