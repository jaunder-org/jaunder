# test-support::main CRAP retirement (#232) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating to a subagent via **jaunder-dispatch** when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-11-issue-232-test-support-main-crap.md`](../specs/2026-07-11-issue-232-test-support-main-crap.md)
— the "what/why." This plan is the "how."

**Goal:** Extract a thin `main` → `run(cli)` + per-command handlers in
`test-support/src/main.rs`, cover the 3 DB arms with in-process temp-SQLite
tests, and remove **both** the `crap:allow` and the `cov:ignore` block — so the
harness passes the T=30 CRAP gate on its own merits.

**Architecture:** `main` becomes `run(Cli::parse()).await`. `run` is a flat
`match` delegating each variant to a small handler (`cmd_seed_posts` /
`cmd_create_user` / `cmd_set_site_config` / `cmd_reset_mail` /
`cmd_capture_path`). New in-process tests init a temp SQLite DB
(`open_database(&sqlite_url(&dir))`) and drive the 3 DB commands through `run`;
the existing `tests/cli.rs` subprocess tests cover the rest. Fully covered →
each function's CRAP = its small complexity.

**Tech Stack:** Rust, `test-support` bin crate, clap, anyhow, tokio, `storage`
(+ `storage::test_support::sqlite_url`), `tempfile`.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Gate before commit:** `cargo xtask check` (via
  `devtool run -- cargo xtask check`) must pass clean — see **jaunder-commit**.
- **No behavior change:** clap arg/subcommand surface, stdout/stderr messages,
  and exit codes stay identical; `tests/cli.rs` passes **unmodified**; no
  `lib.rs` change.
- Run test-support tests with `cargo nextest run -p test-support <filter>`.

## Review header

**Scope (in):** `test-support/src/main.rs` only — rewrite `main`, add `run` + 5
handlers, add `#[cfg(test)] mod tests`, delete the `crap:allow` line and the
`cov:ignore` block. **Scope (out):** `test-support/src/lib.rs`, `tests/cli.rs`,
the `Cli`/`Commands` arg definitions, Postgres coverage. No separable concerns —
nothing to file.

**Tasks:**

1. Refactor `main` → `run` + per-command handlers, add in-process temp-DB tests,
   remove both markers. Single cohesive deliverable (the test drives `run`,
   which only exists after the refactor; a reviewer wouldn't accept one half
   alone).

**Key risks/decisions:**

- **CRAP reads the raw profile** — only _covering_ the DB arms lowers it;
  `cov:ignore` never did (spec §Problem). Hence the in-process tests are the
  crux.
- **SQLite pools:** `open_database(&db).await.unwrap();` is an unbound temporary
  → its migrating pool drops at the `;` before any `run` call; each `run` opens
  its own short-lived `open_existing_database` pool (WAL + 5s busy_timeout). No
  overlap.
- **Order:** `create-user` before `seed-posts` (the latter looks the user up,
  `lib.rs:69-73`). Hold the `TempDir` for the whole test (dropping it unlinks
  the SQLite file).

---

### Task 1: extract `run` + handlers, cover the DB arms, drop the markers

**Files:**

- Modify: `test-support/src/main.rs` (rewrite `main` 81-141; add `run` +
  handlers; add `#[cfg(test)] mod tests`; delete `crap:allow` 83 + `cov:ignore`
  86-125)

**Interfaces:**

- Consumes (unchanged): `storage::open_existing_database`,
  `storage::open_database`, `storage::test_support::sqlite_url`, and the
  `lib.rs` fns `seed_posts_for_user`, `create_user`, `set_site_config`,
  `reset_mail`; `host::capture`.
