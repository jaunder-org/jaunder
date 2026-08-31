//! User account and profile storage.

use crate::WriteTransaction;
use async_trait::async_trait;

use sqlx::{Database, Pool};
use thiserror::Error;
use tracing::Instrument;

use crate::backend::Backend;
use common::bio::Bio;
use common::display_name::DisplayName;
use common::email::Email;
use common::ids::UserId;
use common::time::UtcInstant;
use common::username::Username;
use host::password::Password;
use host::stored_password_hash::StoredPasswordHash;

use crate::helpers::{self, EmailVerified, OperatorStatus, UserRow};

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
    pub bio: Option<Bio>,
    /// When the account was created.
    pub created_at: UtcInstant,
    /// When the user last successfully authenticated.
    pub last_authenticated_at: Option<UtcInstant>,
    /// User's verified or pending email address.
    pub email: Option<Email>,
    /// Whether the email address has been verified.
    pub email_verified: bool,
    /// Whether the user has site-wide administrative privileges.
    pub is_operator: bool,
}
/// An Argon2-hashed password ready for a capability-guarded user mutation.
///
/// Construct this with [`prepare_password`] before acquiring a [`WriteTransaction`].
/// Invite and password-reset flows may construct it after their high-entropy
/// capability claim.
pub struct PreparedPassword(StoredPasswordHash);

/// A successfully verified login ready to record its authentication timestamp.
///
/// Password verification and the account lookup happen before the write scope;
/// consuming this value requires the sealed write capability.
#[derive(Debug)]
pub struct PreparedAuthentication(UserRecord);

/// Hashes a password without acquiring a write transaction.
///
/// # Errors
///
/// Returns an I/O error if the blocking hashing task fails or Argon2 rejects
/// the password.
#[tracing::instrument(name = "storage.user.prepare_password", skip(password))]
pub async fn prepare_password(password: Password) -> std::io::Result<PreparedPassword> {
    helpers::hash_password(password).await.map(PreparedPassword)
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
/// [`DisplayName`] and `bio` a validated [`Bio`] (the invariants are held at the
/// boundary).
pub struct ProfileUpdate<'a> {
    /// New display name, or `None` to clear.
    pub display_name: Option<&'a DisplayName>,
    /// New bio, or `None` to clear.
    pub bio: Option<&'a Bio>,
}

