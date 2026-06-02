# Move PostgreSQL Bootstrap Logic Into `storage` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Relocate the PostgreSQL admin DDL (`cmd_create_pg_db`'s machinery) and the cross-backend `database_has_users` check out of `server/src/commands.rs` into the `storage` crate, behind a typed `PgBootstrapError`, so `storage` owns all SQL/connections and `server` only orchestrates.

**Architecture:** A new `storage::postgres::bootstrap` module exposes `create_postgres_database_and_role` returning a typed `PgBootstrapError`; the SQLSTATE→meaning mapping and SQL quoting become private to it. `database_has_users` moves to the cross-backend `storage::db` module. `cmd_create_pg_db` keeps CLI URL validation and maps the typed error to user-facing messages via a new pure `describe_bootstrap_error` helper.

**Tech Stack:** Rust, sqlx (Postgres/SQLite), thiserror, anyhow (server side), cargo test / `scripts/verify`.

**Reference spec:** `docs/superpowers/specs/2026-06-02-pg-bootstrap-to-storage-design.md`

---

### Task 1: Add `storage::postgres::bootstrap` with typed error + moved DDL

**Files:**
- Create: `storage/src/postgres/bootstrap.rs`
- Modify: `storage/src/postgres/mod.rs` (declare + re-export module)
- Modify: `storage/src/lib.rs:39-45` (add to the `pub use postgres::{…}` block)

- [ ] **Step 1: Create the module file**

Create `storage/src/postgres/bootstrap.rs` with this exact content:

```rust
//! PostgreSQL administrative bootstrap: creating the application role and the
//! database it owns, using superuser (bootstrap) credentials.
//!
//! These are DDL utility statements (`CREATE ROLE`, `CREATE DATABASE`) whose
//! identifiers and password literals cannot be supplied through bind
//! placeholders, so the SQL is assembled with explicit quoting helpers.

use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, PgConnection};

/// Error returned by [`create_postgres_database_and_role`].
#[derive(Debug, thiserror::Error)]
pub enum PgBootstrapError {
    /// The application role already existed (SQLSTATE 42710).
    #[error("application role '{0}' already exists")]
    RoleExists(String),
    /// The application database already existed (SQLSTATE 42P04).
    #[error("database '{0}' already exists")]
    DatabaseExists(String),
    /// Any other connection or statement failure.
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

/// Creates the application role and the database it owns, connecting with the
/// supplied bootstrap (superuser) options.
///
/// Connects with `bootstrap` directly; it does NOT apply
/// [`resolved_postgres_options`](crate::resolved_postgres_options) env-password
/// resolution, because bootstrap credentials come from the URL, not the
/// application environment.
///
/// # Errors
///
/// Returns [`PgBootstrapError::RoleExists`] or
/// [`PgBootstrapError::DatabaseExists`] when the role or database already
/// exists, or [`PgBootstrapError::Sqlx`] for any other failure.
pub async fn create_postgres_database_and_role(
    bootstrap: &PgConnectOptions,
    app_role: &str,
    app_role_password: &str,
    database_name: &str,
) -> Result<(), PgBootstrapError> {
    let mut admin_conn = PgConnection::connect_with(bootstrap).await?;

    // The role name is an identifier and the password appears in a utility
    // statement, so this SQL must be assembled with the quoting helpers rather
    // than query placeholders.
    let role_sql = format!(
        "CREATE ROLE {} WITH LOGIN PASSWORD {}",
        quote_postgres_identifier(app_role),
        quote_postgres_literal(app_role_password),
    );
    if !execute_utility(&mut admin_conn, &role_sql, "42710").await? {
        return Err(PgBootstrapError::RoleExists(app_role.to_owned()));
    }

    // CREATE DATABASE ... OWNER ... is another identifier-bearing utility
    // statement, so placeholders are not usable here either.
    let create_db_sql = format!(
        "CREATE DATABASE {} OWNER {}",
        quote_postgres_identifier(database_name),
        quote_postgres_identifier(app_role),
    );
    if !execute_utility(&mut admin_conn, &create_db_sql, "42P04").await? {
        return Err(PgBootstrapError::DatabaseExists(database_name.to_owned()));
    }

    Ok(())
}

/// Runs a utility statement. Returns `Ok(true)` on success, `Ok(false)` when it
/// fails with `already_exists_code` (the benign "already exists" case), and
/// `Err` for any other database error.
async fn execute_utility(
    conn: &mut PgConnection,
    sql: &str,
    already_exists_code: &str,
) -> Result<bool, sqlx::Error> {
    match sqlx::query(sql).execute(&mut *conn).await {
        Ok(_) => Ok(true),
        Err(sqlx::Error::Database(ref db_error))
            if pg_error_code_matches(db_error.code().as_deref(), already_exists_code) =>
        {
            Ok(false)
        }
        Err(other) => Err(other),
    }
}

fn quote_postgres_identifier(name: &str) -> String {
    // PostgreSQL role/database names are identifiers, not data values, so they
    // cannot be supplied through bind placeholders. Administrative utility
    // statements such as CREATE ROLE and CREATE DATABASE therefore require
    // validated identifier quoting when assembling SQL dynamically.
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn quote_postgres_literal(value: &str) -> String {
    // PostgreSQL also rejects prepared/bound parameters in these utility
    // statements. Password literals therefore need explicit SQL quoting when
    // used in CREATE ROLE statements.
    format!("'{}'", value.replace('\'', "''"))
}

fn pg_error_code_matches(code: Option<&str>, expected: &str) -> bool {
    code == Some(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pg_error_code_matches_returns_true_for_exact_match() {
        assert!(pg_error_code_matches(Some("42710"), "42710"));
    }

    #[test]
    fn pg_error_code_matches_returns_false_for_different_code() {
        assert!(!pg_error_code_matches(Some("42000"), "42710"));
    }

    #[test]
    fn pg_error_code_matches_returns_false_when_no_code() {
        assert!(!pg_error_code_matches(None, "42710"));
    }

    #[test]
    fn quote_postgres_identifier_wraps_and_escapes() {
        assert_eq!(quote_postgres_identifier("users"), "\"users\"");
        assert_eq!(quote_postgres_identifier("user\"name"), "\"user\"\"name\"");
    }

    #[test]
    fn quote_postgres_literal_wraps_and_escapes() {
        assert_eq!(quote_postgres_literal("password"), "'password'");
        assert_eq!(quote_postgres_literal("can't"), "'can''t'");
    }
}
```

