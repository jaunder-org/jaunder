use common::mailer::{test_utils::CapturingMailSender, MailSender};
use jaunder::storage::{
    open_database, AppState, DbConnectOptions, PostgresAtomicOps, PostgresEmailVerificationStorage,
    PostgresInviteStorage, PostgresPasswordResetStorage, PostgresSessionStorage,
    PostgresSiteConfigStorage, PostgresUserStorage, SqliteAtomicOps,
    SqliteEmailVerificationStorage, SqliteInviteStorage, SqlitePasswordResetStorage,
    SqliteSessionStorage, SqliteSiteConfigStorage, SqliteUserStorage,
};
use leptos::prelude::LeptosOptions;
use std::sync::{Arc, OnceLock};
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
    std::env::var("JAUNDER_PG_TEST_URL")
        .unwrap_or_else(|_| "postgres://jaunder@127.0.0.1:55432/jaunder".to_owned())
        .parse()
        .unwrap()
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

pub async fn test_state(base: &TempDir) -> Arc<AppState> {
    if std::env::var("JAUNDER_PG_TEST_URL").is_ok() {
        reset_postgres_schema().await;
        open_database(&postgres_url()).await.unwrap()
    } else {
        open_database(&sqlite_url(base)).await.unwrap()
    }
}

pub async fn test_state_with_mailer(base: &TempDir) -> (Arc<AppState>, Arc<CapturingMailSender>) {
    let mailer = Arc::new(CapturingMailSender::new());
    let state = if std::env::var("JAUNDER_PG_TEST_URL").is_ok() {
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
            password_resets: Arc::new(PostgresPasswordResetStorage::new(pool)),
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
            password_resets: Arc::new(SqlitePasswordResetStorage::new(pool)),
            mailer: mailer.clone() as Arc<dyn MailSender>,
        })
    };
    (state, mailer)
}
