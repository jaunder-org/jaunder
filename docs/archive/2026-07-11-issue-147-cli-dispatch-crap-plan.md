# CLI dispatch CRAP reduction (#147) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating to a subagent via **jaunder-dispatch** when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-11-issue-147-cli-dispatch-crap.md`](../specs/2026-07-11-issue-147-cli-dispatch-crap.md)
— the "what/why." This plan is the "how."

**Goal:** Move `main.rs::run`'s 9-arm subcommand match into a
`Commands::execute(self)` method in `commands.rs`, extract `Username`/`Password`
parsing into helpers, so `run`'s CRAP drops from 27.89 well under the 30.0
threshold and future subcommands no longer inflate it.

**Architecture:** `run` (binary) becomes resolve-command → bind telemetry →
`command.execute().await`. `Commands::execute` (lib, `commands.rs`) is a flat
match-**expression** (no per-arm `?`). `parse_username`/`parse_password` helpers
absorb the parse glue; `cli.rs` stays declarative.

**Tech Stack:** Rust, `server` package (`jaunder` lib + binary), clap, anyhow,
`tokio::test`.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Gate before commit:** `cargo xtask check` (run via
  `devtool run -- cargo xtask check`) must pass clean — see **jaunder-commit**.
- **Behavior-neutral refactor:** no `cmd_*` signature or behavior change; every
  existing `main.rs` test passes **unmodified**.
- `Commands::execute` is `pub` (the binary's `run` calls it across the lib/bin
  boundary); `parse_username`/`parse_password` are private to `commands.rs`.
- Run server tests with `cargo nextest run -p jaunder <filter>`.

## Review header

**Scope (in):** `server/src/main.rs` (rewrite `run`, drop now-unused top
imports), `server/src/commands.rs` (add `Commands` import,
`parse_username`/`parse_password`, `impl Commands { execute }`, helper unit
tests). **Scope (out):** `cli.rs` (unchanged), all `cmd_*` bodies/signatures,
telemetry, the clap arg surface. No separable concerns surfaced — nothing to
file.

**Tasks:**

1. Add the parse helpers + `Commands::execute` (with helper unit tests) in
   `commands.rs`, then rewrite `main.rs::run` to delegate. Single cohesive
   deliverable — `execute` without `run` using it would be a half-change no
   reviewer would accept separately.

**Key risks/decisions:**

- `cmd_backup` returns `Result<PathBuf>` (all other `cmd_*` return
  `Result<()>`), so its arm ends `.await.map(drop)` — the only non-uniform arm.
  Not a decision point (no CRAP cost).
- Dropping the top-level `Commands`/`Username`/`Password` imports from
  `main.rs`: after the rewrite `run` names none of them (the test module keeps
  its own imports), so clippy `unused_imports` requires removing them.
- Coverage: existing `main.rs` tests drive all 9 arms through `run` → `execute`;
  new helper unit tests cover `parse_*` incl. the `None`-password branch. No arm
  or helper branch left uncovered.

---

### Task 1: `Commands::execute` dispatch + parse helpers; rewrite `run`

**Files:**

- Modify: `server/src/commands.rs` (import `Commands`; add helpers +
  `impl Commands { execute }`; add unit tests in the `mod tests` at ~610)
- Modify: `server/src/main.rs` (rewrite `run` ~30-127; drop unused imports ~2-4)

**Interfaces:**

- Consumes: existing `cmd_init`, `cmd_create_pg_db`, `cmd_serve`,
  `cmd_user_create`, `cmd_app_password_create`, `cmd_user_invite`,
  `cmd_smtp_test`, `cmd_backup`, `cmd_restore` (unchanged);
  `jaunder::cli::Commands`.
- Produces:
  - `impl Commands { pub async fn execute(self) -> anyhow::Result<()> }`
  - `fn parse_username(s: &str) -> anyhow::Result<Username>` (private)
  - `fn parse_password(p: Option<String>) -> anyhow::Result<Option<Password>>`
    (private)

- [x] **Step 1: Write the failing helper unit tests** in `commands.rs`'s
      `#[cfg(test)] mod tests` (it already has `use super::*;`, so the private
      helpers and `Username`/`Password` are in scope):

```rust
#[test]
fn parse_username_accepts_valid_and_rejects_invalid() {
    assert!(parse_username("alice").is_ok());
    let err = parse_username("invalid username").unwrap_err().to_string();
    assert!(err.contains("username must be"), "got: {err}");
}

#[test]
fn parse_password_none_is_ok_none() {
    assert!(parse_password(None).unwrap().is_none());
}

#[test]
fn parse_password_validates_some() {
    assert!(parse_password(Some("password123".to_owned())).unwrap().is_some());
    let err = parse_password(Some("short".to_owned())).unwrap_err().to_string();
    assert!(err.contains("at least 8 characters"), "got: {err}");
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p jaunder parse_username parse_password` Expected: FAIL
to compile — `parse_username` / `parse_password` not defined.

- [x] **Step 3: Implement the helpers + `execute` in `commands.rs`.** Change the
      import at line 9 to `use crate::cli::{Commands, StorageArgs};`. Add the
      helpers and the impl (near the top, after the imports/`use` block, or just
      above the first `cmd_*` — keep them together):

