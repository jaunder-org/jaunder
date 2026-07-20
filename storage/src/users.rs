//! User account and profile storage.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Database, Pool};
use thiserror::Error;
use tracing::Instrument;

use crate::backend::Backend;
use common::display_name::DisplayName;
use common::email::Email;
use common::ids::UserId;
use common::password::Password;
use common::username::Username;

use crate::helpers::{user_record_from_row, UserRow};

/// A user account record returned by [`UserStorage`] queries.
///
/// Does not expose `password_hash`; that field is only accessed inside the
/// storage implementation to ensure it is never accidentally leaked to
/// higher-level application logic.
#[derive(Clone, Debug)]
pub struct UserRecord {
    /// Unique internal identifier.
    pub user_id: UserId,
    /// Unique username (canonicalized).
    pub username: Username,
    /// User's preferred display name.
    pub display_name: Option<DisplayName>,
    /// Optional short biography.
    pub bio: Option<String>,
    /// When the account was created.
    pub created_at: DateTime<Utc>,
    /// When the user last successfully authenticated.
    pub last_authenticated_at: Option<DateTime<Utc>>,
    /// User's verified or pending email address.
    pub email: Option<Email>,
    /// Whether the email address has been verified.
    pub email_verified: bool,
    /// Whether the user has site-wide administrative privileges.
    pub is_operator: bool,
}

/// Errors that can occur when creating a user.
#[derive(Debug, Error)]
pub enum CreateUserError {
    /// The requested username is already in use by another account.
    #[error("username is already taken")]
    UsernameTaken,
    /// An unexpected database error occurred.
    #[error(transparent)]
    Internal(#[from] sqlx::Error),
}

/// Errors that can occur when authenticating a user by password.
#[derive(Debug, Error)]
pub enum UserAuthError {
    /// The username or password was incorrect.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// An unexpected error occurred during the authentication process.
    ///
    /// Carries the underlying error as a typed source (a `sqlx::Error` from the
    /// DB lookup/update, an `io::Error` from password verification, or a record
    /// conversion error) rather than a flattened string, so the boundary can
    /// downcast for classification.
    #[error("internal error: {0}")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<CreateUserError> for host::error::InternalError {
    /// Reproduces the former `web::auth::server::register_open_error`
    /// `(kind, class, public_message)`: a taken username is a client conflict,
    /// anything else is a masked storage failure.
    fn from(error: CreateUserError) -> Self {
        use host::error::InternalError;
        match error {
            CreateUserError::UsernameTaken => InternalError::conflict("username is already taken"),
            CreateUserError::Internal(e) => InternalError::storage(e),
        }
    }
}

impl From<UserAuthError> for host::error::InternalError {
    /// Reproduces the former `web::auth::server::login_error`
    /// `(kind, class, public_message)`: bad credentials are an unauthorized
    /// client error, an internal failure is a masked server error preserving the
    /// boxed typed cause chain for operator logs (not flattened to a string).
    fn from(error: UserAuthError) -> Self {
        use host::error::InternalError;
        match error {
            UserAuthError::InvalidCredentials => InternalError::unauthorized("invalid credentials"),
            UserAuthError::Internal(source) => InternalError::server_boxed(source),
        }
    }
}

/// Maps an authentication failure to its bounded `outcome` attribute for the
/// `jaunder.auth.logins` metric. Exhaustively tested so every variant's mapping
/// is covered independent of which failures the login path is exercised with.
#[must_use]
pub fn login_outcome(error: &UserAuthError) -> host::metrics::LoginOutcome {
    match error {
        UserAuthError::InvalidCredentials => host::metrics::LoginOutcome::InvalidCredentials,
        UserAuthError::Internal(_) => host::metrics::LoginOutcome::InternalError,
    }
}

/// Fields to update on a user's profile.
///
/// `None` clears the field, `Some(v)` sets it. `display_name` is a validated
/// [`DisplayName`] (the invariant is held at the boundary); `bio` is free-form.
pub struct ProfileUpdate<'a> {
    /// New display name, or `None` to clear.
    pub display_name: Option<&'a DisplayName>,
    /// New bio text, or `None` to clear.
    pub bio: Option<&'a str>,
}