/// Async operations on the `users` table.
///
/// This trait defines the core interface for managing user accounts, including
/// creation, authentication, and profile management.
#[cfg_attr(feature = "test-utils", mockall::automock)]
#[async_trait]
pub trait UserStorage: Send + Sync {
    /// Inserts a user whose password has already been prepared.
    ///
    /// Ordinary callers construct `password` before acquiring the write scope.
    /// Capability-claiming flows may prepare it after their claim.
    async fn create_user<'a>(
        &self,
        transaction: &mut WriteTransaction,
        username: &Username,
        password: &PreparedPassword,
        display_name: Option<&'a DisplayName>,
        is_operator: bool,
    ) -> Result<UserId, CreateUserError>;

    /// Looks up and verifies login credentials without acquiring a write transaction.
    async fn prepare_authentication(
        &self,
        username: &Username,
        password: &Password,
    ) -> Result<PreparedAuthentication, UserAuthError>;

    /// Records a previously verified login.
    async fn authenticate(
        &self,
        transaction: &mut WriteTransaction,
        authentication: PreparedAuthentication,
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
        transaction: &mut WriteTransaction,
        user_id: UserId,
        update: &ProfileUpdate<'a>,
    ) -> sqlx::Result<()>;

    /// Sets or clears a user's email address and verification status.
    // Explicit `'a` for `mockall::automock` — see
    // `PostStorage::list_published_by_user`.
    async fn set_email<'a>(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        email: Option<&'a Email>,
        verified: bool,
    ) -> sqlx::Result<()>;

    /// Replaces the stored password hash for `user_id` with a prepared hash.
    ///
    /// Callers prepare ordinary password changes before acquiring their write
    /// scope. Password-reset flows prepare only after claiming the reset code.
    async fn set_password(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        password: &PreparedPassword,
    ) -> sqlx::Result<()>;
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

    pub(crate) async fn prepare_authentication_with(
        &self,
        username: &Username,
        password: &Password,
        verify_operation: helpers::VerifyPasswordOperation,
    ) -> Result<PreparedAuthentication, UserAuthError>
    where
        DB: Backend,
        (
            UserId,
            Username,
            Option<DisplayName>,
            Option<Bio>,
            UtcInstant,
            Option<UtcInstant>,
            StoredPasswordHash,
            Option<Email>,
            EmailVerified,
            OperatorStatus,
        ): for<'r> sqlx::FromRow<'r, DB::Row>,
        for<'r> UserId: sqlx::Decode<'r, DB> + sqlx::Type<DB>,
        usize: sqlx::ColumnIndex<DB::Row>,
        for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
        String: sqlx::Type<DB>,
        for<'q> String: sqlx::Encode<'q, DB>,
        for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
        for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
        for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
        for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    {
        let row = sqlx::query_as::<
            _,
            (
                UserId,
                Username,
                Option<DisplayName>,
                Option<Bio>,
                UtcInstant,
                Option<UtcInstant>,
                StoredPasswordHash,
                Option<Email>,
                EmailVerified,
                OperatorStatus,
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
            if let Err(error) = helpers::verify_password_with(
                password.clone(),
                helpers::dummy_password_hash().clone(),
                verify_operation,
            )
            .await
            {
                host::error::report_swallowed(
                    host::error::ErrorKind::Internal,
                    host::error::ErrorClass::Bug,
                    "storage.user.authenticate.dummy_verify",
                    host::error::SwallowedSource::Error(&error),
                );
            }
            return Err(UserAuthError::InvalidCredentials);
        };

        let valid = helpers::verify_password_with(password.clone(), hash, verify_operation)
            .instrument(tracing::info_span!(
                "storage.user.authenticate.verify_password",
                db.system = DB::DB_SYSTEM
            ))
            .await
            .map_err(|e| UserAuthError::Internal(Box::new(e)))?;

        if !valid {
            return Err(UserAuthError::InvalidCredentials);
        }

        Ok(PreparedAuthentication(helpers::build_user_record(
            helpers::UserRecordParts {
                user_id,
                username,
                display_name,
                bio,
                created_at,
                last_authenticated_at: None,
                email,
                email_verified: email_verified.value(),
                is_operator: is_operator.value(),
            },
        )))
    }
}

