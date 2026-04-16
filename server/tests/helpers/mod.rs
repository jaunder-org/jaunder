#![allow(dead_code)]

use common::mailer::{test_utils::CapturingMailSender, MailSender};
use jaunder::storage::{
    open_database, AppState, DbConnectOptions, PostgresAtomicOps, PostgresEmailVerificationStorage,
    PostgresInviteStorage, PostgresPasswordResetStorage, PostgresPostStorage,
    PostgresSessionStorage, PostgresSiteConfigStorage, PostgresUserStorage, SqliteAtomicOps,
    SqliteEmailVerificationStorage, SqliteInviteStorage, SqlitePasswordResetStorage,
    SqlitePostStorage, SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserStorage,
};
use leptos::prelude::LeptosOptions;
use sqlx::Connection;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, OnceLock,
};
use tempfile::TempDir;

pub fn ensure_server_fns_registered() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        server_fn::axum::register_explicit::<web::auth::CurrentUser>();
        server_fn::axum::register_explicit::<web::auth::GetRegistrationPolicy>();
        server_fn::axum::register_explicit::<web::auth::Register>();
        server_fn::axum::register_explicit::<web::auth::Login>();
        server_fn::axum::register_explicit::<web::auth::Logout>();
        server_fn::axum::register_explicit::<web::email::RequestEmailVerification>();
        server_fn::axum::register_explicit::<web::email::VerifyEmail>();
        server_fn::axum::register_explicit::<web::profile::GetProfile>();
        server_fn::axum::register_explicit::<web::profile::UpdateProfile>();
        server_fn::axum::register_explicit::<web::sessions::ListSessions>();
        server_fn::axum::register_explicit::<web::sessions::RevokeSession>();
        server_fn::axum::register_explicit::<web::invites::CreateInvite>();
        server_fn::axum::register_explicit::<web::invites::ListInvites>();
        server_fn::axum::register_explicit::<web::password_reset::RequestPasswordReset>();
        server_fn::axum::register_explicit::<web::password_reset::ConfirmPasswordReset>();
        server_fn::axum::register_explicit::<web::posts::CreatePost>();
        server_fn::axum::register_explicit::<web::posts::GetPost>();
        server_fn::axum::register_explicit::<web::posts::GetPostPreview>();
        server_fn::axum::register_explicit::<web::posts::UpdatePost>();
        server_fn::axum::register_explicit::<web::posts::ListDrafts>();
        server_fn::axum::register_explicit::<web::posts::PublishPost>();
        server_fn::axum::register_explicit::<web::posts::ListUserPosts>();
    });
}

pub fn test_options() -> LeptosOptions {
    LeptosOptions::builder().output_name("test").build()
}

pub fn sqlite_url(base: &TempDir) -> DbConnectOptions {
    format!("sqlite:{}", base.path().join("test.db").display())
        .parse()
        .unwrap()
}

pub fn postgres_url() -> DbConnectOptions {
    postgres_url_string().parse().unwrap()
}

pub fn postgres_testing_enabled() -> bool {
    std::env::var("JAUNDER_PG_TEST_URL").is_ok()
}

pub fn postgres_bootstrap_url() -> String {
    std::env::var("JAUNDER_PG_BOOTSTRAP_TEST_URL")
        .unwrap_or_else(|_| "postgres://postgres@127.0.0.1:55432/postgres".to_owned())
}

pub fn postgres_url_string() -> String {
    std::env::var("JAUNDER_PG_TEST_URL")
        .unwrap_or_else(|_| "postgres://jaunder@127.0.0.1:55432/jaunder".to_owned())
}

pub fn postgres_test_authority() -> String {
    let bootstrap = postgres_bootstrap_url();
    let without_scheme = bootstrap
        .strip_prefix("postgres://")
        .or_else(|| bootstrap.strip_prefix("postgresql://"))
        .unwrap_or(&bootstrap);
    let after_credentials = without_scheme
        .rsplit_once('@')
        .map(|(_, authority_and_path)| authority_and_path)
        .unwrap_or(without_scheme);
    after_credentials
        .split('/')
        .next()
        .expect("bootstrap URL should include an authority")
        .to_owned()
}

pub async fn reset_postgres_schema() {
    use sqlx::PgPool;
    let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
        panic!("expected postgres options");
    };
    let pool = PgPool::connect_with(options).await.unwrap();
    sqlx::query("DROP SCHEMA public CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
}

