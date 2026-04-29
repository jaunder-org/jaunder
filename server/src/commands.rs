use std::{
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use sqlx::{postgres::PgConnectOptions, Connection, PgConnection, PgPool, SqlitePool};

use crate::cli::StorageArgs;
use crate::mailer::LettreMailSender;
use crate::password::Password;
use crate::storage::{
    export_backup, resolved_postgres_options, BackupExportOptions, BackupMode, DbConnectOptions,
};
use crate::storage::{init_storage, open_database, open_existing_database};
use crate::username::Username;
use common::mailer::{EmailMessage, MailSender};
use common::smtp::load_smtp_config;
use leptos::prelude::{Env, LeptosOptions};

pub async fn cmd_init(storage: &StorageArgs, skip_if_exists: bool) -> anyhow::Result<()> {
    match init_storage(&storage.storage_path) {
        Ok(()) => {}
        Err(e) if skip_if_exists && e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    open_database(&storage.db).await?;
    println!(
        "Initialized: storage={} db={}",
        storage.storage_path.display(),
        storage.db,
    );
    Ok(())
}

fn require_postgres_options(
    opts: &DbConnectOptions,
    label: &str,
) -> anyhow::Result<PgConnectOptions> {
    match opts {
        DbConnectOptions::Postgres { options, .. } => Ok(options.clone()),
        _ => Err(anyhow::anyhow!("{label} must be a PostgreSQL URL")),
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
    // statements. For example, PREPARE ... AS ALTER ROLE ... PASSWORD $1 fails
    // with a syntax error at ALTER. Password literals therefore need explicit
    // SQL quoting when used in CREATE ROLE statements.
    format!("'{}'", value.replace('\'', "''"))
}

async fn execute_postgres_utility(
    conn: &mut PgConnection,
    sql: &str,
    expected_error_code: &str,
    expected_error_message: String,
) -> anyhow::Result<()> {
    if let Err(error) = sqlx::query(sql).execute(conn).await {
        return match error {
            sqlx::Error::Database(db_error)
                if db_error.code().as_deref() == Some(expected_error_code) =>
            {
                Err(anyhow::anyhow!(expected_error_message))
            }
            other => Err(other.into()),
        };
    }

    Ok(())
}

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

    let mut admin_conn = PgConnection::connect_with(&bootstrap_options).await?;

    // The role name is an identifier and the password appears in a
    // PostgreSQL utility statement, so this SQL has to be assembled using
    // the quoting helpers above rather than regular query placeholders.
    let role_sql = format!(
        "CREATE ROLE {} WITH LOGIN PASSWORD {}",
        quote_postgres_identifier(&app_role),
        quote_postgres_literal(app_role_password),
    );
    execute_postgres_utility(
        &mut admin_conn,
        &role_sql,
        "42710",
        format!(
            "application role '{}' already exists; refusing to modify existing role state",
            app_role
        ),
    )
    .await?;

    // CREATE DATABASE ... OWNER ... is another utility statement using
    // identifiers, so placeholders are not usable here either.
    let create_db_sql = format!(
        "CREATE DATABASE {} OWNER {}",
        quote_postgres_identifier(&database_name),
        quote_postgres_identifier(&app_role),
    );
    execute_postgres_utility(
        &mut admin_conn,
        &create_db_sql,
        "42P04",
        format!(
            "database '{}' already exists; refusing to modify existing database state",
            database_name
        ),
    )
    .await?;

    println!(
        "PostgreSQL ready: role='{}' database='{}' owner='{}'",
        app_role, database_name, app_role
    );
    Ok(())
}

pub async fn cmd_user_create(
    storage: &StorageArgs,
    username: &Username,
    password: Option<Password>,
    display_name: Option<&str>,
    is_operator: bool,
) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let password = match password {
        Some(p) => p,
        None => {
            let p1 = rpassword::prompt_password("Password: ")?;
            let p2 = rpassword::prompt_password("Confirm password: ")?;
            if p1 != p2 {
                return Err(anyhow::anyhow!("passwords do not match"));
            }
            p1.parse::<Password>().map_err(|e| anyhow::anyhow!("{e}"))?
        }
    };

    let user_id = state
        .users
        .create_user(username, &password, display_name, is_operator)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Created user '{}' with id {user_id}", username);
    Ok(())
}

pub async fn cmd_user_invite(storage: &StorageArgs, expires_in: Option<u64>) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let hours_u64 = expires_in.unwrap_or(168);
    let hours = i64::try_from(hours_u64)
        .map_err(|_| anyhow::anyhow!("--expires-in value {hours_u64} is too large"))?;
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(hours);

    let code = state.invites.create_invite(expires_at).await?;
    println!("{code}");
    Ok(())
}

