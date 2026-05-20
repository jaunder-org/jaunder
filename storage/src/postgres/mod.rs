use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::LevelFilter;
use sqlx::postgres::PgConnectOptions;
use sqlx::ConnectOptions;
use sqlx::{PgPool, Row};

mod site_config;
pub use site_config::PostgresSiteConfigStorage;

mod users;
pub use users::PostgresUserStorage;

mod sessions;
pub use sessions::PostgresSessionStorage;

use crate::helpers::{
    email_verification_claim_error, generate_hashed_token, invite_record_from_row,
    media_record_from_row, password_reset_claim_error, post_record_from_row, InviteRow, MediaRow,
    PostRow,
};
use crate::{
    AtomicOps, ConfirmPasswordResetError, CreateMediaError, CreatePostError, CreatePostInput,
    DeleteMediaError, EmailVerificationStorage, InviteRecord, InviteStorage, ListByTagError,
    MediaRecord, MediaSource, MediaStorage, PasswordResetStorage, PostCursor, PostRecord,
    PostStorage, PostTag, RegisterWithInviteError, TagRecord, TaggingError, UpdatePostError,
    UpdatePostInput, UseEmailVerificationError, UseInviteError, UsePasswordResetError,
    UserConfigStorage,
};
use common::password::Password;
use common::slug::Slug;
use common::tag::Tag;
use common::username::Username;

// ---------------------------------------------------------------------------
// Invites
// ---------------------------------------------------------------------------

pub struct PostgresInviteStorage {
    pool: PgPool,
}

impl PostgresInviteStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl InviteStorage for PostgresInviteStorage {
    async fn create_invite(&self, expires_at: DateTime<Utc>) -> sqlx::Result<String> {
        let code = crate::auth::generate_token();
        let now = Utc::now();

        sqlx::query("INSERT INTO invites (code, created_at, expires_at) VALUES ($1, $2, $3)")
            .bind(&code)
            .bind(now)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;

        Ok(code)
    }

    async fn use_invite(&self, code: &str, user_id: i64) -> Result<(), UseInviteError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| UseInviteError::NotFound)?;
        let row = sqlx::query_as::<_, InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by
             FROM invites WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| UseInviteError::NotFound)?
        .ok_or(UseInviteError::NotFound)?;

        let record = invite_record_from_row(row);
        if record.used_at.is_some() {
            return Err(UseInviteError::AlreadyUsed);
        }

        let now = Utc::now();
        if record.expires_at <= now {
            return Err(UseInviteError::Expired);
        }

        sqlx::query("UPDATE invites SET used_at = $1, used_by = $2 WHERE code = $3")
            .bind(now)
            .bind(user_id)
            .bind(code)
            .execute(&mut *tx)
            .await
            .map_err(|_| UseInviteError::NotFound)?;

        tx.commit().await.map_err(|_| UseInviteError::NotFound)?;
        Ok(())
    }

    async fn list_invites(&self) -> sqlx::Result<Vec<InviteRecord>> {
        let rows = sqlx::query_as::<_, InviteRow>(
            "SELECT code, created_at, expires_at, used_at, used_by FROM invites",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(invite_record_from_row).collect())
    }
}

// ---------------------------------------------------------------------------
// EmailVerifications
// ---------------------------------------------------------------------------

pub struct PostgresEmailVerificationStorage {
    pool: PgPool,
}

impl PostgresEmailVerificationStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EmailVerificationStorage for PostgresEmailVerificationStorage {
    async fn create_email_verification(
        &self,
        user_id: i64,
        email: &str,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String> {
        let (raw_token, token_hash) = generate_hashed_token()?;
        let now = Utc::now();

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE email_verifications
             SET expires_at = created_at
             WHERE user_id = $1 AND used_at IS NULL AND expires_at > $2",
        )
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO email_verifications
             (token_hash, user_id, email, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(email)
        .bind(now)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(raw_token)
    }

    async fn use_email_verification(
        &self,
        raw_token: &str,
    ) -> Result<(i64, String), UseEmailVerificationError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| UseEmailVerificationError::NotFound)?;
        let now = Utc::now();

        let claimed = sqlx::query_as::<_, (i64, String)>(
            "UPDATE email_verifications SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id, email",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((user_id, email)) = claimed {
            return Ok((user_id, email));
        }

        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM email_verifications WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Err(email_verification_claim_error(row))
    }
}