fn quote_postgres_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn postgres_url_with_db_name(db_name: &str) -> String {
    let template = postgres_url_string();
    let (base, query) = template
        .split_once('?')
        .map_or((template.as_str(), None), |(base, query)| {
            (base, Some(query))
        });
    let (prefix, _) = base
        .rsplit_once('/')
        .expect("PostgreSQL test URL should include a database name");
    match query {
        Some(query) => format!("{prefix}/{db_name}?{query}"),
        None => format!("{prefix}/{db_name}"),
    }
}

fn unique_postgres_db_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let suffix = COUNTER.fetch_add(1, Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    format!("jaunder_test_{timestamp}_{suffix}")
}

pub fn nonexistent_postgres_url() -> DbConnectOptions {
    postgres_url_with_db_name(&unique_postgres_db_name())
        .parse()
        .unwrap()
}

pub async fn unique_postgres_url() -> DbConnectOptions {
    let db_name = unique_postgres_db_name();

    let bootstrap: sqlx::postgres::PgConnectOptions = postgres_bootstrap_url().parse().unwrap();
    let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
        panic!("expected postgres options");
    };
    let owner = options.get_username();
    assert!(
        !owner.is_empty(),
        "PostgreSQL test URL must include a username"
    );

    let mut admin_conn = sqlx::PgConnection::connect_with(&bootstrap).await.unwrap();
    sqlx::query(&format!(
        "CREATE DATABASE {} OWNER {}",
        quote_postgres_identifier(&db_name),
        quote_postgres_identifier(owner),
    ))
    .execute(&mut admin_conn)
    .await
    .unwrap();

    postgres_url_with_db_name(&db_name).parse().unwrap()
}

pub async fn test_state(base: &TempDir) -> Arc<AppState> {
    if postgres_testing_enabled() {
        reset_postgres_schema().await;
        open_database(&postgres_url()).await.unwrap()
    } else {
        open_database(&sqlite_url(base)).await.unwrap()
    }
}

pub async fn test_state_with_mailer(base: &TempDir) -> (Arc<AppState>, Arc<CapturingMailSender>) {
    let mailer = Arc::new(CapturingMailSender::new());
    let state = if postgres_testing_enabled() {
        reset_postgres_schema().await;
        let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
            panic!("expected postgres options");
        };
        let pool = sqlx::PgPool::connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations/postgres")
            .run(&pool)
            .await
            .unwrap();
        Arc::new(AppState {
            site_config: Arc::new(PostgresSiteConfigStorage::new(pool.clone())),
            users: Arc::new(PostgresUserStorage::new(pool.clone())),
            sessions: Arc::new(PostgresSessionStorage::new(pool.clone())),
            invites: Arc::new(PostgresInviteStorage::new(pool.clone())),
            atomic: Arc::new(PostgresAtomicOps::new(pool.clone())),
            email_verifications: Arc::new(PostgresEmailVerificationStorage::new(pool.clone())),
            password_resets: Arc::new(PostgresPasswordResetStorage::new(pool.clone())),
            posts: Arc::new(PostgresPostStorage::new(pool)),
            mailer: mailer.clone() as Arc<dyn MailSender>,
        })
    } else {
        let pool = sqlx::SqlitePool::connect_with(
            format!("sqlite:{}", base.path().join("test.db").display())
                .parse::<sqlx::sqlite::SqliteConnectOptions>()
                .unwrap()
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::migrate!("./migrations/sqlite")
            .run(&pool)
            .await
            .unwrap();
        Arc::new(AppState {
            site_config: Arc::new(SqliteSiteConfigStorage::new(pool.clone())),
            users: Arc::new(SqliteUserStorage::new(pool.clone())),
            sessions: Arc::new(SqliteSessionStorage::new(pool.clone())),
            invites: Arc::new(SqliteInviteStorage::new(pool.clone())),
            atomic: Arc::new(SqliteAtomicOps::new(pool.clone())),
            email_verifications: Arc::new(SqliteEmailVerificationStorage::new(pool.clone())),
            password_resets: Arc::new(SqlitePasswordResetStorage::new(pool.clone())),
            posts: Arc::new(SqlitePostStorage::new(pool)),
            mailer: mailer.clone() as Arc<dyn MailSender>,
        })
    };
    (state, mailer)
}