- [ ] **Step 2: Declare and re-export the module in `postgres/mod.rs`**

In `storage/src/postgres/mod.rs`, after the existing `pub(crate) mod backup;` line (line 44), add:

```rust
mod bootstrap;
pub use bootstrap::{create_postgres_database_and_role, PgBootstrapError};
```

- [ ] **Step 3: Re-export from the crate root**

In `storage/src/lib.rs`, extend the `pub use postgres::{ … };` block (lines 39-45) to include the two new names, alphabetically near the front:

```rust
pub use postgres::{
    create_postgres_database_and_role, resolved_postgres_options, PgBootstrapError,
    PostgresAtomicOps, PostgresEmailVerificationStorage, PostgresFeedCacheStorage,
    PostgresFeedEventStorage, PostgresInviteStorage, PostgresMediaStorage,
    PostgresPasswordResetStorage, PostgresPostStorage, PostgresSessionStorage,
    PostgresSiteConfigStorage, PostgresUserConfigStorage, PostgresUserStorage,
};
```

- [ ] **Step 4: Build and run the new unit tests**

Run: `cargo test -p storage bootstrap`
Expected: PASS — the 5 tests (`pg_error_code_matches_*`, `quote_postgres_identifier_wraps_and_escapes`, `quote_postgres_literal_wraps_and_escapes`).

If the compiler reports that `db_error.code()` cannot be resolved, add `use sqlx::error::DatabaseError;` to the imports and rebuild.

- [ ] **Step 5: Lint**

Run: `cargo clippy -p storage -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add storage/src/postgres/bootstrap.rs storage/src/postgres/mod.rs storage/src/lib.rs
git commit -m "feat(storage): add postgres bootstrap (create role + database)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Move `database_has_users` into `storage::db`

**Files:**
- Modify: `storage/src/db.rs` (add function + imports)

- [ ] **Step 1: Add imports**

In `storage/src/db.rs`, the existing sqlx imports (lines 9-11) are:

```rust
use sqlx::postgres::PgConnectOptions;
use sqlx::sqlite::SqliteConnectOptions;
```

Add below them:

```rust
use sqlx::{PgPool, SqlitePool};
```

- [ ] **Step 2: Add the function**

Append to `storage/src/db.rs` (after `open_existing_database`, before any `#[cfg(test)]` block):