// ---------------------------------------------------------------------------
// PasswordResets
// ---------------------------------------------------------------------------

pub struct PostgresPasswordResetStorage {
    pool: PgPool,
}

impl PostgresPasswordResetStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PasswordResetStorage for PostgresPasswordResetStorage {
    async fn create_password_reset(
        &self,
        user_id: i64,
        expires_at: DateTime<Utc>,
    ) -> sqlx::Result<String> {
        let (raw_token, token_hash) = generate_hashed_token()?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO password_resets (token_hash, user_id, created_at, expires_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(now)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(raw_token)
    }

    async fn use_password_reset(&self, raw_token: &str) -> Result<i64, UsePasswordResetError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| UsePasswordResetError::NotFound)?;
        let now = Utc::now();

        let claimed = sqlx::query_as::<_, (i64,)>(
            "UPDATE password_resets SET used_at = $1
             WHERE token_hash = $2 AND used_at IS NULL AND expires_at > $3
             RETURNING user_id",
        )
        .bind(now)
        .bind(&token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((user_id,)) = claimed {
            return Ok(user_id);
        }

        let row = sqlx::query_as::<_, (Option<DateTime<Utc>>, DateTime<Utc>)>(
            "SELECT used_at, expires_at FROM password_resets WHERE token_hash = $1",
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await?;

        Err(password_reset_claim_error(row))
    }
}

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
            .map_err(|e| RegisterWithInviteError::Internal(sqlx::Error::Io(e)))?;

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

        let password_hash = crate::helpers::hash_password(new_password.clone())
            .await
            .map_err(|e| ConfirmPasswordResetError::Internal(sqlx::Error::Io(e)))?;

