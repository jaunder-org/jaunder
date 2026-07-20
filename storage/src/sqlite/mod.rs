use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use log::LevelFilter;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
    ConnectOptions, SqlitePool,
};

use async_trait::async_trait;

mod site_config;
pub use site_config::SqliteSiteConfigStorage;

mod users;
pub use users::SqliteUserStorage;

mod sessions;
pub use sessions::SqliteSessionStorage;

mod invites;
pub use invites::SqliteInviteStorage;

mod email_verifications;
pub use email_verifications::SqliteEmailVerificationStorage;

mod feed_cache;
pub use feed_cache::SqliteFeedCacheStorage;

mod feed_events;
pub use feed_events::SqliteFeedEventStorage;

mod password_resets;
pub use password_resets::SqlitePasswordResetStorage;

mod user_config;
pub use user_config::SqliteUserConfigStorage;

mod media;
pub use media::SqliteMediaStorage;

mod posts;
pub use posts::SqlitePostStorage;

mod subscriptions;
pub use subscriptions::SqliteSubscriptionStorage;

mod audiences;
pub use audiences::SqliteAudienceStorage;

pub(crate) mod backup;

#[cfg(test)]
mod pool;

use crate::db::sql_slow_query_threshold;
use crate::{AppState, AtomicOps, ConfirmPasswordResetError, RegisterWithInviteError};
use common::display_name::DisplayName;
use common::ids::UserId;
use common::password::Password;
use common::token::RawToken;
use common::username::Username;
use host::invite::InviteCode;

// ---------------------------------------------------------------------------
// Database helpers
// ---------------------------------------------------------------------------

fn make_sqlite_app_state(pool: SqlitePool) -> Arc<AppState> {
    Arc::new(AppState {
        site_config: Arc::new(SqliteSiteConfigStorage::new(pool.clone())),
        users: Arc::new(SqliteUserStorage::new(pool.clone())),
        sessions: Arc::new(SqliteSessionStorage::new(pool.clone())),
        invites: Arc::new(SqliteInviteStorage::new(pool.clone())),
        atomic: Arc::new(SqliteAtomicOps::new(pool.clone())),
        email_verifications: Arc::new(SqliteEmailVerificationStorage::new(pool.clone())),
        password_resets: Arc::new(SqlitePasswordResetStorage::new(pool.clone())),
        posts: Arc::new(SqlitePostStorage::new(pool.clone())),
        subscriptions: Arc::new(SqliteSubscriptionStorage::new(
            pool.clone(),
            Arc::new(common::visibility::OpenSubscriptionPolicy),
        )),
        audiences: Arc::new(SqliteAudienceStorage::new(pool.clone())),
        media: Arc::new(SqliteMediaStorage::new(pool.clone())),
        user_config: Arc::new(SqliteUserConfigStorage::new(pool.clone())),
        feed_cache: Arc::new(SqliteFeedCacheStorage::new(pool.clone())),
        feed_events: Arc::new(SqliteFeedEventStorage::new(pool)),
    })
}

#[tracing::instrument(
    name = "storage.sqlite.open_database",
    skip(options),
    fields(create_if_missing)
)]
pub(crate) async fn open_sqlite_database_with_pool(
    options: &SqliteConnectOptions,
    create_if_missing: bool,
) -> sqlx::Result<(Arc<AppState>, SqlitePool)> {
    let mut options = options.clone();
    if create_if_missing {
        options = options.create_if_missing(true);
    }
    // WAL mode allows concurrent readers while a writer is active, dramatically
    // reducing SQLITE_BUSY errors under load. The busy timeout lets SQLite retry
    // automatically instead of failing immediately when it cannot obtain a lock.
    options = options
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .log_slow_statements(LevelFilter::Warn, sql_slow_query_threshold());

    let pool = sqlx::SqlitePool::connect_with(options).await?;

    // Increase cache size to 32MB. SQLite page size is 4KB by default (usually),
    // so 32MB is 8192 pages. The `-32000` syntax tells SQLite 32MB.
    sqlx::query("PRAGMA cache_size = -32000")
        .execute(&pool)
        .await?;

    sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
    Ok((make_sqlite_app_state(pool.clone()), pool))
}

/// Opens (or creates) the `SQLite` database and returns just the [`AppState`];
/// the pool is dropped. Tests that need to inject a pool fault use
/// [`open_sqlite_database_with_pool`] via the `test_support` harness.
pub(super) async fn open_sqlite_database(
    options: &SqliteConnectOptions,
    create_if_missing: bool,
) -> sqlx::Result<Arc<AppState>> {
    Ok(open_sqlite_database_with_pool(options, create_if_missing)
        .await?
        .0)
}