/// Async operations on the `users` table.
///
/// This trait defines the core interface for managing user accounts, including
/// creation, authentication, and profile management.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait UserStorage: Send + Sync {
    /// Creates a new user account.
    ///
    /// The password will be hashed using a cryptographically secure algorithm
    /// (e.g., bcrypt) before being stored.
    ///
    /// # Errors
    ///
    /// Returns [`CreateUserError::UsernameTaken`] if the username exists, or
    /// [`CreateUserError::Internal`] on database failure.
    // Explicit `'a` for `mockall::automock` — see
    // `PostStorage::list_published_by_user`.
    async fn create_user<'a>(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&'a DisplayName>,
        is_operator: bool,
    ) -> Result<UserId, CreateUserError>;

    /// Authenticates a user by username and password.
    ///
    /// On success, updates `last_authenticated_at` for the user.
    ///
    /// # Errors
    ///
    /// Returns [`UserAuthError::InvalidCredentials`] if the credentials don't match,
    /// or [`UserAuthError::Internal`] on unexpected failures.
    async fn authenticate(
        &self,
        username: &Username,
        password: &Password,
    ) -> Result<UserRecord, UserAuthError>;

    /// Fetches a user record by its internal ID.
    async fn get_user(&self, user_id: UserId) -> sqlx::Result<Option<UserRecord>>;

    /// Fetches a user record by their username.
    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>>;

    /// Updates the display name and/or bio for a user.
    // Explicit `'a` for `mockall::automock` — see
    // `PostStorage::list_published_by_user`.
    async fn update_profile<'a>(
        &self,
        user_id: UserId,
        update: &ProfileUpdate<'a>,
    ) -> sqlx::Result<()>;

    /// Sets or clears a user's email address and verification status.
    // Explicit `'a` for `mockall::automock` — see
    // `PostStorage::list_published_by_user`.
    async fn set_email<'a>(
        &self,
        user_id: UserId,
        email: Option<&'a Email>,
        verified: bool,
    ) -> sqlx::Result<()>;

    /// Replaces the stored password hash for `user_id` with a hash of `new_password`.
    ///
    /// This is typically used during password resets. Hashing is performed
    /// asynchronously on a blocking thread.
    async fn set_password(&self, user_id: UserId, new_password: &Password) -> sqlx::Result<()>;
}