pub async fn cmd_smtp_test(storage: &StorageArgs, to: &str) -> anyhow::Result<()> {
    let state = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let smtp_config = load_smtp_config(state.site_config.as_ref())
        .await
        .map_err(|e| anyhow::anyhow!("SMTP misconfigured: {e}"))?
        .ok_or_else(|| anyhow::anyhow!("SMTP is not configured"))?;

    let mailer = LettreMailSender::from_config(&smtp_config)
        .map_err(|e| anyhow::anyhow!("Failed to build SMTP transport: {e}"))?;

    let to_addr: email_address::EmailAddress = to
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid email address '{to}': {e}"))?;

    let message = EmailMessage {
        from: None,
        to: vec![to_addr],
        subject: "Jaunder SMTP test".to_owned(),
        body_text:
            "This is a test message from Jaunder. If you received it, SMTP is working correctly."
                .to_owned(),
    };

    mailer
        .send_email(&message)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send test email: {e}"))?;

    println!("Test email sent successfully to {to}");
    Ok(())
}

pub async fn cmd_backup(storage: &StorageArgs, path: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let destination_path = path.unwrap_or_else(|| default_backup_path(storage));
    let manifest = export_backup(BackupExportOptions {
        database: &storage.db,
        media_path: &storage.storage_path.join("media"),
        destination_path: &destination_path,
        mode: BackupMode::Directory,
    })
    .await?;

    println!(
        "Backup complete: path={} tables={}",
        destination_path.display(),
        manifest.tables.len()
    );
    Ok(destination_path)
}

pub async fn cmd_restore(storage: &StorageArgs, path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "backup path does not exist: {}",
            path.display()
        ));
    }
    ensure_restore_target_empty(storage).await?;
    Err(anyhow::anyhow!(
        "restore import is not implemented yet; safety pre-flight passed"
    ))
}

fn default_backup_path(storage: &StorageArgs) -> PathBuf {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    storage
        .storage_path
        .join("backups")
        .join(format!("backup-{timestamp}"))
}

async fn ensure_restore_target_empty(storage: &StorageArgs) -> anyhow::Result<()> {
    if database_has_users(&storage.db).await? {
        return Err(anyhow::anyhow!(
            "refusing to restore into a non-empty database"
        ));
    }
    let media_path = storage.storage_path.join("media");
    if directory_has_entries(&media_path)? {
        return Err(anyhow::anyhow!(
            "refusing to restore into a non-empty media directory"
        ));
    }
    Ok(())
}

async fn database_has_users(db: &DbConnectOptions) -> anyhow::Result<bool> {
    match db {
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
            let options = resolved_postgres_options(options)?;
            let pool = PgPool::connect_with(options).await?;
            Ok(
                sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users LIMIT 1)")
                    .fetch_one(&pool)
                    .await?,
            )
        }
    }
}