```rust
/// Returns `true` if the target database already contains at least one user.
///
/// Used as a restore preflight: refusing to restore into a non-empty database.
///
/// # Errors
///
/// Returns the underlying [`sqlx::Error`] if the database cannot be reached or
/// the query fails.
pub async fn database_has_users(options: &DbConnectOptions) -> sqlx::Result<bool> {
    match options {
        DbConnectOptions::Sqlite(options) => {
            let pool = SqlitePool::connect_with(options.clone()).await?;
            Ok(
                sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM users LIMIT 1)")
                    .fetch_one(&pool)
                    .await?
                    != 0,
            )
        }
        DbConnectOptions::Postgres { options, .. } => {
            let options = crate::resolved_postgres_options(options)?;
            let pool = PgPool::connect_with(options).await?;
            Ok(
                sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users LIMIT 1)")
                    .fetch_one(&pool)
                    .await?,
            )
        }
    }
}
```

- [ ] **Step 3: Build**

Run: `cargo build -p storage`
Expected: compiles cleanly. (No new unit test: this path needs a live database and is exercised by the existing nix postgres-VM e2e and the restore flow.)

- [ ] **Step 4: Lint**

Run: `cargo clippy -p storage -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add storage/src/db.rs
git commit -m "feat(storage): add cross-backend database_has_users

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Rewire `commands.rs` to the storage API; delete moved code

**Files:**
- Modify: `server/src/commands.rs`

- [ ] **Step 1: Write the failing test for the error-mapping helper**

In `server/src/commands.rs`, inside `mod tests`, add (this references a `describe_bootstrap_error` function that does not exist yet):

```rust
#[test]
fn describe_bootstrap_error_role_exists_message() {
    let msg = describe_bootstrap_error(storage::PgBootstrapError::RoleExists("alice".to_owned()))
        .to_string();
    assert!(msg.contains("application role 'alice' already exists"));
    assert!(msg.contains("refusing to modify existing role state"));
}