/// Returns `true` if the `SQLite` database holds no user data — every table
/// except the migration-seeded lookups is empty.
pub(super) async fn database_is_empty(options: &SqliteConnectOptions) -> sqlx::Result<bool> {
    let pool = SqlitePool::connect_with(options.clone()).await?;
    let tables = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations'",
    )
    .fetch_all(&pool)
    .await?;
    for table in tables {
        if crate::db::MIGRATION_SEEDED_TABLES.contains(&table.as_str()) {
            continue;
        }
        let has_row = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT EXISTS(SELECT 1 FROM {} LIMIT 1)",
            crate::sql::quote_identifier(&table)
        ))
        .fetch_one(&pool)
        .await?
            != 0;
        if has_row {
            return Ok(false);
        }
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// AtomicOps
// ---------------------------------------------------------------------------

/// `SQLite` implementation of [`AtomicOps`].
///
/// Holds the pool directly so it can span multiple tables in a single
/// transaction without going through the individual storage trait objects.
pub struct SqliteAtomicOps {
    pool: SqlitePool,
}

impl SqliteAtomicOps {
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AtomicOps for SqliteAtomicOps {
    async fn create_user_with_invite(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&DisplayName>,
        is_operator: bool,
        invite_code: &InviteCode,
    ) -> Result<UserId, RegisterWithInviteError> {
        // ADR-0021: take the write lock up front with BEGIN IMMEDIATE rather than a
        // deferred BEGIN, so the SELECT->INSERT step performs no shared->reserved lock
        // upgrade (the SQLITE_BUSY-on-upgrade failure mode). sqlx's Transaction issues
        // its own deferred BEGIN, so drive the transaction manually on a raw connection,
        // mirroring sqlite/backup.rs.
        //
        // ADR-0022: the invite (a high-entropy secret) is validated *before* hashing, so
        // a bogus code is rejected without paying the Argon2 cost. The hash therefore runs
        // inside the immediate transaction on the success path only.
        let mut conn = self.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<UserId, RegisterWithInviteError> = async {
            // Read the invite's state first so the three failures stay distinct: no row ->
            // InviteNotFound, used_at set -> InviteAlreadyUsed, past expires_at -> InviteExpired.
            // These checks deliberately are NOT folded into the write (e.g. a single
            // `UPDATE ... WHERE used_at IS NULL AND expires_at > now` claim): that would collapse
            // all three into one indistinguishable "zero rows affected" outcome and lose the
            // specific error the caller needs. Reporting them distinctly is what keeps this a
            // read-then-write transaction (hence BEGIN IMMEDIATE above), not a single-statement
            // claim.
            let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
                "SELECT used_at, expires_at FROM invites WHERE code = $1",
            )
            .bind(invite_code)
            .fetch_optional(&mut *conn)
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
                .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(e)))?;

            let insert = sqlx::query_scalar::<_, i64>(
                "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
                 VALUES ($1, $2, $3, $4, $5)
                 RETURNING user_id",
            )
            .bind(username)
            .bind(&password_hash)
            .bind(display_name)
            .bind(now)
            .bind(is_operator)
            .fetch_one(&mut *conn)
            .await;

            let user_id = match insert {
                Ok(id) => UserId::from(id),
                Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                    return Err(RegisterWithInviteError::UsernameTaken);
                }
                Err(error) => return Err(RegisterWithInviteError::Internal(error)),
            };

            sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
                .bind(now)
                .bind(i64::from(user_id))
                .bind(invite_code)
                .execute(&mut *conn)
                .await?;

            Ok(user_id)
        }
        .await;

        match result {
            Ok(user_id) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(user_id)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    async fn confirm_password_reset(
        &self,
        raw_token: &RawToken,
        new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError> {
        let token_hash =
            host::token::hash(raw_token).map_err(|_| ConfirmPasswordResetError::NotFound)?;

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
                Some((None, _)) => Err(ConfirmPasswordResetError::Expired),
            };
        };
        let user_id = UserId::from(user_id);

        // ADR-0022: hash only after the token claim succeeds, so a bogus/used/expired
        // token is rejected above without paying the Argon2 cost. A hash failure here
        // `?`-returns and drops the tx → rollback → the claim reverts (token not consumed).
        let password_hash = crate::helpers::hash_password(new_password.clone())
            .await
            .map_err(|e| ConfirmPasswordResetError::Internal(sqlx::Error::Io(e)))?;

        sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
            .bind(&password_hash)
            .bind(i64::from(user_id))
            .execute(&mut *tx)
            .await?;

        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(i64::from(user_id))
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        Ok(())
    }
}