fn directory_has_entries(path: &Path) -> io::Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            if directory_has_entries(&entry.path())? {
                return Ok(true);
            }
        } else {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn cmd_serve(storage: &StorageArgs, bind: SocketAddr, prod: bool) -> anyhow::Result<()> {
    crate::observability::init_tracing();

    let db = match open_existing_database(&storage.db).await {
        Ok(db) => db,
        Err(_) if !prod => {
            // Dev mode: auto-initialize so `cargo leptos end-to-end` and
            // `cargo leptos serve` work without a manual `jaunder init`.
            tracing::warn!(
                storage_path = %storage.storage_path.display(),
                db = %storage.db,
                "Database not found — auto-initializing (dev mode): storage={} db={}",
                storage.storage_path.display(),
                storage.db,
            );
            cmd_init(storage, true).await?;
            open_existing_database(&storage.db)
                .await
                .map_err(|e| anyhow::anyhow!("{e}; auto-init failed"))?
        }
        Err(e) => return Err(anyhow::anyhow!("{e}; run `jaunder init` first")),
    };

    let leptos_options = LeptosOptions::builder()
        .output_name("jaunder")
        .site_root("target/site")
        .site_pkg_dir("pkg")
        .env(if prod { Env::PROD } else { Env::DEV })
        .site_addr(bind)
        .build();

    let backup_scheduler =
        crate::start_backup_worker(db.clone(), storage.db.clone(), storage.storage_path.clone())
            .await?;
    let router = crate::create_router(leptos_options, db, prod);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(bind = %bind, prod, "starting HTTP server");
    let _backup_scheduler = backup_scheduler;
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::DbConnectOptions;

    #[test]
    fn test_quote_postgres_identifier() {
        assert_eq!(quote_postgres_identifier("users"), "\"users\"");
        assert_eq!(quote_postgres_identifier("user\"name"), "\"user\"\"name\"");
    }

    #[test]
    fn test_quote_postgres_literal() {
        assert_eq!(quote_postgres_literal("password"), "'password'");
        assert_eq!(quote_postgres_literal("can't"), "'can''t'");
    }

    #[test]
    fn test_require_postgres_options() {
        let pg_url = "postgres://user:pass@localhost/db";
        let opts: DbConnectOptions = pg_url.parse().unwrap();
        assert!(require_postgres_options(&opts, "test").is_ok());

        let sqlite_url = "sqlite:test.db";
        let opts: DbConnectOptions = sqlite_url.parse().unwrap();
        let err = require_postgres_options(&opts, "test").unwrap_err();
        assert!(err.to_string().contains("test must be a PostgreSQL URL"));
    }

    #[tokio::test]
    async fn cmd_create_pg_db_rejects_non_postgres_app_db() {
        let err = cmd_create_pg_db(
            "postgres://bootstrap:secret@localhost/postgres",
            "sqlite:/tmp/jaunder.db",
            "secret",
        )
        .await
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("--app-db must be a PostgreSQL URL"));
    }

    #[tokio::test]
    async fn cmd_create_pg_db_requires_database_name() {
        let err = cmd_create_pg_db(
            "postgres://bootstrap:secret@localhost/postgres",
            "postgres://app:secret@localhost",
            "secret",
        )
        .await
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("--app-db must include a PostgreSQL database name"));
    }

    #[test]
    fn default_backup_path_is_under_storage_backups() {
        let storage = StorageArgs {
            storage_path: PathBuf::from("/tmp/jaunder"),
            db: "sqlite:/tmp/jaunder.db".parse().expect("sqlite db"),
        };

        let path = default_backup_path(&storage);

        assert!(path.starts_with("/tmp/jaunder/backups"));
    }

    #[test]
    fn directory_has_entries_handles_missing_empty_and_nested_paths() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        assert!(!directory_has_entries(&temp.path().join("missing")).expect("missing"));

        let empty = temp.path().join("empty");
        std::fs::create_dir(&empty).expect("empty dir");
        assert!(!directory_has_entries(&empty).expect("empty"));

        let nested = temp.path().join("nested");
        std::fs::create_dir(&nested).expect("nested dir");
        std::fs::write(nested.join("file.txt"), "content").expect("nested file");
        assert!(directory_has_entries(temp.path()).expect("nested"));
    }
}