#[test]
fn describe_bootstrap_error_database_exists_message() {
    let msg =
        describe_bootstrap_error(storage::PgBootstrapError::DatabaseExists("blog".to_owned()))
            .to_string();
    assert!(msg.contains("database 'blog' already exists"));
    assert!(msg.contains("refusing to modify existing database state"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p jaunder describe_bootstrap_error`
Expected: FAIL to compile — `cannot find function describe_bootstrap_error`.

- [ ] **Step 3: Add the helper and rewrite `cmd_create_pg_db`**

In `server/src/commands.rs`, replace the whole block from `fn quote_postgres_identifier` (line 54) through the end of `cmd_create_pg_db` (line 152) with:

```rust
/// Maps a [`storage::PgBootstrapError`] to a user-facing CLI error.
fn describe_bootstrap_error(err: storage::PgBootstrapError) -> anyhow::Error {
    match err {
        storage::PgBootstrapError::RoleExists(role) => anyhow::anyhow!(
            "application role '{role}' already exists; refusing to modify existing role state"
        ),
        storage::PgBootstrapError::DatabaseExists(name) => anyhow::anyhow!(
            "database '{name}' already exists; refusing to modify existing database state"
        ),
        other => other.into(),
    }
}

/// Bootstraps a `PostgreSQL` database and application role.
///
/// # Errors
///
/// Returns an error if the bootstrap connection fails, or if the role or
/// database already exists.
pub async fn cmd_create_pg_db(
    bootstrap_db: &str,
    app_db_url: &str,
    app_role_password: &str,
) -> anyhow::Result<()> {
    let bootstrap_options = require_postgres_options(&bootstrap_db.parse()?, "--bootstrap-db")?;
    let app_options = require_postgres_options(&app_db_url.parse()?, "--app-db")?;
    let app_role = app_options.get_username().to_owned();
    let database_name = app_options
        .get_database()
        .ok_or_else(|| anyhow::anyhow!("--app-db must include a PostgreSQL database name"))?
        .to_owned();

    storage::create_postgres_database_and_role(
        &bootstrap_options,
        &app_role,
        app_role_password,
        &database_name,
    )
    .await
    .map_err(describe_bootstrap_error)?;

    println!("PostgreSQL ready: role='{app_role}' database='{database_name}' owner='{app_role}'");
    Ok(())
}
```

This deletes `quote_postgres_identifier`, `quote_postgres_literal`, `pg_error_code_matches`, and `execute_postgres_utility` from `commands.rs`. (`require_postgres_options` is kept.)

- [ ] **Step 4: Replace the local `database_has_users` with the storage call**

In `server/src/commands.rs`, delete the entire local `database_has_users` function (lines 334-355) and update its caller in `ensure_restore_target_empty` (line 320) from:

```rust
    if database_has_users(&storage.db).await? {
```

to:

```rust
    if storage::database_has_users(&storage.db).await? {
```

- [ ] **Step 5: Remove the now-unused tests and imports**

In `mod tests`, delete the five tests now living in `storage` (`pg_error_code_matches_returns_true_for_exact_match`, `pg_error_code_matches_returns_false_for_different_code`, `pg_error_code_matches_returns_false_when_no_code`, `test_quote_postgres_identifier`, `test_quote_postgres_literal`).

Then fix imports: at the top of `commands.rs` the use line is

```rust
use sqlx::{postgres::PgConnectOptions, Connection, PgConnection, PgPool, SqlitePool};
```

`Connection`, `PgConnection`, `PgPool`, and `SqlitePool` are now unused (the DDL and `database_has_users` left the file). Reduce it to only what remains — `require_postgres_options` still returns `PgConnectOptions`:

```rust
use sqlx::postgres::PgConnectOptions;
```

Also remove `resolved_postgres_options` from the `use storage::{ … }` import block if it is no longer referenced (it moved into `storage::db`).

- [ ] **Step 6: Build and run all `commands` tests**

Run: `cargo test -p jaunder commands`
Expected: PASS — including the two new `describe_bootstrap_error_*` tests and the retained `cmd_create_pg_db_rejects_non_postgres_app_db`, `cmd_create_pg_db_requires_database_name`, `test_require_postgres_options`, and the backup/invite/directory tests.

- [ ] **Step 7: Lint the whole workspace**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings (catches any leftover unused imports across crates).

- [ ] **Step 8: Commit**

```bash
git add server/src/commands.rs
git commit -m "refactor(server): use storage for postgres bootstrap and user check

Move the CREATE ROLE/DATABASE DDL and database_has_users into the
storage crate; commands.rs keeps CLI validation and maps the typed
PgBootstrapError to user-facing messages via describe_bootstrap_error.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Full verification

**Files:** none (verification only; may add a test if coverage regresses).

- [ ] **Step 1: Run the full verification suite**

Run: `scripts/verify`
Expected: `--- verify: all checks passed ---` (fmt, leptosfmt, prettier, cargo-deny, clippy, coverage+CRAP, nix VM/e2e).

- [ ] **Step 2: Handle a per-file coverage regression if `scripts/check-coverage` fails on `commands.rs`**

If verify fails only on a `server/src/commands.rs` coverage drop, it is because well-tested helpers left the file. The new `describe_bootstrap_error_*` tests should offset this. If it still regresses, run `scripts/check-coverage --investigate` to see the uncovered lines and add a focused unit test for the remaining gap (do not lower the baseline). Re-run `scripts/verify`.

- [ ] **Step 3: Confirm CRAP did not regress**

The run updates `.crap-manifest.json` and `.coverage-manifest.json` on success. `cmd_create_pg_db`'s cyclomatic complexity should drop; the new `storage` functions are new manifest entries (not regressions). Confirm verify reports CRAP OK.

- [ ] **Step 4: Commit the regenerated manifests (and any added test)**

```bash
git add .coverage-manifest.json .crap-manifest.json
git commit -m "chore: update coverage and CRAP manifests after pg bootstrap move

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

(If Step 2 added a test, include that file in this commit.)

---

## Notes for the executor

- Do not commit unless each task's build/test/lint steps pass. One clean commit per task.
- `scripts/verify` is the authority — `cargo test`/`clippy` per crate are fast inner-loop checks only.
- Behavior must be preserved exactly: same SQL, same connection semantics (bootstrap connects with URL creds, no env-password resolution), same user-facing messages.
