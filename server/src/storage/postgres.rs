use async_trait::async_trait;
use chrono::{DateTime, Utc};
use email_address::EmailAddress;
use sqlx::PgPool;

use common::password::Password;
use common::storage::{
    ConfirmPasswordResetError, CreateUserError, EmailVerificationStorage, InviteRecord,
    InviteStorage, PasswordResetStorage, ProfileUpdate, RegisterWithInviteError, SessionAuthError,
    SessionRecord, SessionStorage, SiteConfigStorage, UseEmailVerificationError, UseInviteError,
    UsePasswordResetError, UserAuthError, UserRecord, UserStorage,
};
use common::username::Username;

fn unsupported() -> sqlx::Error {
    sqlx::Error::Configuration("postgres backend storage is not implemented yet".into())
}

pub struct PostgresSiteConfigStorage {
    pool: PgPool,
}

impl PostgresSiteConfigStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SiteConfigStorage for PostgresSiteConfigStorage {
    async fn get(&self, _key: &str) -> sqlx::Result<Option<String>> {
        let _ = &self.pool;
        Err(unsupported())
    }

    async fn set(&self, _key: &str, _value: &str) -> sqlx::Result<()> {
        let _ = &self.pool;
        Err(unsupported())
    }
}

pub struct PostgresUserStorage {
    pool: PgPool,
}

impl PostgresUserStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserStorage for PostgresUserStorage {
    async fn create_user(
        &self,
        _username: &Username,
        _password: &Password,
        _display_name: Option<&str>,
    ) -> Result<i64, CreateUserError> {
        let _ = &self.pool;
        Err(CreateUserError::Internal(unsupported()))
    }

    async fn authenticate(
        &self,
        _username: &Username,
        _password: &Password,
    ) -> Result<UserRecord, UserAuthError> {
        let _ = &self.pool;
        Err(UserAuthError::Internal(
            "postgres backend storage is not implemented yet".to_owned(),
        ))
    }

    async fn get_user(&self, _user_id: i64) -> sqlx::Result<Option<UserRecord>> {
        let _ = &self.pool;
        Err(unsupported())
    }

    async fn get_user_by_username(&self, _username: &Username) -> sqlx::Result<Option<UserRecord>> {
        let _ = &self.pool;
        Err(unsupported())
    }

    async fn update_profile(&self, _user_id: i64, _update: &ProfileUpdate<'_>) -> sqlx::Result<()> {
        let _ = &self.pool;
        Err(unsupported())
    }

    async fn set_email(
        &self,
        _user_id: i64,
        _email: Option<&EmailAddress>,
        _verified: bool,
    ) -> sqlx::Result<()> {
        let _ = &self.pool;
        Err(unsupported())
    }

    async fn set_password(&self, _user_id: i64, _new_password: &Password) -> sqlx::Result<()> {
        let _ = &self.pool;
        Err(unsupported())
    }
}

pub struct PostgresSessionStorage {
    pool: PgPool,
}

impl PostgresSessionStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SessionStorage for PostgresSessionStorage {
    async fn create_session(&self, _user_id: i64, _label: Option<&str>) -> sqlx::Result<String> {
        let _ = &self.pool;
        Err(unsupported())
    }

    async fn authenticate(&self, _raw_token: &str) -> Result<SessionRecord, SessionAuthError> {
        let _ = &self.pool;
        Err(SessionAuthError::Internal(unsupported()))
    }

    async fn revoke_session(&self, _token_hash: &str) -> sqlx::Result<()> {
        let _ = &self.pool;
        Err(unsupported())
    }

    async fn list_sessions(&self, _user_id: i64) -> sqlx::Result<Vec<SessionRecord>> {
        let _ = &self.pool;
        Err(unsupported())
    }
}

pub struct PostgresInviteStorage {
    pool: PgPool,
}

impl PostgresInviteStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InviteStorage for PostgresInviteStorage {
    async fn create_invite(&self, _expires_at: DateTime<Utc>) -> sqlx::Result<String> {
        let _ = &self.pool;
        Err(unsupported())
    }

    async fn use_invite(&self, _code: &str, _user_id: i64) -> Result<(), UseInviteError> {
        let _ = &self.pool;
        Err(UseInviteError::NotFound)
    }

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>> {
        let _ = &self.pool;
        Err(unsupported())
    }
}

pub struct PostgresAtomicOps {
    pool: PgPool,
}

impl PostgresAtomicOps {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl common::storage::AtomicOps for PostgresAtomicOps {
    async fn create_user_with_invite(
        &self,
        _username: &Username,
        _password: &Password,
        _display_name: Option<&str>,
        _invite_code: &str,
    ) -> Result<i64, RegisterWithInviteError> {
        let _ = &self.pool;
        Err(RegisterWithInviteError::Internal(unsupported()))
    }

    async fn confirm_password_reset(
        &self,
        _raw_token: &str,
        _new_password: &Password,
    ) -> Result<(), ConfirmPasswordResetError> {
        let _ = &self.pool;
        Err(ConfirmPasswordResetError::Internal(unsupported()))
    }
}

pub struct PostgresEmailVerificationStorage {
    pool: PgPool,
}

impl PostgresEmailVerificationStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EmailVerificationStorage for PostgresEmailVerificationStorage {
    async fn create_email_verification(
        &self,
        _user_id: i64,
        _email: &str,
        _expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String> {
        let _ = &self.pool;
        Err(unsupported())
    }

    async fn use_email_verification(
        &self,
        _raw_token: &str,
    ) -> Result<(i64, String), UseEmailVerificationError> {
        let _ = &self.pool;
        Err(UseEmailVerificationError::Internal(unsupported()))
    }
}

pub struct PostgresPasswordResetStorage {
    pool: PgPool,
}

impl PostgresPasswordResetStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PasswordResetStorage for PostgresPasswordResetStorage {
    async fn create_password_reset(
        &self,
        _user_id: i64,
        _expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String> {
        let _ = &self.pool;
        Err(unsupported())
    }

    async fn use_password_reset(&self, _raw_token: &str) -> Result<i64, UsePasswordResetError> {
        let _ = &self.pool;
        Err(UsePasswordResetError::Internal(unsupported()))
    }
}