#[async_trait]
impl<DB> UserStorage for UserStore<DB>
where
    DB: Backend,
    UserRow: for<'r> sqlx::FromRow<'r, DB::Row>,
    (
        UserId,
        Username,
        Option<DisplayName>,
        Option<Bio>,
        UtcInstant,
        Option<UtcInstant>,
        StoredPasswordHash,
        Option<Email>,
        EmailVerified,
        OperatorStatus,
    ): for<'r> sqlx::FromRow<'r, DB::Row>,
    // `create_user`'s `RETURNING user_id` decodes straight into `UserId` via the
    // ADR-0071 bridge (#686), so the id never exists as a bare `i64` here (#715).
    for<'r> UserId: sqlx::Decode<'r, DB> + sqlx::Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    // Not residue: the ADR-0071 bridge *delegates* to `i64`, so this pair is what
    // makes every id newtype bind on a generic backend.
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    // `Username`/`DisplayName`/`Bio`/`Email` bind/decode as themselves via the
    // ADR-0071 sqlx bridge (the `String` pair covers the by-value newtype impls;
    // the `Option<&…>` pairs cover the nullable profile binds).
    String: sqlx::Type<DB>,
    for<'q> String: sqlx::Encode<'q, DB>,
    for<'q> Option<&'q DisplayName>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q Bio>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<&'q Email>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> bool: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> UtcInstant: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'c> &'c Pool<DB>: sqlx::Executor<'c, Database = DB>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    #[tracing::instrument(
        skip(self, transaction, password, display_name),
        fields(username = %username, db.system = DB::DB_SYSTEM)
    )]
    async fn create_user<'a>(
        &self,
        transaction: &mut WriteTransaction,
        username: &Username,
        password: &PreparedPassword,
        display_name: Option<&'a DisplayName>,
        is_operator: bool,
    ) -> Result<UserId, CreateUserError> {
        let now = UtcInstant::now();
        let connection = DB::write_connection(transaction).map_err(CreateUserError::Internal)?;

        let result = sqlx::query_scalar::<_, UserId>(
            "INSERT INTO users (username, password_hash, display_name, created_at, is_operator)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING user_id",
        )
        .bind(username)
        .bind(&password.0)
        .bind(display_name)
        .bind(now)
        // sqlx-newtype-bind:allow permanent-primitive — boolean operator flag has no domain identity.
        .bind(is_operator)
        .fetch_one(&mut *connection)
        .instrument(tracing::info_span!(
            "storage.user.create_user.insert_user_row",
            db.system = DB::DB_SYSTEM
        ))
        .await;

        match result {
            Ok(id) => Ok(id),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(CreateUserError::UsernameTaken)
            }
            Err(error) => Err(CreateUserError::Internal(error)),
        }
    }

    async fn prepare_authentication(
        &self,
        username: &Username,
        password: &Password,
    ) -> Result<PreparedAuthentication, UserAuthError> {
        self.prepare_authentication_with(username, password, host::password::verify)
            .await
    }

    #[tracing::instrument(
        name = "storage.user.authenticate",
        skip(self, transaction, authentication),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn authenticate(
        &self,
        transaction: &mut WriteTransaction,
        mut authentication: PreparedAuthentication,
    ) -> Result<UserRecord, UserAuthError> {
        let now = UtcInstant::now();
        let connection = DB::write_connection(transaction)
            .map_err(|error| UserAuthError::Internal(Box::new(error)))?;
        sqlx::query("UPDATE users SET last_authenticated_at = $1 WHERE user_id = $2")
            .bind(now)
            .bind(authentication.0.user_id)
            .execute(&mut *connection)
            .instrument(tracing::info_span!(
                "storage.user.authenticate.update_last_authenticated_at",
                db.system = DB::DB_SYSTEM
            ))
            .await
            .map_err(|error| UserAuthError::Internal(Box::new(error)))?;
        authentication.0.last_authenticated_at = Some(now);
        Ok(authentication.0)
    }

    async fn get_user(&self, user_id: UserId) -> sqlx::Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT user_id, username, display_name, bio, created_at, last_authenticated_at,
                    email, email_verified, is_operator
             FROM users WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(helpers::user_record_from_row))
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
        Ok(row.map(helpers::user_record_from_row))
    }

    async fn update_profile<'a>(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        update: &ProfileUpdate<'a>,
    ) -> sqlx::Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("UPDATE users SET display_name = $1, bio = $2 WHERE user_id = $3")
            .bind(update.display_name)
            .bind(update.bio)
            .bind(user_id)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    async fn set_email<'a>(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        email: Option<&'a Email>,
        verified: bool,
    ) -> sqlx::Result<()> {
        let connection = DB::write_connection(transaction)?;
        sqlx::query("UPDATE users SET email = $1, email_verified = $2 WHERE user_id = $3")
            .bind(email)
            // sqlx-newtype-bind:allow permanent-primitive — email verification is a boolean storage fact with no domain identity.
            .bind(verified)
            .bind(user_id)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    async fn set_password(
        &self,
        transaction: &mut WriteTransaction,
        user_id: UserId,
        password: &PreparedPassword,
    ) -> sqlx::Result<()> {
        let connection = DB::write_connection(transaction)?;

        sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
            .bind(&password.0)
            .bind(user_id)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, SeedUser, backends};
    use common::test_support::{parse_bio, parse_display_name, parse_email, parse_username};
    use rstest::*;
    use rstest_reuse::*;
    use std::sync::Arc;

    async fn create_user_confirmed(
        state: &Arc<crate::AppState>,
        username: Username,
        password: host::password::Password,
        display_name: Option<DisplayName>,
        is_operator: bool,
    ) -> UserId {
        let users = Arc::clone(&state.users);
        let password = prepare_password(password)
            .await
            .expect("user fixture password preparation should succeed");
        let outcome = state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    users
                        .create_user(
                            transaction,
                            &username,
                            &password,
                            display_name.as_ref(),
                            is_operator,
                        )
                        .await
                })
            })
            .await
            .expect("user fixture setup should succeed");
        crate::test_support::confirmed_for(outcome, "user fixture setup")
    }

    async fn authenticate(
        state: &Arc<crate::AppState>,
        username: Username,
        password: host::password::Password,
    ) -> Result<UserRecord, crate::WriteScopeError<UserAuthError>> {
        let users = Arc::clone(&state.users);
        let authentication = users
            .prepare_authentication(&username, &password)
            .await
            .map_err(crate::WriteScopeError::Operation)?;
        let outcome = state
            .write_scope
            .run(|transaction| {
                Box::pin(async move { users.authenticate(transaction, authentication).await })
            })
            .await?;
        Ok(crate::test_support::confirmed_for(
            outcome,
            "authentication",
        ))
    }

    #[apply(backends)]
    #[tokio::test]
    async fn user_round_trips_username_display_name_and_email(#[case] backend: Backend) {
        // Keep the whole `TestEnv` bound: dropping `base` unlinks the SQLite file
        // (ADR-0053 TempDir hazard).
        let env = backend.setup().await;

        // `create_user` binds a typed `Username` + `Option<&DisplayName>`, and
        // `set_email` binds `Option<&Email>`; the reads decode each column straight
        // back into its newtype — exercising both bridge directions.
        let username: Username = parse_username("alice");
        let display_name = parse_display_name("Alice Example");
        let password = host::test_support::parse_password("password123");
        let user_id = create_user_confirmed(
            &env.state,
            username.clone(),
            password.clone(),
            Some(display_name.clone()),
            true,
        )
        .await;

        let email = parse_email("alice@example.com");
        let users = Arc::clone(&env.state.users);
        let updated_email = email.clone();
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    users
                        .set_email(transaction, user_id, Some(&updated_email), true)
                        .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, common::MutationOutcome::Confirmed(())));

        let record = env.state.users.get_user(user_id).await.unwrap().unwrap();
        assert_eq!(record.username, username);
        assert_eq!(record.display_name, Some(display_name));
        assert_eq!(record.email, Some(email));
        assert!(record.email_verified);
        assert!(record.is_operator);

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
        assert!(by_name.email_verified);
        assert!(by_name.is_operator);

        let authenticated = authenticate(&env.state, username.clone(), password.clone())
            .await
            .unwrap();
        assert!(authenticated.email_verified);
        assert!(authenticated.is_operator);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn user_round_trips_absent_display_name_and_email(#[case] backend: Backend) {
        // The `None` decode path for `Option<DisplayName>` / `Option<Email>`.
        let env = backend.setup().await;
        let username: Username = parse_username("bob");
        let user_id = create_user_confirmed(
            &env.state,
            username.clone(),
            host::test_support::parse_password("password123"),
            None,
            false,
        )
        .await;

        let record = env.state.users.get_user(user_id).await.unwrap().unwrap();
        assert_eq!(record.username, username);
        assert_eq!(record.display_name, None);
        assert_eq!(record.email, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn user_created_and_authenticated_instants_round_trip(#[case] backend: Backend) {
        let env = backend.setup().await;
        let username = parse_username("authenticated");
        let password = host::test_support::parse_password("password123");
        let user_id =
            create_user_confirmed(&env.state, username.clone(), password.clone(), None, false)
                .await;

        let created = env.state.users.get_user(user_id).await.unwrap().unwrap();
        assert!(created.created_at <= UtcInstant::now());
        assert_eq!(created.last_authenticated_at, None);

        let authenticated = authenticate(&env.state, username.clone(), password.clone())
            .await
            .unwrap();
        assert_eq!(authenticated.created_at, created.created_at);
        assert!(authenticated.last_authenticated_at.is_some());

        let reread = env.state.users.get_user(user_id).await.unwrap().unwrap();
        assert_eq!(
            reread
                .last_authenticated_at
                .map(|instant| instant.value().timestamp_micros()),
            authenticated
                .last_authenticated_at
                .map(|instant| instant.value().timestamp_micros())
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn get_user_rejects_a_malformed_username_column(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;

        // Overwrite the `username` column with a value `Username::from_str`
        // rejects (a space is not a valid username character), binding it as a raw
        // `&str` so the bad value actually lands in the column — the typed bind
        // could not produce it.
        let sql = "UPDATE users SET username = $1 WHERE user_id = $2";
        crate::with_closeable_pool!(env.base.pool(), pool, {
            sqlx::query(sql)
                .bind("bad name")
                .bind(user_id)
                .execute(pool)
                .await
                .unwrap();
        });

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
    async fn update_profile_sets_and_then_clears_bio(#[case] backend: Backend) {
        // `bio` binds as `Option<&Bio>` (a typed newtype bind via the ADR-0071 sqlx
        // bridge) and decodes back into `Option<Bio>`; `None` clears it. Exercises
        // both the set and the clear paths across both backends.
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;

        let bio = parse_bio("hi");
        let users = Arc::clone(&env.state.users);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    users
                        .update_profile(
                            transaction,
                            user_id,
                            &ProfileUpdate {
                                display_name: None,
                                bio: Some(&bio),
                            },
                        )
                        .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, common::MutationOutcome::Confirmed(())));
        let record = env.state.users.get_user(user_id).await.unwrap().unwrap();
        assert_eq!(record.bio, Some(parse_bio("hi")));

        let users = Arc::clone(&env.state.users);
        let outcome = env
            .state
            .write_scope
            .run(|transaction| {
                Box::pin(async move {
                    users
                        .update_profile(
                            transaction,
                            user_id,
                            &ProfileUpdate {
                                display_name: None,
                                bio: None,
                            },
                        )
                        .await
                })
            })
            .await
            .unwrap();
        assert!(matches!(outcome, common::MutationOutcome::Confirmed(())));
        let cleared = env.state.users.get_user(user_id).await.unwrap().unwrap();
        assert_eq!(cleared.bio, None);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn reading_user_with_overlong_bio_in_db_errors(#[case] backend: Backend) {
        // A pre-existing row whose bio exceeds MAX_BIO_CHARS (the column is unbounded
        // TEXT) must surface as a column-decode error at the strict read boundary —
        // never a panic — because the validating sqlx `Decode` fails closed through
        // `Bio`'s `FromStr`. The over-cap value is unconstructible via the newtype, so
        // it is forced in with raw SQL. Mirrors the overlong-display-name case.
        let env = backend.setup().await;
        let user_id = SeedUser::new().seed(&env.state).await.user_id;
        let overlong = "a".repeat(common::bio::MAX_BIO_CHARS + 1);
        let sql = format!(
            "UPDATE users SET bio='{overlong}' WHERE user_id={}",
            i64::from(user_id)
        );
        env.base.pool().execute(sql.as_str()).await.unwrap();
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
        let result = authenticate(
            &env.state,
            parse_username("alice"),
            host::test_support::parse_password("password123"),
        )
        .await;
        let Err(crate::WriteScopeError::Operation(UserAuthError::Internal(source))) = result else {
            panic!("expected authentication preparation to fail against the closed pool");
        };
        assert!(
            matches!(
                source.downcast_ref::<sqlx::Error>(),
                Some(sqlx::Error::PoolClosed)
            ),
            "expected the closed-pool error to remain in the source chain"
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_corrupted_hash_returns_internal_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let sql = format!(
            "UPDATE users SET password_hash='not-a-bcrypt-hash' WHERE username='{}'",
            user.username
        );
        env.base.pool().execute(sql.as_str()).await.unwrap();
        let result = authenticate(
            &env.state,
            user.username,
            host::test_support::parse_password("password123"),
        )
        .await;
        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Operation(UserAuthError::Internal(
                _
            )))
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_invalid_email_in_db_returns_internal_error(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let sql = format!(
            "UPDATE users SET email='not-an-email' WHERE username='{}'",
            user.username
        );
        env.base.pool().execute(sql.as_str()).await.unwrap();
        let result = authenticate(
            &env.state,
            user.username,
            host::test_support::parse_password("password123"),
        )
        .await;
        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Operation(UserAuthError::Internal(
                _
            )))
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_overlong_display_name_in_db_returns_internal_error(
        #[case] backend: Backend,
    ) {
        // A pre-existing row whose display_name exceeds the DisplayName length
        // bound (the column itself is unbounded, #401) must surface as a typed
        // Internal error at the strict read boundary — never a panic. Mirrors the
        // invalid-email-in-db case above.
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
        let overlong = "a".repeat(common::display_name::MAX_DISPLAY_NAME_CHARS + 1);
        let sql = format!(
            "UPDATE users SET display_name='{overlong}' WHERE username='{}'",
            user.username
        );
        env.base.pool().execute(sql.as_str()).await.unwrap();
        let result = authenticate(
            &env.state,
            user.username,
            host::test_support::parse_password("password123"),
        )
        .await;
        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Operation(UserAuthError::Internal(
                _
            )))
        ));
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authenticate_with_blocked_update_returns_internal_error(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await;
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
        let result = authenticate(
            &env.state,
            user.username,
            host::test_support::parse_password("password123"),
        )
        .await;
        assert!(matches!(
            result,
            Err(crate::WriteScopeError::Operation(UserAuthError::Internal(
                _
            )))
        ));
    }

    // guard:no-backend — injected password hashing failure; no database
    #[tokio::test]
    async fn create_user_with_hash_error_returns_internal_error() {
        let password = host::test_support::parse_password("force-hash-error-for-test-coverage");
        let result = prepare_password(password).await;
        assert!(result.is_err());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn authentication_password_source_chain_is_preserved(#[case] backend: Backend) {
        let env = backend.setup().await;
        let username = parse_username("alice");
        let password = host::test_support::parse_password("password123");
        create_user_confirmed(&env.state, username.clone(), password.clone(), None, false).await;
        let expected = crate::helpers::forced_verify_failure(
            &password,
            crate::helpers::dummy_password_hash().as_ref(),
        )
        .unwrap_err();

        let username_for_auth = username.clone();
        let password_for_auth = password.clone();
        let result = crate::with_closeable_pool!(env.base.pool(), pool, {
            let users = UserStore::new((*pool).clone());
            users
                .prepare_authentication_with(
                    &username_for_auth,
                    &password_for_auth,
                    crate::helpers::forced_verify_failure,
                )
                .await
        });

        let error = result.unwrap_err();
        let UserAuthError::Internal(source) = &error else {
            panic!("expected internal authentication failure");
        };
        let io_error = source
            .downcast_ref::<std::io::Error>()
            .expect("UserAuthError retains io::Error");
        let password_error = io_error
            .get_ref()
            .and_then(|source| source.downcast_ref::<host::password::PasswordError>())
            .expect("io::Error retains PasswordError");
        let (
            host::password::PasswordError::VerificationFailed(actual),
            host::password::PasswordError::VerificationFailed(expected),
        ) = (password_error, &expected)
        else {
            panic!("expected typed verification failures");
        };

        assert_eq!(actual, expected);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn continuation_reporting_absent_user_dummy_verify_failure_preserves_invalid_credentials_and_reports_once(
        #[case] backend: Backend,
    ) {
        let env = backend.setup().await;
        let username = parse_username("absent");
        let password = host::test_support::parse_password("password123");
        let base = env.base;
        let operation = async move {
            match base.pool() {
                crate::test_support::CloseablePool::Sqlite(pool) => {
                    let users = UserStore::new(pool.clone());
                    users
                        .prepare_authentication_with(
                            &username,
                            &password,
                            crate::helpers::forced_verify_failure,
                        )
                        .await
                }
                crate::test_support::CloseablePool::Postgres(pool) => {
                    let users = UserStore::new(pool.clone());
                    users
                        .prepare_authentication_with(
                            &username,
                            &password,
                            crate::helpers::forced_verify_failure,
                        )
                        .await
                }
            }
        };
        let (result, trace) = crate::helpers::swallowed_test::capture_async(operation).await;
        assert!(matches!(result, Err(UserAuthError::InvalidCredentials)));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.user.authenticate.dummy_verify",
        );
    }

    // Each variant maps to a fixed `(kind, public_message)` pair.
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

    // Each variant maps to a fixed `(kind, public_message)` pair; the boxed cause
    // chain is preserved (not flattened).
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
