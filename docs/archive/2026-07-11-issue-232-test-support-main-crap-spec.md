# Spec — #232: cover & refactor `test-support::main` to retire its `crap:allow`

**Issue:** [#232](https://github.com/jaunder-org/jaunder/issues/232) **Status:**
proposed **Depends on:** nothing (#231's T=30 gate is already live).

## Problem — and the stale framing

#232 was filed when #231 _planned_ the T=30 CRAP gate and
`test-support/src/main.rs::main` (CRAP 156, cyclomatic 12, 0% cov) was the sole
blocker. The gate is now **live and green** — but only because `main` carries a
stopgap **`// crap:allow: … real fix tracked in #232`** (main.rs:83) plus a
**`cov:ignore`** block over its three DB-opening arms (main.rs:86-125). So the
gate passes on an override, and #232's real task is to **earn its removal**.

Verified facts:

- **`cargo crap` scores from the raw profile**
  (`tools/devtool/src/coverage/emit.rs` runs `cargo crap` on the lcov; it knows
  nothing of `cov:ignore`). So `cov:ignore` does **not** lower `main`'s CRAP —
  only `crap:allow` exempts it. Removing `crap:allow` alone would fail (main is
  still well over 30, its DB arms uncovered in the profile).
- **All real logic is already tested.** `lib.rs`'s `seed_posts_for_user`,
  `create_user`, `set_site_config`, `reset_mail` have in-process SQLite tests
  via `storage::test_support::Backend::Sqlite.setup()`. The **only** uncovered
  code is `main.rs`'s CLI glue — the 3 DB arms (`open_existing_database(&db)` →
  tested lib fn → `eprintln`), which are multi-line so they leave
  genuinely-uncovered lines.
- The 2 non-DB arms (`ResetMail`, `CapturePath`) are already covered by the
  subprocess tests in `tests/cli.rs`; their `.ok_or_else(|| …)` error arms are
  single-line, so they leave no uncovered lines (per the single-line-coverage
  idiom).

## Approach (issue options 1 **and** 2)

Both refactor `main` into a thin shell **and** cover the arms, so **both** the
`cov:ignore` and the `crap:allow` come out:

- **Extract `async fn run(cli: Cli) -> anyhow::Result<()>`** from `main`; `main`
  becomes `run(Cli::parse()).await` (mirrors `server::run`). `main` is then a
  thin, low-complexity shell.
- **Extract one small async handler per subcommand** — `cmd_seed_posts`,
  `cmd_create_user`, `cmd_set_site_config`, `cmd_reset_mail`, `cmd_capture_path`
  — so `run`'s `match` is uniform one-line arms
  (`Commands::X { … } => cmd_x(…).await`). Each handler owns its
  `open_existing_database` + tested-lib call + `eprintln`/`println` (verbatim
  from today's arm bodies).
- **Cover the 3 DB handlers in-process.** Add a `#[cfg(test)] mod tests` to
  `main.rs` that inits a temp SQLite DB and drives each DB command through
  `run`:
  ```rust
  let dir = tempfile::TempDir::new().unwrap();
  let db = storage::test_support::sqlite_url(&dir); // DbConnectOptions
  storage::open_database(&db).await.unwrap();        // create + migrate; state dropped → pool closed
  run(cli(Commands::CreateUser { db: db.clone(), username: "alice".into(),
      password: "password123".into(), display_name: None, operator: false })).await.unwrap();
  run(cli(Commands::SeedPosts { db: db.clone(), username: "alice".into(),
      count: 1, body_prefix: "P".into(), published: true })).await.unwrap();
  run(cli(Commands::SetSiteConfig { db, key: "site.x".into(), value: "y".into() })).await.unwrap();
  ```
  Driving `run(Cli{…})` (not the handlers directly) covers **both** `run`'s
  dispatch arms and the handler bodies. `open_database` migrates; each `run`
  call then opens its own short-lived `open_existing_database` connection, so
  the pools never overlap (no SQLite lock contention). `create-user` runs first
  because `seed-posts` needs an existing user.
- **Remove** the `// crap:allow` line and the `// cov:ignore-start/stop` block.
  The existing `tests/cli.rs` continues to cover `main` → `run` → the non-DB
  arms (subprocess) and `Cli::parse`.

Once every arm is covered, CRAP = each function's cyclomatic complexity: `run`
(~6, dispatch) and each handler (~2-3), all far under 30 — no override needed.

## Acceptance criteria

1. **No `crap:allow` in `test-support`.** `rg 'crap:allow' test-support/` → no
   match. `main` (and every function in `main.rs`) passes the T=30 CRAP gate on
   its own.
2. **No `cov:ignore` in `test-support/src/main.rs`.**
   `rg 'cov:ignore' test-support/src/main.rs` → no match; the DB-arm ignore
   block is gone.
3. **`main` is a thin shell.** `main` resolves args and calls `run` only
   (`run(Cli::parse()).await` plus the fail-closed `#[tokio::main]`); it holds
   no `match` over `Commands`.
4. **`run` + per-command handlers exist.** `main.rs` defines
   `async fn run(cli: Cli) -> anyhow::Result<()>` whose body is a flat `match`
   with one uniform arm per variant delegating to a handler fn (`cmd_seed_posts`
   / `cmd_create_user` / `cmd_set_site_config` / `cmd_reset_mail` /
   `cmd_capture_path`).
5. **DB arms are genuinely covered.** New in-process tests drive `create-user` →
   `seed-posts` → `set-site-config` through `run` against a temp SQLite DB; the
   coverage gate reports **0 uncovered lines** in `main.rs` with no ignore
   markers.
6. **Behavior preserved.** The subcommand surface (clap args, subcommand names,
   `stdout`/`stderr` messages, exit codes) is unchanged; `tests/cli.rs` passes
   **unmodified**, and the e2e suite (which drives `test-support` over a process
   boundary) is unaffected — no lib (`lib.rs`) signature change.
7. **Gate green.** `cargo xtask check` passes:
   `0 failures, 0 guard violations, 0 CRAP over threshold`.

## Out of scope

- Any change to `lib.rs` (`seed_posts_for_user`/`create_user`/`set_site_config`/
  `reset_mail`) — signatures and behavior stay; they are already tested.
- Changing the clap `Cli`/`Commands` arg definitions or subcommand names/flags
  (the e2e harness invokes them by name).
- Postgres coverage of the DB arms — SQLite in-process coverage suffices for the
  gate; the dual-backend path is proven by the e2e matrix (per `lib.rs`'s
  SQLite-only test rationale).