- Produces (private to `main.rs`):
  - `async fn run(cli: Cli) -> anyhow::Result<()>`
  - `async fn cmd_seed_posts(db: &DbConnectOptions, username: &str, count: usize, body_prefix: &str, published: bool) -> anyhow::Result<()>`
  - `async fn cmd_create_user(db: &DbConnectOptions, username: &str, password: &str, display_name: Option<&str>, operator: bool) -> anyhow::Result<()>`
  - `async fn cmd_set_site_config(db: &DbConnectOptions, key: &str, value: &str) -> anyhow::Result<()>`
  - `fn cmd_reset_mail() -> anyhow::Result<()>`
  - `fn cmd_capture_path(stream: &str) -> anyhow::Result<()>`

- [x] **Step 1: Write the failing in-process test** — add to the bottom of
      `main.rs`. It references `run`/`Cli`/`Commands`, which the current file
      exposes only inside `main`, so it fails to compile until Step 3:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use storage::test_support::sqlite_url;
    use tempfile::TempDir;

    fn cli(command: Commands) -> Cli {
        Cli { command }
    }

    /// A temp SQLite DB, created + migrated. The migrating pool is dropped before
    /// return (unbound temporary), so each `run` below opens its own connection.
    /// The returned `TempDir` must outlive the test — dropping it unlinks the file.
    async fn temp_db() -> (TempDir, DbConnectOptions) {
        let dir = TempDir::new().unwrap();
        let db = sqlite_url(&dir);
        storage::open_database(&db).await.unwrap();
        (dir, db)
    }

    #[tokio::test]
    async fn run_dispatches_db_commands_against_a_temp_db() {
        let (_dir, db) = temp_db().await;

        run(cli(Commands::CreateUser {
            db: db.clone(),
            username: "alice".to_owned(),
            password: "password123".to_owned(),
            display_name: None,
            operator: false,
        }))
        .await
        .expect("create-user should dispatch and succeed");

        run(cli(Commands::SeedPosts {
            db: db.clone(),
            username: "alice".to_owned(),
            count: 1,
            body_prefix: "Post".to_owned(),
            published: true,
        }))
        .await
        .expect("seed-posts should dispatch and succeed");

        run(cli(Commands::SetSiteConfig {
            db,
            key: "site.registration_policy".to_owned(),
            value: "open".to_owned(),
        }))
        .await
        .expect("set-site-config should dispatch and succeed");
    }
}
```

- [x] **Step 2: Run the test, verify it fails**

Run: `cargo nextest run -p test-support run_dispatches_db_commands` Expected:
FAIL to compile — `run` / `cli` helper reference items not yet defined at file
scope.

- [x] **Step 3: Refactor `main.rs`.** Replace the whole `main` fn (81-141,
      including the `crap:allow` comment and the `cov:ignore` block) with a thin
      `main` + `run` + the five handlers. The handler bodies are the current arm
      bodies verbatim, each ending `Ok(())`:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run(Cli::parse()).await
}

/// Dispatch the parsed subcommand to its handler. A flat match: each arm
/// evaluates to the handler's `Result<()>`, so `main` stays a thin shell and each
/// command is a small, individually-covered unit (#232).
async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::SeedPosts {
            db,
            username,
            count,
            body_prefix,
            published,
        } => cmd_seed_posts(&db, &username, count, &body_prefix, published).await,
        Commands::CreateUser {
            db,
            username,
            password,
            display_name,
            operator,
        } => cmd_create_user(&db, &username, &password, display_name.as_deref(), operator).await,
        Commands::SetSiteConfig { db, key, value } => cmd_set_site_config(&db, &key, &value).await,
        Commands::ResetMail => cmd_reset_mail(),
        Commands::CapturePath { stream } => cmd_capture_path(&stream),
    }
}

/// Seed `count` posts for `username` through the real storage path.
async fn cmd_seed_posts(
    db: &DbConnectOptions,
    username: &str,
    count: usize,
    body_prefix: &str,
    published: bool,
) -> anyhow::Result<()> {
    let state = storage::open_existing_database(db).await?;
    let ids = seed_posts_for_user(&state, username, count, published, body_prefix).await?;
    eprintln!("seeded {} posts for {username}", ids.len());
    Ok(())
}

/// Create a fixture user through the real storage path.
async fn cmd_create_user(
    db: &DbConnectOptions,
    username: &str,
    password: &str,
    display_name: Option<&str>,
    operator: bool,
) -> anyhow::Result<()> {
    let state = storage::open_existing_database(db).await?;
    let id = create_user(&state, username, password, display_name, operator).await?;
    eprintln!("created user {username} with id {id}");
    Ok(())
}

/// Set a `site_config` key/value (an upsert) through the real storage path.
async fn cmd_set_site_config(db: &DbConnectOptions, key: &str, value: &str) -> anyhow::Result<()> {
    let state = storage::open_existing_database(db).await?;
    set_site_config(&state, key, value).await?;
    eprintln!("set site_config {key} = {value}");
    Ok(())
}

/// Reset the mail-capture file (delete it; missing is fine).
fn cmd_reset_mail() -> anyhow::Result<()> {
    let path = capture::file(capture::Stream::Mail)
        .ok_or_else(|| anyhow::anyhow!("JAUNDER_CAPTURE_DIR is not set"))?;
    reset_mail(&path)?;
    eprintln!("reset mail-capture file {}", path.display());
    Ok(())
}

/// Print the resolved capture-file path for a stream (`mail`/`websub`/`diag`).
fn cmd_capture_path(stream: &str) -> anyhow::Result<()> {
    let stream = capture::Stream::parse(stream)
        .ok_or_else(|| anyhow::anyhow!("unknown capture stream {stream:?}"))?;
    let path = capture::file(stream)
        .ok_or_else(|| anyhow::anyhow!("JAUNDER_CAPTURE_DIR is not set"))?;
    println!("{}", path.display());
    Ok(())
}
```

