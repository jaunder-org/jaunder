# `SessionLabel` through the app-password CLI path — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`../specs/2026-08-05-issue-690-session-label-cli.md`](../specs/2026-08-05-issue-690-session-label-cli.md)
**Issue:** [#690](https://github.com/jaunder-org/jaunder/issues/690) ·
**Milestone** #13

**Goal:** Type the app-password CLI's `label` as `SessionLabel` end to end, so
the `#325` validation chokepoint can be **deleted** rather than moved.

**Architecture:** Clap's derived `FromStr` value parser does the work — the same
mechanism already parsing `username: Username` two lines above in the same
struct. Typing the clap arg and both function hops makes the chokepoint's parse
dead code, and rejection moves ahead of the database open.

**Tech Stack:** Rust; `clap` derive; `rstest`/`rstest_reuse` + `nextest`;
`cargo xtask`.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Backend parity (ADR-0019/0053):** `server/tests/**` tests touching a live
  pool are `#[apply(backends)]`.
- **Delete, don't relocate.** If the implementation ends with a `.parse()` on a
  label anywhere below the clap boundary, the task has failed its own point.
- The pre-commit hook runs the full `cargo xtask check`; run it first
  (**`jaunder-commit`**).
- **Commit via `devtool run -- git commit …`** so the hook's output parks
  instead of flooding the transcript.

## Review header

**Scope in:** `server/src/cli.rs` (the arg + a parse-rejection test + a default
test), `server/src/commands.rs` (both fn signatures, chokepoint deleted),
`server/src/main.rs` (one test literal), `server/tests/misc/commands.rs` (two
call sites + one new test). **Scope out:** `web/src/auth/api.rs` (already typed
by #685); `SubscriberSummary.label` (moved to #750); the elisp integration test
(passes a valid label, needs no edit); making `--label` mandatory.

**Tasks:**

1. Type the seam and delete the chokepoint — compiler-forced, with the
   clap-level tests.
2. The default-path readback test (dual-backend).
3. Full `validate` with e2e.

**Key risks / decisions:**

- **The red test is genuinely red.**
  `app_password_create_malformed_label_is_clap_error` fails today because
  `label: String` accepts anything — clap parses `--label ""` successfully and
  the rejection only happens later, after the database is opened. That is the
  defect, expressed as a test.
- **Spec correction on A6's home.** The spec put the default-path test in
  `server/src/main.rs`, reasoning that `run()` is the only way to see a clap
  default. `server/tests/misc/commands.rs` already runs **dual-backend** CLI
  tests via `storage_args(backend, &base)` (`:222`), and the default can be
  observed by parsing argv and passing the parsed label to
  `cmd_app_password_create` — no `run()` needed. It goes there instead, and
  gains real backend parity that `main.rs`'s SQLite-temp-dir tests do not have.
- **`Cli` reachability: resolved, no decision needed.** `server/src/lib.rs:4` is
  `pub mod cli;` with `pub struct Cli` (`:19`) and `pub enum Commands` (`:232`),
  and `server/tests/misc/commands.rs:12` already imports
  `jaunder::cli::StorageArgs`. The test file only needs to add
  `use clap::Parser;` for `try_parse_from`.
- **Known coverage gap, accepted.** Task 2 calls `cmd_app_password_create`
  directly rather than going through the `commands.rs:80` dispatch, so the
  _default_ label is not exercised through dispatch. That arm is already covered
  with an **explicit** label by `main.rs:270`'s
  `run_app_password_create_mints_for_existing_user`, and the default is a
  clap-level concern pinned by A5 — so the untested combination is "dispatch ×
  default", which no realistic edit breaks in isolation. Do not add machinery
  for it.

---

### Task 1: Type the seam, delete the chokepoint

**Files:**

- Modify: `server/src/cli.rs:317-318` (the arg), plus its
  `#[cfg(test)] mod tests`
- Modify: `server/src/commands.rs:238-253` (`app_password_create`), `:267-278`
  (`cmd_app_password_create`)
- Modify: `server/src/main.rs:299` (`label: "ert".to_string()`)
- Modify: `server/tests/misc/commands.rs:230,243` (two call sites passing
  `"ert"`)

**Interfaces:**

- Produces `AppPasswordCreate { …, label: SessionLabel }`;
  `app_password_create(&AppState, &Username, &SessionLabel) -> anyhow::Result<RawToken>`;
  `cmd_app_password_create(&StorageArgs, &Username, &SessionLabel) -> anyhow::Result<()>`.
- Consumes `common::session_label::{SessionLabel, MAX_SESSION_LABEL_CHARS}` and,
  in tests, `common::test_support::parse_session_label`.

- [ ] **Step 1: Write the failing tests**

In `server/src/cli.rs`'s test module, beside
`user_create_malformed_username_is_clap_error` (`:801-808`), which is the direct
analogue. **Add `MAX_SESSION_LABEL_CHARS` to that module's imports** — it
currently has `use super::*` plus
`common::test_support::{parse_display_name, parse_invite_ttl_hours}`, and Step 3
only brings `SessionLabel` into the non-test scope.

```rust
#[test]
fn app_password_create_malformed_label_is_clap_error() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    // The `SessionLabel` value parser rejects at parse time, before any handler
    // opens the database. Until #690 this was a `String` and the rejection lived in
    // `app_password_create`, one call *after* `open_existing_database`.
    let over = "a".repeat(MAX_SESSION_LABEL_CHARS + 1);
    for bad in ["", over.as_str()] {
        let result = Cli::try_parse_from([
            "jaunder",
            "app-password-create",
            "--username",
            "alice",
            "--label",
            bad,
        ]);
        let err = result.expect_err("label {bad:?} must be rejected at parse");
        // A4 wants the error to *name the constraint*, not merely be an error — otherwise
        // an operator sees "invalid value" with no idea what the rule is.
        assert!(
            err.to_string().contains("non-empty and at most"),
            "the clap error must name the constraint; got: {err}"
        );
    }
}

/// A5: the `default_value` literal is itself parsed by `SessionLabel`, so a typo in it
/// would otherwise fail only at runtime, on every invocation that omits `--label`.
#[test]
fn app_password_create_label_defaults_to_app_password() {
    let _guard = ENV_LOCK.lock().expect("env lock");
    let cli = parse(&["app-password-create", "--username", "alice"]);
    let Some(Commands::AppPasswordCreate { label, .. }) = cli.command else {
        panic!("expected app-password-create");
    };
    assert_eq!(label, "app-password");
}
```

`assert_eq!(label, "app-password")` compiles because `StrNewtype` emits
`PartialEq<str>` — the same form `common/src/session_label.rs:130` already uses.

- [ ] **Step 2: Run them, verify they fail**

`devtool run --cwd <worktree> -- cargo nextest run -p jaunder app_password_create_malformed_label`
Expected: **FAIL** — `label: String` accepts `""` and the over-long string, so
`try_parse_from` returns `Ok`. The default test fails to compile (`label` is a
`String`, and destructuring then comparing is fine, so it may _pass_ — that one
only becomes meaningful after Step 3; the malformed-label test is the real red).

- [ ] **Step 3: Type the seam**

`server/src/cli.rs:317-318`:

```rust
/// Label recorded with the session (shown in the sessions UI).
#[arg(long, default_value = "app-password")]
label: SessionLabel,
```

`default_value` stays a `&str` literal — clap pushes it through the same value
parser, so no `default_value_t` is needed and the default is validated by the
same rule as user input (spec D3).

`server/src/commands.rs` — both signatures take `&SessionLabel`, and **delete**
the parse:

```rust
pub async fn app_password_create(
    state: &storage::AppState,
    username: &Username,
    label: &SessionLabel,
) -> anyhow::Result<RawToken> {
    let user = state.users.get_user_by_username(username).await
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .ok_or_else(|| anyhow::anyhow!("no such user '{username}'"))?;
    // No validation here: `label` arrives validated by the clap boundary (#690). The
    // `#325` chokepoint that used to re-parse it is gone, not relocated.
    let token = state.sessions.create_session(user.user_id, label).await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(token)
}
```

…and the same `&SessionLabel` change on `cmd_app_password_create` (`:267-278`),
whose body is otherwise unchanged. The dispatch at `:80` already passes `&label`
and should compile untouched — verify rather than edit.

Then follow the compiler: `main.rs:299` becomes
`label: parse_session_label("ert")`, and `server/tests/misc/commands.rs:230,243`
pass `&parse_session_label("ert")`. Use
`common::test_support::parse_session_label` (the door
`storage/src/sessions.rs:261` already uses), not an inline `.parse().unwrap()`.

- [ ] **Step 4: Run, verify PASS**

`devtool run --cwd <worktree> -- cargo nextest run -p jaunder app_password_create`
`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p jaunder`
Expected: **PASS** on both, including the two pre-existing dual-backend
`cmd_app_password_create_*` tests.

- [ ] **Step 5: Prove the chokepoint is gone**

`devtool run --cwd <worktree> -- rg -n 'label.*\.parse\(\)|invalid label' server/src`
Expected: **no matches** (spec A3 — the parse is deleted, not moved).

`devtool run --cwd <worktree> -- rg -n 'label: &str|label: String' server/src`
Expected: **no matches** (spec A2).

- [ ] **Step 6: Commit**

`devtool run --cwd <worktree> -- cargo xtask check`, then:

```bash
devtool run --cwd <worktree> -- git commit -a -m "refactor(cli): type the app-password label as SessionLabel (#690)"
```

---

### Task 2: The default-path readback test

**Files:** Modify `server/tests/misc/commands.rs` (new test beside
`cmd_app_password_create_succeeds_for_existing_user` at `:218`).

**Interfaces:** consumes Task 1's typed `cmd_app_password_create`; the
per-backend `storage_args(backend, &base)` helper (`:222`);
`SessionRecord.label: SessionLabel` (`storage/src/sessions.rs:23`) via
`list_sessions` (`:87`).

- [ ] **Step 1: Write the failing test**

```rust
/// A6: the `--label` default is applied by **clap**, so nothing below
/// `cmd_app_password_create` can observe it. Drive argv through the parser, then hand the
/// parsed label to the command, and read the session back — this is what pins the default
/// end to end rather than merely asserting the literal parses (that is the cli.rs test).
#[apply(backends)]
#[tokio::test]
async fn app_password_create_records_the_default_label(#[case] backend: Backend) {
    let base = TempDir::new().unwrap();
    let (args, _pg) = storage_args(backend, &base).await;
    cmd_init(&args, false).await.unwrap();
    let username: Username = "alice".parse().unwrap();
    let password: Password = "password123".parse().unwrap();
    cmd_user_create(&args, &username, Some(password), None, false).await.unwrap();

    // No `--label`: clap supplies the default.
    let cli = Cli::try_parse_from([
        "jaunder", "app-password-create", "--username", "alice",
    ])
    .expect("app-password-create must parse without --label");
    let Some(Commands::AppPasswordCreate { label, .. }) = cli.command else {
        panic!("expected app-password-create");
    };

    cmd_app_password_create(&args, &username, &label).await.unwrap();

    let state = open_existing_database(&args.db).await.expect("reopen");
    let user = state.users.get_user_by_username(&username).await.unwrap().expect("alice");
    let sessions = state.sessions.list_sessions(user.user_id).await.unwrap();
    assert_eq!(sessions.len(), 1, "one app password was minted");
    assert_eq!(sessions[0].label, "app-password");
}
```

`Cli` and `Commands` are reachable — `server/src/lib.rs:4` is `pub mod cli;`
with `pub struct Cli` (`:19`) and `pub enum Commands` (`:232`) — and this file
already imports `jaunder::cli::StorageArgs` (`:12`) and `open_existing_database`
(`:17`). Add `use clap::Parser;` for `try_parse_from`, plus
`jaunder::cli::{Cli, Commands}`.

- [ ] **Step 2: Run it, verify PASS on both backends**

**This task has no TDD red step, deliberately.** It is a characterization test
written after Task 1, not a driver for it: what it locks is that the clap
default actually reaches the stored session, which no other test observes.

`devtool run --cwd <worktree> -- devtool pg run -- cargo nextest run -p jaunder app_password_create_records_the_default_label`
Expected: **PASS**, `case_1_sqlite` and `case_2_postgres`. If it **fails**, the
default is not reaching the session — a real defect in Task 1, not a test bug.
Fix the code, not the assertion.

- [ ] **Step 4: Commit**

```bash
devtool run --cwd <worktree> -- git commit -a -m "test(cli): pin the app-password default label end to end (#690)"
```

---

### Task 3: Full-gate verification

**Files:** none — verification only, no commit.

- [ ] **Step 1: Verify-only gate (spec A8)**

`devtool run --cwd <worktree> -- cargo xtask validate --no-e2e` Expected:
**PASS**. Not redundant with Task 1's `check`: `check` auto-fixes formatting, so
a green `check` can leave the tree mutated after the commit; `validate` is
verify-only.

- [ ] **Step 2: Full gate with e2e (spec A9)**

`devtool run --cwd <worktree> -- cargo xtask validate` (Bash background mode;
~25 min) Expected: **PASS**.

Required rather than optional: `elisp/test/jaunder-integration-helper.el:129`
drives the real binary with `--label "ert"` through this exact argument, and it
is the only coverage of the CLI's parse path end to end. A valid label must
still work — if this newly fails, the value parser is rejecting something it
should accept.

- [ ] **Step 3: On failure, read the sidecar**

`devtool run --cwd <worktree> -- jq '.steps[] | select(.ok == false)' .xtask/last-result.json`
