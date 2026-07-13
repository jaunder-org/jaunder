use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::LevelFilter;
use sqlx::postgres::PgConnectOptions;
use sqlx::ConnectOptions;
use sqlx::PgPool;

mod site_config;
pub use site_config::PostgresSiteConfigStorage;

mod users;
pub use users::PostgresUserStorage;

mod sessions;
pub use sessions::PostgresSessionStorage;

mod invites;
pub use invites::PostgresInviteStorage;

mod email_verifications;
pub use email_verifications::PostgresEmailVerificationStorage;

mod feed_cache;
pub use feed_cache::PostgresFeedCacheStorage;

mod feed_events;
pub use feed_events::PostgresFeedEventStorage;

mod password_resets;
pub use password_resets::PostgresPasswordResetStorage;

mod user_config;
pub use user_config::PostgresUserConfigStorage;

mod media;
pub use media::PostgresMediaStorage;

mod posts;
pub use posts::PostgresPostStorage;

mod subscriptions;
pub use subscriptions::PostgresSubscriptionStorage;

mod audiences;
pub use audiences::PostgresAudienceStorage;

mod bootstrap;
pub use bootstrap::{create_postgres_database_and_role, PgBootstrapError};

pub(crate) mod backup;

#[cfg(test)]
mod migrations;

#[cfg(test)]
mod schema;

#[cfg(test)]
mod teardown;

use crate::{AtomicOps, ConfirmPasswordResetError, RegisterWithInviteError};
use common::password::Password;
use common::username::Username;

// ---------------------------------------------------------------------------
// AtomicOps
// ---------------------------------------------------------------------------

pub struct PostgresAtomicOps {
    pool: PgPool,
}

impl PostgresAtomicOps {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AtomicOps for PostgresAtomicOps {
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&str>,
        is_operator: bool,
        invite_code: &str,
    ) -> Result<i64, RegisterWithInviteError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM invites WHERE code = $1",
        )
        .bind(invite_code)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RegisterWithInviteError::InviteNotFound)?;

        let (used_at, expires_at) = row;
        if used_at.is_some() {
            return Err(RegisterWithInviteError::InviteAlreadyUsed);
        }

        let now = Utc::now();
        if expires_at <= now {
            return Err(RegisterWithInviteError::InviteExpired);
        }

        let password_hash = crate::helpers::hash_password(password.clone())
            .await
            .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(e)))?; // cov:ignore

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING user_id",
        )
        .bind(username.as_str())
        .bind(&password_hash)
        .bind(display_name)
        .bind(now)
        .bind(is_operator)
        .fetch_one(&mut *tx)
        .await;

        let user_id = match result {
            Ok(id) => id,
            // Let the UNIQUE(username) constraint be the arbiter rather than a
            // pre-INSERT existence check: that closes the check-then-insert race
            // between concurrent registrations.
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                return Err(RegisterWithInviteError::UsernameTaken);
            }
            Err(error) => return Err(RegisterWithInviteError::Internal(error)),
        };

        sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
            .bind(now)
            .bind(user_id)
            .bind(invite_code)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(user_id)
    }

    async fn confirm_password_reset(
        &self,
        raw_token: &str,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| ConfirmPasswordResetError::NotFound)?;

        let mut tx = self.pool.begin().await?;
        let now = Utc::now();
        // Claim the token in one atomic UPDATE: it matches only when the token
        // exists, is unused, and is unexpired, so concurrent confirmations cannot
        // both win (ADR-0021). On a miss we re-read to classify the failure into a
        // precise NotFound / AlreadyUsed / Expired error.
        let claimed = sqlx::query_as::<_, (i64,)>(
            "UPDATE password_resets SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;

        let Some((user_id,)) = claimed else {
            let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
                "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
            )
            .bind(&token_hash)
            .fetch_optional(&mut *tx)
            .await?;

            tx.rollback().await.ok();
            return match row {
                None => Err(ConfirmPasswordResetError::NotFound),
                Some((Some(_), _)) => Err(ConfirmPasswordResetError::AlreadyUsed),
                Some((None, expires_at)) if expires_at <= now => {
                    Err(ConfirmPasswordResetError::Expired)
                }
                Some((None, _)) => Err(ConfirmPasswordResetError::Expired),
            };
        };

        // ADR-0022: hash only after the token claim succeeds, so a bogus/used/expired
        // token is rejected above without paying the Argon2 cost. A hash failure here
        // `?`-returns and drops the tx → rollback → the claim reverts (token not consumed).
        let password_hash = crate::helpers::hash_password(new_password.clone())
            .await
            .map_err(|e| ConfirmPasswordResetError::Internal(sqlx::Error::Io(e)))?;

        sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
            .bind(&password_hash)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Database open / connection
// ---------------------------------------------------------------------------

fn make_postgres_app_state(pool: PgPool) -> Arc<crate::AppState> {
    Arc::new(crate::AppState {
        site_config: Arc::new(PostgresSiteConfigStorage::new(pool.clone())),
        users: Arc::new(PostgresUserStorage::new(pool.clone())),
        sessions: Arc::new(PostgresSessionStorage::new(pool.clone())),
        invites: Arc::new(PostgresInviteStorage::new(pool.clone())),
        atomic: Arc::new(PostgresAtomicOps::new(pool.clone())),
        email_verifications: Arc::new(PostgresEmailVerificationStorage::new(pool.clone())),
        password_resets: Arc::new(PostgresPasswordResetStorage::new(pool.clone())),
        posts: Arc::new(PostgresPostStorage::new(pool.clone())),
        subscriptions: Arc::new(PostgresSubscriptionStorage::new(
            pool.clone(),
            Arc::new(common::visibility::OpenSubscriptionPolicy),
        )),
        audiences: Arc::new(PostgresAudienceStorage::new(pool.clone())),
        media: Arc::new(PostgresMediaStorage::new(pool.clone())),
        user_config: Arc::new(PostgresUserConfigStorage::new(pool.clone())),
        feed_cache: Arc::new(PostgresFeedCacheStorage::new(pool.clone())),
        feed_events: Arc::new(PostgresFeedEventStorage::new(pool)),
    })
}

