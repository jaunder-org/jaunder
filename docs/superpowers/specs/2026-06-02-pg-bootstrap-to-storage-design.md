# Move PostgreSQL bootstrap logic from `server` to `storage`

Date: 2026-06-02

## Problem

`server/src/commands.rs` mixes two altitudes of code. Most of it is thin CLI
orchestration that delegates to `storage::` functions and the storage trait
objects (`cmd_init`, `cmd_user_create`, `cmd_user_invite`, `cmd_smtp_test`,
`cmd_backup`, `cmd_restore`, `cmd_serve`) — correctly backend-agnostic.

But a substantial chunk is raw PostgreSQL data-layer logic that breaks the
boundary the rest of the codebase respects (`storage/` owns all SQL and
connection handling; `server/` orchestrates):

- `cmd_create_pg_db` plus `quote_postgres_identifier`, `quote_postgres_literal`,
  `pg_error_code_matches`, `execute_postgres_utility` — ~80 lines of
  `sqlx::PgConnection`, `CREATE ROLE` / `CREATE DATABASE` DDL, SQL
  identifier/literal quoting, and SQLSTATE error-code mapping.
- `database_has_users` — hand-rolled pool construction and raw
  `SELECT EXISTS(...)` for both backends.

This is exactly the backend-specific knowledge `storage/` encapsulates
everywhere else (`storage/src/postgres/` already owns connection concerns such
as `resolved_postgres_options`).

### Legitimate vs. illegitimate asymmetry

SQLite has no "create role / create database" step — the file is created on
connect — so there genuinely is no symmetric SQLite counterpart to
`cmd_create_pg_db`. The fix is not to invent one; it is that the PostgreSQL
bootstrap *operation* belongs in `storage/src/postgres/`, which SQLite simply
does not need.

## Goal

Restore the storage/server boundary by moving the PostgreSQL admin DDL and the
cross-backend empty-database check into `storage/`, leaving only CLI
orchestration and flag-level validation in `commands.rs`. Behavior-preserving.

## What moves, what stays

### Moves into `storage/` (the mechanism)

- `quote_postgres_identifier`, `quote_postgres_literal`, `pg_error_code_matches`,
  `execute_postgres_utility` — become **private** helpers in storage.
- The `CREATE ROLE` / `CREATE DATABASE` assembly currently inline in
  `cmd_create_pg_db`.
- `database_has_users` (both backend arms + pool construction).

### Stays in `commands.rs` (orchestration + CLI presentation)

- `require_postgres_options` and the `--app-db must include a PostgreSQL
  database name` check — flag-named CLI validations.
- The `PostgreSQL ready: role=… database=…` success print (the command already
  holds the names).
- `ensure_restore_target_empty` — its DB check delegates to storage; the
  media-directory filesystem check stays (a `storage_path` concern the server
  owns).
- The typed-error → user-facing-message formatting (the "refusing to modify…"
  wording).

## New storage API (typed-error boundary)

### `storage/src/postgres/bootstrap.rs` (new file)

```rust
#[derive(Debug, thiserror::Error)]
pub enum PgBootstrapError {
    #[error("application role '{0}' already exists")]
    RoleExists(String),
    #[error("database '{0}' already exists")]
    DatabaseExists(String),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

/// Connects with `bootstrap` (superuser) options and creates the application
/// role and the database it owns.
///
/// Connects with the supplied bootstrap options directly; it does NOT apply
/// `resolved_postgres_options` env-password resolution, because bootstrap
/// (superuser) credentials come from the URL, not the application env.
pub async fn create_postgres_database_and_role(
    bootstrap: &PgConnectOptions,
    app_role: &str,
    app_role_password: &str,
    database_name: &str,
) -> Result<(), PgBootstrapError>;
```

The function maps SQLSTATE `42710` → `RoleExists` and `42P04` →
`DatabaseExists` internally; the quoting and execute helpers become private to
this module.

The error type follows the existing `storage::BackupError` style (thiserror,
`#[error(...)]`, `#[from] sqlx::Error`).

### `storage/src/db.rs`

```rust
/// Returns true if the target database already contains any users. Used as a
/// restore preflight.
pub async fn database_has_users(options: &DbConnectOptions) -> sqlx::Result<bool>;
```

Cross-backend (matches on `DbConnectOptions`), so it lives at the `db`
abstraction level rather than in `postgres/` or `sqlite/`. The PostgreSQL arm
uses `resolved_postgres_options` (unchanged from current behavior).

### Re-exports

- `storage/src/postgres/mod.rs`: `mod bootstrap;` +
  `pub use bootstrap::{create_postgres_database_and_role, PgBootstrapError};`
  (follows the existing `mod x; pub use x::Type;` pattern).
- `storage/src/lib.rs`: add `create_postgres_database_and_role, PgBootstrapError`
  to the `pub use postgres::{…}` block.
- `database_has_users` rides the existing `pub use db::*`.

## `cmd_create_pg_db` after the move

Parses/validates the URLs (unchanged), extracts `app_role` and `database_name`
(the `--app-db` database-name check stays), calls the storage function, and maps
the typed error via a small pure helper:

```rust
fn describe_bootstrap_error(err: PgBootstrapError) -> anyhow::Error {
    match err {
        PgBootstrapError::RoleExists(role) => anyhow::anyhow!(
            "application role '{role}' already exists; refusing to modify existing role state"
        ),
        PgBootstrapError::DatabaseExists(name) => anyhow::anyhow!(
            "database '{name}' already exists; refusing to modify existing database state"
        ),
        other => other.into(),
    }
}
```

The command calls `storage::create_postgres_database_and_role(...)` and on
`Err(e)` returns `describe_bootstrap_error(e)`. Extracting the mapping makes the
user-facing wording unit-testable without a live database.

## Testing

- **Move with the code:** the quoting unit tests (`test_quote_postgres_identifier`,
  `test_quote_postgres_literal`) and the `pg_error_code_matches_*` tests →
  private-fn tests in `bootstrap.rs`.
- **Stay in `commands.rs`:** `test_require_postgres_options`,
  `cmd_create_pg_db_rejects_non_postgres_app_db`,
  `cmd_create_pg_db_requires_database_name` — all exercise CLI validation that
  runs *before* storage is called, so they remain valid.
- **New:** a `describe_bootstrap_error` mapping test in `commands.rs`, covering
  the `RoleExists` and `DatabaseExists` wording (currently uncovered by unit
  tests).
- The live DDL path stays covered by the existing nix postgres-VM e2e.

## Verification & risk

- Behavior-preserving move. Run `scripts/verify` (fmt, clippy, coverage+CRAP,
  nix VM/e2e).
- Removing the tested quoting helpers from `commands.rs` shifts that file's
  coverage percentage; if `scripts/check-coverage` flags a per-file regression,
  cover it with the new `describe_bootstrap_error` test.
- The new storage functions land at unit-coverage 0 (e2e-only, like the existing
  `storage/src/postgres/backup.rs` functions). They are new manifest entries, so
  they do not trip the CRAP regression gate, and `cmd_create_pg_db`'s own
  cyclomatic complexity drops once the DDL leaves it.

## Out of scope

- No changes to the CLI surface (`cli.rs` flags, `main.rs` wiring) beyond what
  the unchanged `cmd_create_pg_db` signature already requires.
- No new SQLite bootstrap operation (none is needed).
- No unrelated refactoring of the backend-agnostic command functions.