```rust
/// Parse a CLI username string into the validated `Username` newtype, surfacing
/// the validation error as an `anyhow` message.
fn parse_username(s: &str) -> anyhow::Result<Username> {
    s.parse().map_err(|e| anyhow::anyhow!("{e}"))
}

/// Parse an optional CLI password string into `Option<Password>` (None stays
/// None), surfacing the validation error as an `anyhow` message.
fn parse_password(p: Option<String>) -> anyhow::Result<Option<Password>> {
    p.map(|p| p.parse::<Password>())
        .transpose()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

impl Commands {
    /// Dispatch this parsed subcommand to its handler. A flat match-expression:
    /// each arm evaluates to the command's `Result<()>`, so there is no per-arm
    /// `?` and no trailing `Ok(())` — keeping any single function's cyclomatic
    /// complexity (and thus CRAP) low as subcommands are added (#147).
    ///
    /// # Errors
    /// Propagates the selected command's failure.
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            Commands::Init {
                storage,
                skip_if_exists,
            } => cmd_init(&storage, skip_if_exists).await,
            Commands::CreatePgDb { pg } => {
                cmd_create_pg_db(&pg.bootstrap_db, &pg.app_db, &pg.app_role_password).await
            }
            Commands::Serve {
                storage,
                bind,
                environment,
                runtime_file,
            } => cmd_serve(&storage, bind, environment.is_prod(), runtime_file).await,
            Commands::UserCreate {
                storage,
                username,
                password,
                display_name,
                operator,
            } => {
                cmd_user_create(
                    &storage,
                    &parse_username(&username)?,
                    parse_password(password)?,
                    display_name.as_deref(),
                    operator,
                )
                .await
            }
            Commands::AppPasswordCreate {
                storage,
                username,
                label,
            } => cmd_app_password_create(&storage, &parse_username(&username)?, &label).await,
            Commands::UserInvite {
                storage,
                expires_in,
            } => cmd_user_invite(&storage, expires_in).await,
            Commands::SmtpTest { storage, to } => cmd_smtp_test(&storage, &to).await,
            Commands::Backup {
                storage,
                mode,
                path,
            } => cmd_backup(&storage, mode.into(), path).await.map(drop),
            Commands::Restore { storage, path } => cmd_restore(&storage, &path).await,
        }
    }
}
```

(The arms are transcribed verbatim from the current `main.rs::run` match, minus
each arm's `?`/`;` and with parsing routed through the helpers. `Backup`'s
`.map(drop)` discards `cmd_backup`'s `PathBuf`.)

- [x] **Step 4: Run the helper tests, verify they pass**

Run: `cargo nextest run -p jaunder parse_username parse_password` Expected: PASS
(3 tests).

- [x] **Step 5: Rewrite `main.rs::run` to delegate** (replace the whole
      `match     command { … }` block and the trailing `Ok(())`; keep the
      no-subcommand `let…else` and the telemetry bind exactly):

```rust
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let Some(command) = cli.command else {
        // `jaunder` with no subcommand is not runnable — re-parse to trigger
        // clap's built-in help/usage, which prints and exits.
        // cov:ignore-start
        Cli::parse_from(["jaunder", "--help"]);
        // cov:ignore-stop
        unreachable!("Cli::parse_from([\"jaunder\", \"--help\"]) prints help and exits the process")
    };
    // `run` owns telemetry for *every* command, `serve` included: one guard,
    // held across the whole dispatch, whose Drop flushes the OTLP exporters
    // before exit. Bound after command resolution so the no-subcommand `--help`
    // path exits via clap without initializing telemetry it would never use.
    let _telemetry = jaunder::observability::init_tracing(cli.verbose);
    command.execute().await
}
```

Then drop the now-unused top-level imports at `main.rs:2-4` —
`use common::password::Password;`, `use common::username::Username;`, and
`Commands` from `use jaunder::cli::{Cli, Commands};` (leaving
`use   jaunder::cli::Cli;`). The `#[cfg(test)] mod tests` keeps its own
`use   jaunder::cli::{...}` untouched.

- [x] **Step 6: Run the full `run` test suite, verify behavior preserved**

Run: `cargo nextest run -p jaunder --lib --bin jaunder` (or
`cargo nextest run -p jaunder`) — covers `main.rs`'s `mod tests` (all 9
variants + the two parse-error tests) and the new helper tests. Expected: PASS,
all unmodified — same success and error-message assertions.

- [x] **Step 7: Confirm CRAP dropped.** Run the gate and inspect the CRAP
      outcome.

Run: `devtool run -- cargo xtask check` Expected: green —
`coverage — clean … 0 CRAP over threshold`. That gate line is the binding proof
(every function ≤ 30). The AC targets (`run` ≤ 5, `execute` ≤ 12) follow
structurally from the flat one-liner arms; if you want the exact per-function
numbers, run `devtool run -- devtool coverage emit` and read the CRAP section of
its report (don't assume a fixed `.xtask` path). If `run` or `execute` reads
high, the arms aren't uniform one-liners — revisit Step 3.

- [x] **Step 8: Commit.**

```bash
git add server/src/commands.rs server/src/main.rs
git commit -m "refactor(server): dispatch subcommands via Commands::execute (#147)"
```

Run `devtool run -- cargo xtask check` first so the pre-commit gate passes clean
(**jaunder-commit**).

---

## Self-review notes

- **Spec coverage:** AC#1 → Step 5 (`run` holds no match); AC#2 → Step 3 (flat
  `execute`, `Backup` `.map(drop)`); AC#3 → Step 3 helpers + Step 5 (no inline
  parse in `main.rs`); AC#4 → `cli.rs` untouched (scope-out); AC#5/#8 → Step 7
  gate + CRAP check; AC#6 → Step 6 (existing tests unmodified); AC#7 → Steps 1-4
  (helper unit tests incl. `parse_password(None)`).
- **No placeholders:** every step has real code + exact commands.
- **Type consistency:** `parse_username(&str)->Result<Username>`,
  `parse_password(Option<String>)->Result<Option<Password>>`,
  `execute(self)-> Result<()>` — matched at every call site; arg temporaries
  (`&parse_username(&u)?`) live to statement end so the `&Username` borrow
  survives the `.await`.