fn postgres_password_from_env() -> io::Result<Option<String>> {
    if let Ok(path) = std::env::var("JAUNDER_DB_PASSWORD_FILE") {
        return std::fs::read_to_string(path).map(|s| Some(s.trim_end().to_owned()));
    }

    Ok(std::env::var("JAUNDER_DB_PASSWORD").ok())
}

/// Resolve final Postgres options, applying password overrides from env
/// and the slow-query log threshold.
///
/// # Errors
///
/// Returns `sqlx::Error::Io` if the password file env var points at an
/// unreadable file.
pub fn resolved_postgres_options(options: &PgConnectOptions) -> sqlx::Result<PgConnectOptions> {
    let mut options = options.clone();
    if let Some(password) = postgres_password_from_env().map_err(sqlx::Error::Io)? {
        options = options.password(&password);
    }
    options = options.log_slow_statements(LevelFilter::Warn, crate::db::sql_slow_query_threshold());
    Ok(options)
}

#[tracing::instrument(name = "storage.postgres.open_database", skip(options))]
pub(crate) async fn open_postgres_database_with_pool(
    options: &PgConnectOptions,
) -> sqlx::Result<(Arc<crate::AppState>, PgPool)> {
    let options = resolved_postgres_options(options)?;
    let pool = PgPool::connect_with(options).await?;
    sqlx::migrate!("./migrations/postgres").run(&pool).await?;
    Ok((make_postgres_app_state(pool.clone()), pool))
}

/// Opens the `PostgreSQL` database and returns just the [`AppState`]; the pool is
/// dropped. Tests that need to inject a pool fault use
/// [`open_postgres_database_with_pool`] via the `test_support` harness.
pub(crate) async fn open_postgres_database(
    options: &PgConnectOptions,
) -> sqlx::Result<Arc<crate::AppState>> {
    Ok(open_postgres_database_with_pool(options).await?.0)
}

/// Returns `true` if the `PostgreSQL` database holds no user data — every table
/// except the migration-seeded lookups is empty.
pub(crate) async fn database_is_empty(options: &PgConnectOptions) -> sqlx::Result<bool> {
    let options = resolved_postgres_options(options)?;
    let pool = PgPool::connect_with(options).await?;
    let tables = sqlx::query_scalar::<_, String>(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE' \
           AND table_name <> '_sqlx_migrations'",
    )
    .fetch_all(&pool)
    .await?;
    for table in tables {
        if crate::db::MIGRATION_SEEDED_TABLES.contains(&table.as_str()) {
            continue;
        }
        let has_row = sqlx::query_scalar::<_, bool>(&format!(
            "SELECT EXISTS(SELECT 1 FROM {} LIMIT 1)",
            crate::sql::quote_identifier(&table)
        ))
        .fetch_one(&pool)
        .await?;
        if has_row {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn postgres_password_prefers_file_over_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_DB_PASSWORD", "from-env");
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("db-password");
        std::fs::write(&path, "from-file\n").unwrap();
        std::env::set_var("JAUNDER_DB_PASSWORD_FILE", &path);

        let password = postgres_password_from_env().unwrap();

        std::env::remove_var("JAUNDER_DB_PASSWORD");
        std::env::remove_var("JAUNDER_DB_PASSWORD_FILE");
        assert_eq!(password.as_deref(), Some("from-file"));
    }

    #[test]
    fn postgres_password_uses_env_when_file_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("JAUNDER_DB_PASSWORD_FILE");
        std::env::set_var("JAUNDER_DB_PASSWORD", "from-env");

        let password = postgres_password_from_env().unwrap();

        std::env::remove_var("JAUNDER_DB_PASSWORD");
        assert_eq!(password.as_deref(), Some("from-env"));
    }

    #[test]
    fn postgres_password_returns_none_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("JAUNDER_DB_PASSWORD");
        std::env::remove_var("JAUNDER_DB_PASSWORD_FILE");

        let password = postgres_password_from_env().unwrap();

        assert_eq!(password, None);
    }

    #[test]
    fn resolved_postgres_options_applies_password_override_when_env_set() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("JAUNDER_DB_PASSWORD", "secret");
        std::env::remove_var("JAUNDER_DB_PASSWORD_FILE");

        let base: PgConnectOptions = "postgres://user@localhost/db".parse().unwrap();
        let resolved = resolved_postgres_options(&base);

        std::env::remove_var("JAUNDER_DB_PASSWORD");
        assert!(resolved.is_ok());
    }

    #[test]
    fn resolved_postgres_options_returns_io_error_when_password_file_unreadable() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("JAUNDER_DB_PASSWORD");
        std::env::set_var(
            "JAUNDER_DB_PASSWORD_FILE",
            "/nonexistent/path/to/db-password",
        );

        let base: PgConnectOptions = "postgres://user@localhost/db".parse().unwrap();
        let result = resolved_postgres_options(&base);

        std::env::remove_var("JAUNDER_DB_PASSWORD_FILE");
        assert!(matches!(result, Err(sqlx::Error::Io(_))));
    }
}