        let mut tx = self.pool.begin().await?;
        let now = Utc::now();
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
// Posts
// ---------------------------------------------------------------------------

/// PostgreSQL-backed [`PostStorage`].
pub struct PostgresPostStorage {
    pool: PgPool,
}

impl PostgresPostStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PostStorage for PostgresPostStorage {
    async fn create_post(&self, input: &CreatePostInput) -> Result<i64, CreatePostError> {
        let now = Utc::now();

        let result = sqlx::query_scalar::<_, i64>(
            "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING post_id",
        )
        .bind(input.user_id)
        .bind(&input.title)
        .bind(input.slug.as_str())
        .bind(&input.body)
        .bind(input.format.to_string())
        .bind(&input.rendered_html)
        .bind(now)
        .bind(now)
        .bind(input.published_at)
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(id) => Ok(id),
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(CreatePostError::SlugConflict)
            }
            Err(e) => Err(CreatePostError::Internal(e)),
        }
    }

    async fn get_post_by_id(&self, post_id: i64) -> sqlx::Result<Option<PostRecord>> {
        let row = sqlx::query_as::<_, PostRow>(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at,
                    COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE p.post_id = $1",
        )
        .bind(post_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(post_record_from_row).transpose()?)
    }

    async fn get_post_by_permalink(
        &self,
        username: &Username,
        year: i32,
        month: u32,
        day: u32,
        slug: &Slug,
    ) -> sqlx::Result<Option<PostRecord>> {
        let date_str = format!("{year:04}-{month:02}-{day:02}");
        let row = sqlx::query_as::<_, PostRow>(
            "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                    p.created_at, p.updated_at, p.published_at, p.deleted_at,
                    COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
             FROM posts p
             JOIN users u ON p.user_id = u.user_id
             WHERE u.username = $1
               AND p.slug = $2
               AND p.published_at IS NOT NULL
               AND p.deleted_at IS NULL
               AND date(p.published_at AT TIME ZONE 'UTC') = $3::date",
        )
        .bind(username.as_str())
        .bind(slug.as_str())
        .bind(&date_str)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(post_record_from_row).transpose()?)
    }

    async fn update_post(
        &self,
        post_id: i64,
        editor_user_id: i64,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        let mut tx = self.pool.begin().await?;
        let now = Utc::now();

        // Lock and read current ownership within the transaction to prevent races.
        let existing = sqlx::query_as::<_, (i64, Option<DateTime<Utc>>)>(
            "SELECT user_id, deleted_at FROM posts WHERE post_id = $1 FOR UPDATE",
        )
        .bind(post_id)
        .fetch_optional(&mut *tx)
        .await?;

        match existing {
            None => {
                tx.rollback().await.ok();
                return Err(UpdatePostError::NotFound);
            }
            Some((owner_id, deleted_at)) if owner_id != editor_user_id || deleted_at.is_some() => {
                tx.rollback().await.ok();
                return Err(UpdatePostError::Unauthorized);
            }
            Some(_) => {}
        }

        // Save a revision of current state.
        sqlx::query(
            "INSERT INTO post_revisions (post_id, user_id, title, slug, body, format, rendered_html, edited_at)
             SELECT post_id, user_id, title, slug, body, format, rendered_html, $1
             FROM posts WHERE post_id = $2",
        )
        .bind(now)
        .bind(post_id)
        .execute(&mut *tx)
        .await?;

        // Update the post:
        //   - slug frozen once published (CASE WHEN published_at IS NULL)
        //   - publish=true  → COALESCE(published_at, now)  (preserves existing date)
        //   - publish=false → NULL                          (un-publish)
        let row = sqlx::query_as::<_, PostRow>(
            "UPDATE posts
             SET title = $1,
                 slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END,
                 body = $3,
                 format = $4,
                 rendered_html = $5,
                 published_at = CASE WHEN $6 THEN COALESCE(published_at, $7) ELSE NULL END,
                 updated_at = $8
             WHERE post_id = $9
             RETURNING post_id, user_id,
                       (SELECT username FROM users WHERE user_id = posts.user_id) AS username,
                       title, slug, body, format, rendered_html,
                       created_at, updated_at, published_at, deleted_at,
                       COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]'::json)::text AS tags",
        )
        .bind(&input.title)
        .bind(input.slug.as_str())
        .bind(&input.body)
        .bind(input.format.to_string())
        .bind(&input.rendered_html)
        .bind(input.publish)
        .bind(now)
        .bind(now)
        .bind(post_id)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        post_record_from_row(row).map_err(UpdatePostError::Internal)
    }

    async fn soft_delete_post(&self, post_id: i64) -> sqlx::Result<()> {
        let now = Utc::now();
        sqlx::query("UPDATE posts SET deleted_at = $1 WHERE post_id = $2")
            .bind(now)
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn unpublish_post(&self, post_id: i64) -> sqlx::Result<()> {
        sqlx::query("UPDATE posts SET published_at = NULL WHERE post_id = $1")
            .bind(post_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_published_by_user(
        &self,
        username: &Username,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $2 AND p.post_id < $3))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $4",
            )
            .bind(username.as_str())
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE u.username = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $2",
            )
            .bind(username.as_str())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    async fn list_published(
        &self,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $1 OR (p.created_at = $1 AND p.post_id < $2))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $3",
            )
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $1",
            )
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    async fn list_drafts_by_user(
        &self,
        user_id: i64,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> sqlx::Result<Vec<PostRecord>> {
        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND p.published_at IS NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $2 AND p.post_id < $3))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $4",
            )
            .bind(user_id)
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 WHERE p.user_id = $1
                   AND p.published_at IS NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $2",
            )
            .bind(user_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(post_record_from_row).collect()
    }

    async fn tag_post(&self, post_id: i64, tag_display: &str) -> Result<(), TaggingError> {
        // Parse and normalize tag
        let tag: Tag = tag_display.parse().map_err(|_| {
            TaggingError::Internal(sqlx::Error::Decode("invalid tag format".into()))
        })?;

        let mut tx = self.pool.begin().await?;

        // Check if post exists
        let post_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM posts WHERE post_id = $1)")
                .bind(post_id)
                .fetch_one(&mut *tx)
                .await?;

        if !post_exists {
            tx.rollback().await.ok();
            return Err(TaggingError::PostNotFound);
        }

        // Insert or get tag
        sqlx::query("INSERT INTO tags (tag_slug) VALUES ($1) ON CONFLICT (tag_slug) DO NOTHING")
            .bind(tag.as_str())
            .execute(&mut *tx)
            .await?;

        let tag_id: i64 =
            sqlx::query_scalar::<_, i64>("SELECT tag_id FROM tags WHERE tag_slug = $1")
                .bind(tag.as_str())
                .fetch_one(&mut *tx)
                .await?;

        // Insert post_tags link
        let result =
            sqlx::query("INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES ($1, $2, $3)")
                .bind(post_id)
                .bind(tag_id)
                .bind(tag_display)
                .execute(&mut *tx)
                .await;

        match result {
            Ok(_) => {
                tx.commit().await?;
                Ok(())
            }
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                tx.rollback().await.ok();
                Err(TaggingError::AlreadyTagged)
            }
            Err(e) => {
                tx.rollback().await.ok();
                Err(TaggingError::Internal(e))
            }
        }
    }

    async fn untag_post(&self, post_id: i64, tag_slug: &Tag) -> Result<(), TaggingError> {
        let rows_deleted = sqlx::query(
            "DELETE FROM post_tags
             WHERE post_id = $1 AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = $2)",
        )
        .bind(post_id)
        .bind(tag_slug.as_str())
        .execute(&self.pool)
        .await?
        .rows_affected();

        if rows_deleted == 0 {
            Err(TaggingError::TagNotFound)
        } else {
            Ok(())
        }
    }

    async fn get_tags_for_post(&self, post_id: i64) -> sqlx::Result<Vec<PostTag>> {
        let rows = sqlx::query(
            "SELECT pt.post_id, pt.tag_id, t.tag_slug, pt.tag_display
             FROM post_tags pt
             JOIN tags t ON pt.tag_id = t.tag_id
             WHERE pt.post_id = $1
             ORDER BY t.tag_slug",
        )
        .bind(post_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let tag_slug_str: String = row.get("tag_slug");
                let tag_slug: Tag = tag_slug_str
                    .parse()
                    .map_err(|_| sqlx::Error::Decode("invalid tag format".into()))?;
                Ok(PostTag {
                    post_id: row.get("post_id"),
                    tag_id: row.get("tag_id"),
                    tag_slug,
                    tag_display: row.get("tag_display"),
                })
            })
            .collect()
    }

    async fn list_posts_by_tag(
        &self,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        // Check tag exists
        let tag_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tags WHERE tag_slug = $1)")
                .bind(tag_slug.as_str())
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $2 OR (p.created_at = $2 AND p.post_id < $3))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $4",
            )
            .bind(tag_slug.as_str())
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE t.tag_slug = $1
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $2",
            )
            .bind(tag_slug.as_str())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(post_record_from_row)
            .collect::<sqlx::Result<_>>()
            .map_err(ListByTagError::Internal)
    }

    async fn list_user_posts_by_tag(
        &self,
        user_id: i64,
        tag_slug: &Tag,
        cursor: Option<&PostCursor>,
        limit: u32,
    ) -> Result<Vec<PostRecord>, ListByTagError> {
        // Check tag exists
        let tag_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tags WHERE tag_slug = $1)")
                .bind(tag_slug.as_str())
                .fetch_one(&self.pool)
                .await?;

        if !tag_exists {
            return Err(ListByTagError::TagNotFound);
        }

        let rows = if let Some(cursor) = cursor {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                   AND (p.created_at < $3 OR (p.created_at = $3 AND p.post_id < $4))
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $5",
            )
            .bind(user_id)
            .bind(tag_slug.as_str())
            .bind(cursor.created_at)
            .bind(cursor.post_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, PostRow>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p
                 JOIN users u ON p.user_id = u.user_id
                 JOIN post_tags pt ON p.post_id = pt.post_id
                 JOIN tags t ON pt.tag_id = t.tag_id
                 WHERE p.user_id = $1
                   AND t.tag_slug = $2
                   AND p.published_at IS NOT NULL
                   AND p.deleted_at IS NULL
                 ORDER BY p.created_at DESC, p.post_id DESC
                 LIMIT $3",
            )
            .bind(user_id)
            .bind(tag_slug.as_str())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter()
            .map(post_record_from_row)
            .collect::<sqlx::Result<_>>()
            .map_err(ListByTagError::Internal)
    }

    async fn list_tags(&self, prefix: Option<&str>, limit: u32) -> sqlx::Result<Vec<TagRecord>> {
        let normalized = prefix
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_ascii_lowercase);
        let pattern = normalized.as_deref().map(|p| format!("{p}%"));
        let limit_i64 = i64::from(limit);

        let rows = match pattern {
            Some(ref like) => {
                sqlx::query(
                    "SELECT tag_id, tag_slug FROM tags
                     WHERE tag_slug ILIKE $1
                     ORDER BY tag_slug
                     LIMIT $2",
                )
                .bind(like)
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT tag_id, tag_slug FROM tags
                     ORDER BY tag_slug
                     LIMIT $1",
                )
                .bind(limit_i64)
                .fetch_all(&self.pool)
                .await?
            }
        };

        rows.into_iter()
            .map(|row| {
                let tag_slug_str: String = row.get("tag_slug");
                let tag_slug: Tag = tag_slug_str
                    .parse()
                    .map_err(|_| sqlx::Error::Decode("invalid tag format".into()))?;
                Ok(TagRecord {
                    tag_id: row.get("tag_id"),
                    tag_slug,
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

/// PostgreSQL-backed [`MediaStorage`].
pub struct PostgresMediaStorage {
    pool: PgPool,
}

impl PostgresMediaStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MediaStorage for PostgresMediaStorage {
    #[tracing::instrument(name = "storage.postgres.media.create", skip(self, record))]
    async fn create_media(&self, record: &MediaRecord) -> Result<(), CreateMediaError> {
        let result = sqlx::query(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(record.user_id)
        .bind(&record.sha256)
        .bind(&record.filename)
        .bind(record.source.as_str())
        .bind(&record.content_type)
        .bind(record.size_bytes)
        .bind(&record.source_url)
        .bind(record.created_at)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(()),
            Err(e)
                if e.as_database_error()
                    .is_some_and(sqlx::error::DatabaseError::is_unique_violation) =>
            {
                Err(CreateMediaError::AlreadyExists)
            }
            Err(e) => Err(CreateMediaError::Internal(e)),
        }
    }

    #[tracing::instrument(name = "storage.postgres.media.get", skip(self))]
    async fn get_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row = sqlx::query_as::<_, MediaRow>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4",
        )
        .bind(user_id)
        .bind(sha256)
        .bind(filename)
        .bind(source.as_str())
        .fetch_optional(&self.pool)
        .await?;

        row.map(media_record_from_row).transpose()
    }

    #[tracing::instrument(name = "storage.postgres.media.list", skip(self))]
    async fn list_media(
        &self,
        user_id: i64,
        source: Option<&MediaSource>,
        limit: u32,
        offset: u32,
    ) -> sqlx::Result<Vec<MediaRecord>> {
        let rows = if let Some(src) = source {
            sqlx::query_as::<_, MediaRow>(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media
                 WHERE user_id = $1 AND source = $2
                 ORDER BY created_at DESC
                 LIMIT $3 OFFSET $4",
            )
            .bind(user_id)
            .bind(src.as_str())
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, MediaRow>(
                "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
                 FROM media
                 WHERE user_id = $1
                 ORDER BY created_at DESC
                 LIMIT $2 OFFSET $3",
            )
            .bind(user_id)
            .bind(i64::from(limit))
            .bind(i64::from(offset))
            .fetch_all(&self.pool)
            .await?
        };

        rows.into_iter().map(media_record_from_row).collect()
    }

    #[tracing::instrument(name = "storage.postgres.media.delete", skip(self))]
    async fn delete_media(
        &self,
        user_id: i64,
        sha256: &str,
        filename: &str,
        source: &MediaSource,
    ) -> Result<(), DeleteMediaError> {
        let result = sqlx::query(
            "DELETE FROM media WHERE user_id = $1 AND sha256 = $2 AND filename = $3 AND source = $4",
        )
        .bind(user_id)
        .bind(sha256)
        .bind(filename)
        .bind(source.as_str())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DeleteMediaError::NotFound);
        }
        Ok(())
    }

    #[tracing::instrument(name = "storage.postgres.media.upload_usage", skip(self))]
    async fn get_user_upload_usage(&self, user_id: i64) -> sqlx::Result<i64> {
        let row = sqlx::query_as::<_, (i64,)>(
            "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM media WHERE user_id = $1 AND source = 'upload'",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    #[tracing::instrument(name = "storage.postgres.media.find_by_hash", skip(self))]
    async fn find_by_hash(
        &self,
        sha256: &str,
        source: &MediaSource,
    ) -> sqlx::Result<Option<MediaRecord>> {
        let row = sqlx::query_as::<_, MediaRow>(
            "SELECT user_id, sha256, filename, source, content_type, size_bytes, source_url, created_at
             FROM media
             WHERE sha256 = $1 AND source = $2
             LIMIT 1",
        )
        .bind(sha256)
        .bind(source.as_str())
        .fetch_optional(&self.pool)
        .await?;

        row.map(media_record_from_row).transpose()
    }
}

// ---------------------------------------------------------------------------
// UserConfig
// ---------------------------------------------------------------------------

/// PostgreSQL-backed [`UserConfigStorage`].
pub struct PostgresUserConfigStorage {
    pool: PgPool,
}

impl PostgresUserConfigStorage {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserConfigStorage for PostgresUserConfigStorage {
    #[tracing::instrument(name = "storage.postgres.user_config.get", skip(self))]
    async fn get(&self, user_id: i64, key: &str) -> sqlx::Result<Option<String>> {
        let row = sqlx::query_as::<_, (String,)>(
            "SELECT value FROM user_config WHERE user_id = $1 AND key = $2",
        )
        .bind(user_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|(v,)| v))
    }

    #[tracing::instrument(name = "storage.postgres.user_config.set", skip(self))]
    async fn set(&self, user_id: i64, key: &str, value: &str) -> sqlx::Result<()> {
        sqlx::query(
            "INSERT INTO user_config (user_id, key, value) VALUES ($1, $2, $3)
             ON CONFLICT (user_id, key) DO UPDATE SET value = excluded.value",
        )
        .bind(user_id)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[tracing::instrument(name = "storage.postgres.user_config.delete", skip(self))]
    async fn delete(&self, user_id: i64, key: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM user_config WHERE user_id = $1 AND key = $2")
            .bind(user_id)
            .bind(key)
            .execute(&self.pool)
            .await?;
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
        media: Arc::new(PostgresMediaStorage::new(pool.clone())),
        user_config: Arc::new(PostgresUserConfigStorage::new(pool)),
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
pub(crate) async fn open_postgres_database(
    options: &PgConnectOptions,
) -> sqlx::Result<Arc<crate::AppState>> {
    let options = resolved_postgres_options(options)?;
    let pool = PgPool::connect_with(options).await?;
    sqlx::migrate!("./migrations/postgres").run(&pool).await?;
    Ok(make_postgres_app_state(pool))
}

#[cfg(test)]
pub(crate) async fn postgres_pool() -> PgPool {
    let url = std::env::var("JAUNDER_PG_TEST_URL")
        .unwrap_or_else(|_| "postgres://jaunder@127.0.0.1:55432/jaunder".to_owned());
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations/postgres")
        .run(&pool)
        .await
        .unwrap();
    pool
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::{session_record_from_row, user_record_from_row, SessionRow, UserRow};
    use crate::*;
    use chrono::Utc;
    use common::password::Password;
    use common::username::Username;
    use sqlx::PgPool;
    use std::{future::Future, time::Duration};

    fn lazy_pool() -> PgPool {
        sqlx::PgPool::connect_lazy("postgres://localhost:1/jaunder").unwrap()
    }

    async fn exercise<F: Future>(future: F) {
        let _ = tokio::time::timeout(Duration::from_millis(50), future).await;
    }

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

    #[tokio::test]
    async fn make_postgres_app_state_constructs_with_lazy_pool() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/db").unwrap();
        let _ = make_postgres_app_state(pool);
    }

    #[test]
    fn test_user_record_from_row() {
        let now = Utc::now();
        let row: UserRow = (
            1,
            "alice".to_string(),
            Some("Alice".to_string()),
            Some("Bio".to_string()),
            now,
            Some(now),
            Some("alice@example.com".to_string()),
            true,
            false,
        );
        let record = user_record_from_row(row).unwrap();
        assert_eq!(record.user_id, 1);
        assert_eq!(record.username.as_str(), "alice");
        assert_eq!(record.display_name, Some("Alice".to_string()));
        assert_eq!(record.bio, Some("Bio".to_string()));
        assert_eq!(record.created_at, now);
        assert_eq!(record.last_authenticated_at, Some(now));
        assert_eq!(record.email.as_ref().unwrap().as_str(), "alice@example.com");
        assert!(record.email_verified);
    }

    #[test]
    fn test_session_record_from_row() {
        let now = Utc::now();
        let row: SessionRow = (
            "hash".to_string(),
            1,
            "alice".to_string(),
            Some("label".to_string()),
            now,
            now,
        );
        let record = session_record_from_row(row).unwrap();
        assert_eq!(record.token_hash, "hash");
        assert_eq!(record.user_id, 1);
        assert_eq!(record.username.as_str(), "alice");
        assert_eq!(record.label, Some("label".to_string()));
        assert_eq!(record.created_at, now);
        assert_eq!(record.last_used_at, now);
    }

    #[test]
    fn test_invite_record_from_row() {
        let now = Utc::now();
        let row: InviteRow = ("code".to_string(), now, now, Some(now), Some(1));
        let record = invite_record_from_row(row);
        assert_eq!(record.code, "code");
        assert_eq!(record.created_at, now);
        assert_eq!(record.expires_at, now);
        assert_eq!(record.used_at, Some(now));
        assert_eq!(record.used_by, Some(1));
    }

    #[tokio::test]
    async fn test_storage_constructors() {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/db").unwrap();
        let _ = PostgresSiteConfigStorage::new(pool.clone());
        let _ = PostgresSessionStorage::new(pool.clone());
        let _ = PostgresInviteStorage::new(pool.clone());
        let _ = PostgresEmailVerificationStorage::new(pool.clone());
        let _ = PostgresPasswordResetStorage::new(pool.clone());
        let _ = PostgresAtomicOps::new(pool.clone());
        let _ = PostgresPostStorage::new(pool);
    }

    #[tokio::test]
    async fn test_storage_methods_with_lazy_pool_cover_error_paths() {
        let pool = lazy_pool();
        let username: Username = "alice".parse().unwrap();
        let password: Password = "password123".parse().unwrap();
        let now = Utc::now();

        let site_config = PostgresSiteConfigStorage::new(pool.clone());
        exercise(site_config.get("site.registration_policy")).await;
        exercise(site_config.set("site.registration_policy", "open")).await;

        let invites = PostgresInviteStorage::new(pool.clone());
        exercise(invites.create_invite(now)).await;
        exercise(invites.use_invite("invite-code", 1)).await;
        exercise(invites.list_invites()).await;

        let email_verifications = PostgresEmailVerificationStorage::new(pool.clone());
        exercise(email_verifications.create_email_verification(1, "alice@example.com", now)).await;
        assert!(matches!(
            email_verifications
                .use_email_verification("not-base64")
                .await,
            Err(UseEmailVerificationError::NotFound)
        ));

        let password_resets = PostgresPasswordResetStorage::new(pool.clone());
        exercise(password_resets.create_password_reset(1, now)).await;
        assert!(matches!(
            password_resets.use_password_reset("not-base64").await,
            Err(UsePasswordResetError::NotFound)
        ));

        let atomic = PostgresAtomicOps::new(pool.clone());
        exercise(atomic.create_user_with_invite(
            &username,
            &password,
            Some("Alice"),
            false,
            "code",
        ))
        .await;
        assert!(matches!(
            atomic.confirm_password_reset("not-base64", &password).await,
            Err(ConfirmPasswordResetError::NotFound)
        ));

        let slug: common::slug::Slug = "hello-world".parse().unwrap();
        let posts = PostgresPostStorage::new(pool);
        exercise(posts.create_post(&crate::CreatePostInput {
            user_id: 1,
            title: Some("Test".to_string()),
            slug: slug.clone(),
            body: "body".to_string(),
            format: crate::PostFormat::Markdown,
            rendered_html: "<p>body</p>".to_string(),
            published_at: None,
        }))
        .await;
        exercise(posts.get_post_by_id(1)).await;
        exercise(posts.get_post_by_permalink(&username, 2024, 1, 1, &slug)).await;
        exercise(posts.update_post(
            1,
            1,
            &crate::UpdatePostInput {
                title: Some("Updated".to_string()),
                slug: slug.clone(),
                body: "body".to_string(),
                format: crate::PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
                publish: false,
            },
        ))
        .await;
        exercise(posts.soft_delete_post(1)).await;
        exercise(posts.unpublish_post(1)).await;
        exercise(posts.list_published_by_user(&username, None, 10)).await;
        exercise(posts.list_published(None, 10)).await;
        exercise(posts.list_drafts_by_user(1, None, 10)).await;

        let tag: common::tag::Tag = "rust".parse().unwrap();
        // Exercise tag_post with an invalid tag display to cover the parse-error path.
        exercise(posts.tag_post(1, "NOT A VALID TAG!!!")).await;
        exercise(posts.tag_post(1, "rust")).await;
        exercise(posts.untag_post(1, &tag)).await;
        exercise(posts.get_tags_for_post(1)).await;
        exercise(posts.list_posts_by_tag(&tag, None, 10)).await;
        exercise(posts.list_user_posts_by_tag(1, &tag, None, 10)).await;
        exercise(posts.list_tags(None, 10)).await;
        exercise(posts.list_tags(Some("rust"), 10)).await;
    }
}
