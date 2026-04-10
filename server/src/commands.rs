use std::{io, net::SocketAddr};

use sqlx::{postgres::PgConnectOptions, Connection, PgConnection};

use crate::cli::StorageArgs;
use crate::mailer::LettreMailSender;
use crate::password::Password;
use crate::storage::DbConnectOptions;
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
        .create_user(username, &password, display_name)
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

pub async fn cmd_serve(storage: &StorageArgs, bind: SocketAddr, prod: bool) -> anyhow::Result<()> {
    let db = open_existing_database(&storage.db)
        .await
        .map_err(|e| anyhow::anyhow!("{e}; run `jaunder init` first"))?;

    let leptos_options = LeptosOptions::builder()
        .output_name("jaunder")
        .site_root("target/site")
        .site_pkg_dir("pkg")
        .env(if prod { Env::PROD } else { Env::DEV })
        .site_addr(bind)
        .build();

    let router = crate::create_router(leptos_options, db, prod);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
