# Spec — #147: reduce `main.rs::run` CRAP via a `Commands::execute` dispatch

**Issue:** [#147](https://github.com/jaunder-org/jaunder/issues/147) **Status:**
proposed **Depends on:** nothing.

## Problem

`server/src/main.rs::run` is a 9-arm `match` over `jaunder::cli::Commands`. Its
CRAP score is **27.89** and rose 25.48 → 27.89 when `app-password-create` landed
— approaching the **CRAP_THRESHOLD of 30.0** (exclusive;
`xtask/src/coverage/crap.rs`). Two structural traits inflate it beyond the 9-arm
count:

1. Every arm ends `cmd_*(…).await?;` with a trailing `Ok(())`, so each arm
   carries its own `?` error-branch (~9 extra decision points).
2. The `UserCreate` and `AppPasswordCreate` arms inline `Username`/`Password`
   parsing with verbose `.map_err(|e| anyhow::anyhow!("{e}"))` (~3 more
   branches).

This is a **proactive ratchet** (milestone "Code quality ratchet"), not a gate
failure: `run` currently passes, but one or two more subcommands would
breach 30. The fix must lower `run`'s complexity _structurally_ so future
subcommands don't re-inflate one giant function.

## Approach

Distribute the complexity (CRAP penalises concentration quadratically) rather
than relocate it wholesale:

- **`run` (main.rs) becomes trivial:** resolve the command (the no-subcommand
  `--help` `let…else` stays), bind the telemetry guard, then delegate:
  `command.execute().await`. No `match`, no trailing `Ok(())`.
- **`Commands::execute(self) -> anyhow::Result<()>`** — a new method holding a
  flat **match-expression** (each arm evaluates to the `Result`, so **no per-arm
  `?` and no trailing `Ok(())`**). One wrinkle: `cmd_backup` returns
  `Result<PathBuf>` (every other `cmd_*` returns `Result<()>`), so the `Backup`
  arm ends `.await.map(drop)` to discard the path — still a single expression,
  still no `?`, and `.map(drop)` is not a decision point (no CRAP cost). Placed
  in **`server/src/commands.rs`** (an `impl Commands` block — legal in any
  module of the defining crate), so `cli.rs` stays purely declarative and
  dispatch lives beside the `cmd_*` functions it calls.
- **Extract the CLI-string parsing** into two private helpers in `commands.rs`:
  - `fn parse_username(s: &str) -> anyhow::Result<Username>`
  - `fn parse_password(p: Option<String>) -> anyhow::Result<Option<Password>>`

  so the `UserCreate` / `AppPasswordCreate` arms stay single expressions and the
  `.map_err` noise has one home. Error text is unchanged (both still surface the
  parse error's `Display` via `anyhow!("{e}")`).

- **Direct unit tests for the two helpers** (sync, storage-free) pin their
  contracts — notably `parse_password(None) → Ok(None)`, which no existing test
  reaches — instead of relying only on the heavier indirect coverage through
  `run`. See AC#7.

The telemetry contract is preserved: `run` binds `_telemetry` **after** command
resolution (so `--help` still skips it) and holds it across `execute().await`;
`execute` does not touch telemetry.

Illustrative shape:

```rust
// main.rs
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let Some(command) = cli.command else {
        // cov:ignore-start
        Cli::parse_from(["jaunder", "--help"]);
        // cov:ignore-stop
        unreachable!("Cli::parse_from([\"jaunder\", \"--help\"]) prints help and exits the process")
    };
    let _telemetry = jaunder::observability::init_tracing(cli.verbose);
    command.execute().await
}

// commands.rs
impl Commands {
    pub async fn execute(self) -> anyhow::Result<()> {
        match self {
            Commands::Init { storage, skip_if_exists } => cmd_init(&storage, skip_if_exists).await,
            Commands::UserCreate { storage, username, password, display_name, operator } =>
                cmd_user_create(
                    &storage, &parse_username(&username)?, parse_password(password)?,
                    display_name.as_deref(), operator,
                ).await,
            // …7 more one-line arms…
        }
    }
}

fn parse_username(s: &str) -> anyhow::Result<Username> {
    s.parse().map_err(|e| anyhow::anyhow!("{e}"))
}
fn parse_password(p: Option<String>) -> anyhow::Result<Option<Password>> {
    p.map(|p| p.parse::<Password>()).transpose().map_err(|e| anyhow::anyhow!("{e}"))
}
```

## Acceptance criteria

1. **`run` no longer holds the subcommand match.** `main.rs::run` contains no
   `match … { Commands::… }`; it resolves the command (keeping the no-subcommand
   `--help` `let…else`), binds `_telemetry`, and its final expression is
   `command.execute().await`.
2. **`Commands::execute` exists and is flat.** `commands.rs` defines
   `impl Commands { pub async fn execute(self) -> anyhow::Result<()> }` whose
   body is a single match-expression with exactly one arm per `Commands`
   variant, each arm a single call expression that evaluates to `Result<()>` (no
   per-arm `?`, no trailing `Ok(())`). The `Backup` arm appends `.map(drop)`
   because `cmd_backup` returns `Result<PathBuf>` — the only arm needing an
   adaptor.
3. **Parsing is extracted.** `parse_username` and `parse_password` exist in
   `commands.rs`; `UserCreate` uses both, `AppPasswordCreate` uses
   `parse_username`; no `Username`/`Password` `.parse()`/`.map_err` remains
   inline in `main.rs` or in `execute`'s arms.
4. **`cli.rs` stays declarative** — no execution/dispatch logic added to it
   (`rg 'cmd_|execute' server/src/cli.rs` finds no dispatch); the `impl` lives
   in `commands.rs`.
5. **CRAP is cut and the gate is clean.** After the change, `run`'s CRAP is well
   below its old 27.89 (target ≤ 5) and `execute`'s CRAP is materially lower too
   (target ≤ 12); the coverage gate reports **0 CRAP over threshold** and no new
   `crap:allow` override is added.
6. **Behavior preserved.** Every existing `main.rs` `#[cfg(test)] mod tests`
   test passes **unmodified** — same success paths and same error-message
   assertions (`"username must be non-empty"`,
   `"password must be at least 8 characters"`, `"PostgreSQL URL"`,
   `"SMTP is not configured"`, `"run \`jaunder init\`
   first"`). The wildcard-free `match`in`execute`statically guarantees no variant is dropped, and these tests drive all 9 variants through`run`→`execute`,
   so they remain the dispatch regression guard.
7. **Parse helpers have direct unit tests.** `commands.rs`'s
   `#[cfg(test)] mod tests` gains sync, storage-free tests pinning the extracted
   helpers' contracts:
   - `parse_password(None)` → `Ok(None)` (the one branch **no** existing test
     exercises), `parse_password(Some(valid))` → `Ok(Some(_))`, and
     `parse_password(Some("short"))` → `Err` whose message contains
     `"at least 8 characters"`.
   - `parse_username("alice")` → `Ok(_)` and
     `parse_username("invalid username")` → `Err` whose message contains
     `"username must be"`.
8. **Gate green.** `cargo xtask check` passes (static + clippy + coverage).

## Out of scope

- Changing any `cmd_*` function signature or behavior (they keep taking already-
  parsed `&Username` / `Option<Password>` — parse-at-the-boundary is preserved).
- Changing the `Commands` variant set, clap arg definitions, or telemetry.
- A runtime/data-driven dispatch table (clap's per-variant field shapes make an
  enum `match` the idiomatic dispatch; a new subcommand adds one uniform arm).