/// Generic [`UserStorage`] backed by any [`Backend`] database.
///
/// Zero backend divergence (shared SQL across `SQLite` and Postgres), so it is
/// implemented once here; see ADR-0019.
pub struct UserStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> UserStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB> UserStorage for UserStore<DB>
where
    DB: Backend,
    UserRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    (
        i64,
        Username,
        Option<DisplayName>,
        Option<String>,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
        String,
        Option<Email>,
        bool,
        bool,
    ): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'r> i64: sqlx::Decode<'r, DB> + sqlx::Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `Username`/`DisplayName`/`Email` bind/decode as themselves via the sqlx
    // bridge (#438), which delegates to `String`; these bounds make that bridge
    // available on the generic backend (the `String` pair covers the by-value
    // newtype impls; the `Option<&…>` pairs cover the nullable profile binds,
    // mirroring the `Option<&str>` bound the old `&**`-deref binds required).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> Option<&'q DisplayName>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q Email>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> bool: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q str>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        name = "storage.user.create_user",
        skip(self, password, display_name),
        fields(username = %username, db.system = DB::DB_SYSTEM)
    )]
    async fn create_user<'a>(
        &self,
        username: &Username,
        password: &Password,
        display_name: Option<&'a DisplayName>,
        is_operator: bool,
    ) -> Result<UserId, CreateUserError> {
        let password_hash = crate::helpers::hash_password(password.clone())
            .instrument(tracing::info_span!(
                "storage.user.create_user.hash_password",
                db.system = DB::DB_SYSTEM
            ))
            .await
            .map_err(|e| CreateUserError::Internal(sqlx::Error::Io(e)))?;

        let now = Utc::now();

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING user_id",
        )
        .bind(username)
        .bind(password_hash.as_str())
        .bind(display_name)
        .bind(now)
        .bind(is_operator)
        .fetch_one(&self.pool)
        .instrument(tracing::info_span!(
            "storage.user.create_user.insert_user_row",
            db.system = DB::DB_SYSTEM
        ))
        .await;

        match result {
            Ok(id) => Ok(UserId::from(id)),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(CreateUserError::UsernameTaken)
            }
            Err(error) => Err(CreateUserError::Internal(error)),
        }
    }

    #[tracing::instrument(
        name = "storage.user.authenticate",
        skip(self, password),
        fields(username = %username, db.system = DB::DB_SYSTEM)
    )]
    async fn authenticate(
        &self,
        username: &Username,
        password: &Password,
    ) -> Result<UserRecord, UserAuthError> {
        let row = sqlx::query_as::<
            _,
            (
                i64,
                Username,
                Option<DisplayName>,
                Option<String>,
                DateTime<Utc>,
                Option<DateTime<Utc>>,
                String,
                Option<Email>,
                bool,
                bool,
            ),
        >(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    password_hash, email, email_verified, is_operator
             FROM users WHERE username = $1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .instrument(tracing::info_span!(
            "storage.user.authenticate.lookup_user",
            db.system = DB::DB_SYSTEM
        ))
        .await
        .map_err(|e| UserAuthError::Internal(Box::new(e)))?;

        let Some((
            user_id,
            username,
            display_name,
            bio,
            created_at,
            _last_authenticated_at,
            hash,
            email,
            email_verified,
            is_operator,
        )) = row
        else {
            // Equalize timing with the present-user path to avoid a username
            // enumeration oracle (§2.1): perform a dummy Argon2 verification
            // before rejecting. The result is intentionally discarded.
            let _ = crate::helpers::verify_password(
                password.clone(),
                crate::helpers::dummy_password_hash().to_string(),
            )
            .await;
            return Err(UserAuthError::InvalidCredentials);
        };

        let valid = crate::helpers::verify_password(password.clone(), hash)
            .instrument(tracing::info_span!(
                "storage.user.authenticate.verify_password",
                db.system = DB::DB_SYSTEM
            ))
            .await
            .map_err(|e| UserAuthError::Internal(Box::new(e)))?;

        if !valid {
            return Err(UserAuthError::InvalidCredentials);
        }

        let now = Utc::now();

        sqlx::query("UPDATE users SET last_authenticated_at = $1 WHERE user_id = $2")
            .bind(now)
            .bind(user_id)
            .execute(&self.pool)
            .instrument(tracing::info_span!(
                "storage.user.authenticate.update_last_authenticated_at",
                db.system = DB::DB_SYSTEM
            ))
            .await
            .map_err(|e| UserAuthError::Internal(Box::new(e)))?;

        Ok(crate::helpers::build_user_record((
            user_id,
            username,
            display_name,
            bio,
            created_at,
            Some(now),
            email,
            email_verified,
            is_operator,
        )))
    }

    async fn get_user(&self, user_id: UserId) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    email, email_verified, is_operator
             FROM users WHERE user_id = $1",
        )
        .bind(i64::from(user_id))
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row))
    }

    async fn get_user_by_username(&self, username: &Username) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    email, email_verified, is_operator
             FROM users WHERE username = $1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(user_record_from_row))
    }

    async fn update_profile<'a>(
        &self,
        user_id: UserId,
        update: &ProfileUpdate<'a>,
    ) -> sqlx::Result<()> {
        sqlx::query("UPDATE users SET display_name = $1, bio = $2 WHERE user_id = $3")
            .bind(update.display_name)
            .bind(update.bio)
            .bind(i64::from(user_id))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_email<'a>(
        &self,
        user_id: UserId,
        email: Option<&'a Email>,
        verified: bool,
    ) -> sqlx::Result<()> {
        sqlx::query("UPDATE users SET email = $1, email_verified = $2 WHERE user_id = $3")
            .bind(email)
            .bind(verified)
            .bind(i64::from(user_id))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_password(&self, user_id: UserId, new_password: &Password) -> sqlx::Result<()> {
        let password_hash = crate::helpers::hash_password(new_password.clone())
            .await
            .map_err(sqlx::Error::Io)?;

        sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
            .bind(password_hash.as_str())
            .bind(i64::from(user_id))
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, seed_user, Backend, CloseablePool};
    use common::test_support::{parse_display_name, parse_email};
    use rstest::*;
    use rstest_reuse::*;

    #[apply(backends)]
    #[tokio::test]
    async fn user_round_trips_username_display_name_and_email(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;

        // `create_user` binds a typed `Username` + `Option<&DisplayName>`, and
        // `set_email` binds `Option<&Email>`; the reads decode each column straight
        // back into its newtype — exercising both bridge directions.
        let username: Username = "alice".parse().unwrap();
        let display_name = parse_display_name("Alice Example");
        let user_id = env
            .state
            .users
            .create_user(
                &username,
                &"password123".parse().unwrap(),
                Some(&display_name),
                false,
            )
            .await
            .unwrap();

        let email = parse_email("alice@example.com");
        env.state
            .users
            .set_email(user_id, Some(&email), true)
            .await
            .unwrap();

        let record = env.state.users.get_user(user_id).await.unwrap().unwrap();
        assert_eq!(record.username, username);
        assert_eq!(record.display_name, Some(display_name));
        assert_eq!(record.email, Some(email));

        // `get_user_by_username` binds the `Username` and decodes the same columns
        // via a second query.
        let by_name = env
            .state
            .users
            .get_user_by_username(&username)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_name.username, username);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn user_round_trips_absent_display_name_and_email(#[case] backend: Backend) {
        // The `None` decode path for `Option<DisplayName>` / `Option<Email>`.
        let env = backend.setup().await;
        let username: Username = "bob".parse().unwrap();
        let user_id = env
            .state
            .users
            .create_user(&username, &"password123".parse().unwrap(), None, false)
            .await
            .unwrap();

        let record = env.state.users.get_user(user_id).await.unwrap().unwrap();
        assert_eq!(record.username, username);
        assert_eq!(record.display_name, None);
        assert_eq!(record.email, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_user_rejects_a_malformed_username_column(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = seed_user(&env.state).await;

        // Overwrite the `username` column with a value `Username::from_str`
        // rejects (a space is not a valid username character), binding it as a raw
        // `&str` so the bad value actually lands in the column — the typed bind
        // could not produce it.
        let sql = "UPDATE users SET username = $1 WHERE user_id = $2";
        match env.base.pool() {
            CloseablePool::Sqlite(pool) => {
                sqlx::query(sql)
                    .bind("bad name")
                    .bind(i64::from(user_id))
                    .execute(pool)
                    .await
                    .unwrap();
            }
            CloseablePool::Postgres(pool) => {
                sqlx::query(sql)
                    .bind("bad name")
                    .bind(i64::from(user_id))
                    .execute(pool)
                    .await
                    .unwrap();
            }
        }

        // The read decodes the `username` column into `Username` via the sqlx
        // bridge, which validates through `FromStr`; the malformed value surfaces
        // as a column-decode error rather than being silently admitted (covers the
        // bridge's `Decode` error arm).
        let err = env.state.users.get_user(user_id).await.unwrap_err();
        assert!(
            matches!(err, sqlx::Error::ColumnDecode { .. }),
            "expected a column-decode error, got: {err:?}"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_closed_pool_returns_internal_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base.close_pool().await;
        let result = env
            .state
            .users
            .authenticate(&"alice".parse().unwrap(), &"password123".parse().unwrap())
            .await;
        // §3.1a: the underlying sqlx::Error is preserved as a typed source
        // (not stringified), so the boundary can classify it.
        assert!(
            matches!(
                result,
                Err(UserAuthError::Internal(ref source))
                    if source.downcast_ref::<sqlx::Error>().is_some()
            ),
            "expected Internal carrying a sqlx::Error source"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_corrupted_hash_returns_internal_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        seed_user(&env.state).await;
        env.base
            .pool()
            .execute("UPDATE users SET password_hash='not-a-bcrypt-hash' WHERE username='testuser'")
            .await
            .unwrap();
        let result = env
            .state
            .users
            .authenticate(
                &"testuser".parse().unwrap(),
                &"password123".parse().unwrap(),
            )
            .await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_invalid_email_in_db_returns_internal_error(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        seed_user(&env.state).await;
        env.base
            .pool()
            .execute("UPDATE users SET email='not-an-email' WHERE username='testuser'")
            .await
            .unwrap();
        let result = env
            .state
            .users
            .authenticate(
                &"testuser".parse().unwrap(),
                &"password123".parse().unwrap(),
            )
            .await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_overlong_display_name_in_db_returns_internal_error(
        #[case] backend: Backend,
    ) {
        // A pre-existing row whose display_name exceeds the DisplayName length
        // bound (the column was unbounded before #401) must surface as a typed
        // Internal error at the strict read boundary — never a panic. Mirrors the
        // invalid-email-in-db case above.
        let env = backend.setup().await;
        seed_user(&env.state).await;
        let overlong = "a".repeat(common::display_name::MAX_DISPLAY_NAME_CHARS + 1);
        let sql = format!("UPDATE users SET display_name='{overlong}' WHERE username='testuser'");
        env.base.pool().execute(sql.as_str()).await.unwrap();
        let result = env
            .state
            .users
            .authenticate(
                &"testuser".parse().unwrap(),
                &"password123".parse().unwrap(),
            )
            .await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_blocked_update_returns_internal_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        seed_user(&env.state).await;
        // Block the `last_authenticated_at` UPDATE the successful-auth path runs,
        // so authentication fails with `Internal` after the password verifies.
        match backend {
            Backend::Sqlite => {
                env.base
                    .pool()
                    .execute(
                        "CREATE TRIGGER block_auth_update \
                         BEFORE UPDATE OF last_authenticated_at ON users \
                         BEGIN SELECT RAISE(FAIL, 'blocked'); END",
                    )
                    .await
                    .unwrap();
            }
            Backend::Postgres => {
                env.base
                    .pool()
                    .execute(
                        "CREATE FUNCTION block_auth() RETURNS trigger AS $$ \
                         BEGIN RAISE EXCEPTION 'blocked'; END; $$ LANGUAGE plpgsql",
                    )
                    .await
                    .unwrap();
                env.base
                    .pool()
                    .execute(
                        "CREATE TRIGGER block_auth_update \
                         BEFORE UPDATE OF last_authenticated_at ON users \
                         FOR EACH ROW EXECUTE FUNCTION block_auth()",
                    )
                    .await
                    .unwrap();
            }
        }
        let result = env
            .state
            .users
            .authenticate(
                &"testuser".parse().unwrap(),
                &"password123".parse().unwrap(),
            )
            .await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn create_user_with_hash_error_returns_internal_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        let result = env
            .state
            .users
            .create_user(
                &"alice".parse().unwrap(),
                &"force-hash-error-for-test-coverage".parse().unwrap(),
                None,
                false,
            )
            .await;
        assert!(matches!(result, Err(CreateUserError::Internal(_))));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_verify_error_returns_internal_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.state
            .users
            .create_user(
                &"alice".parse().unwrap(),
                &"password123".parse().unwrap(),
                None,
                false,
            )
            .await
            .unwrap();
        let result = env
            .state
            .users
            .authenticate(
                &"alice".parse().unwrap(),
                &"force-verify-error-for-test-coverage".parse().unwrap(),
            )
            .await;
        assert!(matches!(result, Err(UserAuthError::Internal(_))));
    }

    // Behavior-preserving translation of the former `web` `register_open_error`
    // test: variants map to the same `(kind, public_message)`.
    #[test]
    fn from_create_user_error_maps_variants() {
        use host::error::{ErrorKind, InternalError};

        let taken: InternalError = CreateUserError::UsernameTaken.into();
        assert_eq!(taken.kind(), ErrorKind::Conflict);
        assert_eq!(taken.public_message(), "username is already taken");

        let internal: InternalError = CreateUserError::Internal(sqlx::Error::PoolClosed).into();
        assert_eq!(internal.kind(), ErrorKind::Storage);
        assert_eq!(internal.public_message(), "storage operation failed");
    }

    // Behavior-preserving translation of the former `web` `login_error` test,
    // including that the boxed cause chain is preserved (not flattened).
    #[test]
    fn from_user_auth_error_maps_variants() {
        use host::error::{ErrorKind, InternalError};
        use std::fmt;

        // A two-level source chain proves the mapping preserves the structured
        // cause chain rather than flattening it to the top error's string.
        #[derive(Debug)]
        struct Inner;
        impl fmt::Display for Inner {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "inner cause")
            }
        }
        impl std::error::Error for Inner {}

        #[derive(Debug)]
        struct Outer(Inner);
        impl fmt::Display for Outer {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "outer failure")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }

        let invalid: InternalError = UserAuthError::InvalidCredentials.into();
        assert_eq!(invalid.kind(), ErrorKind::Auth);
        // The unauthorized wire variant carries no message.
        assert_eq!(invalid.public_message(), "");

        let internal: InternalError = UserAuthError::Internal(Box::new(Outer(Inner))).into();
        assert_eq!(internal.kind(), ErrorKind::Internal);
        assert_eq!(internal.public_message(), "server operation failed");
        let op = internal.operator_message();
        assert!(op.contains("outer failure"), "operator message: {op}");
        assert!(op.contains("inner cause"), "operator message: {op}");
    }

    #[test]
    fn login_outcome_maps_each_variant() {
        use host::metrics::LoginOutcome;
        assert!(matches!(
            login_outcome(&UserAuthError::InvalidCredentials),
            LoginOutcome::InvalidCredentials
        ));
        assert!(matches!(
            login_outcome(&UserAuthError::Internal(Box::new(std::fmt::Error))),
            LoginOutcome::InternalError
        ));
    }
}