The `use` block (1-8), `Cli` (10-18), and `Commands` (20-79) are unchanged.
Confirm no `crap:allow` / `cov:ignore` string survives in the file.

- [x] **Step 4: Run the test + the subprocess suite, verify green**

Run: `cargo nextest run -p test-support` Expected: PASS — the new
`run_dispatches_db_commands_against_a_temp_db`, the existing `tests/cli.rs`
(`reset_mail_*`, `capture_path_*`, **unmodified**), and the `lib.rs` unit tests
all pass.

- [x] **Step 5: Gate + confirm the markers are gone and coverage is clean**

Run: `rg 'crap:allow' test-support/` → **no match**;
`rg 'cov:ignore' test-support/src/main.rs` → **no match**. Run:
`devtool run -- cargo xtask check` Expected: green —
`coverage — clean … 0 failures, 0 guard violations, 0 CRAP over threshold`. If
the coverage step reports an uncovered line in `main.rs`, a handler arm wasn't
reached — check the in-process test drove all three DB commands via `run` (not
the handlers directly).

- [x] **Step 6: Commit.**

```bash
git add test-support/src/main.rs
git commit -m "refactor(test-support): dispatch via run + handlers, cover DB arms (#232)"
```

Run `devtool run -- cargo xtask check` first so the pre-commit gate passes clean
(**jaunder-commit**).

---

## Self-review notes

- **Spec coverage:** AC#1 → Step 5 (`rg crap:allow`); AC#2 → Step 5
  (`rg cov:ignore`); AC#3 → Step 3 (thin `main`, no match); AC#4 → Step 3
  (`run` + handlers); AC#5 → Steps 1/4 (in-process DB tests) + Step 5 (0
  uncovered lines); AC#6 → Step 4 (`tests/cli.rs` unmodified, no `lib.rs`
  change); AC#7 → Step 5 (gate).
- **No placeholders:** every step carries real Rust + exact commands.
- **Type consistency:** handler signatures match `run`'s call sites (`&db`,
  `&username`, `display_name.as_deref()`, `&stream`); `cmd_reset_mail`/
  `cmd_capture_path` are sync (`run`'s arms don't `.await` them);
  `Cli { command }` matches the single-field `Cli` struct.
