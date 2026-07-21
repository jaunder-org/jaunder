use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Pool, Sqlite};

use crate::helpers::{post_record_from_row, PostRow};
use crate::{PostDialect, PostRecord, PostStore, TaggingError, UpdatePostError, UpdatePostInput};
use common::ids::{PostId, UserId};
use common::tag::{Tag, TagLabel};

/// SQLite-backed post storage.
pub type SqlitePostStorage = PostStore<Sqlite>;

#[async_trait]
impl PostDialect for Sqlite {
    const TAGS_SUBQUERY: &'static str = "COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]')";

    const PERMALINK_DATE_CLAUSE: &'static str = "date(p.published_at) = $3";

    const DELETE_POST_AUDIENCES: &'static str = "DELETE FROM post_audiences WHERE post_id = ?";

    // Bind order: post_id, audience_id, kind_name (matches `replace_post_audiences`).
    const INSERT_POST_AUDIENCE: &'static str = "INSERT INTO post_audiences \
         (post_id, audience_id, target_kind_id) \
         VALUES (?, ?, (SELECT kind_id FROM target_kinds WHERE name = ?))";

    async fn update_post(
        pool: &Pool<Sqlite>,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        // ADR-0021: take the write lock up front with BEGIN IMMEDIATE rather than a
        // deferred BEGIN, so the SELECT->INSERT step performs no shared->reserved lock
        // upgrade (the SQLITE_BUSY-on-upgrade failure mode). sqlx's Transaction issues
        // its own deferred BEGIN, so drive the transaction manually on a raw connection,
        // mirroring create_user_with_invite / sqlite/backup.rs.
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        let now = Utc::now();

        let result: Result<PostRow, UpdatePostError> = async {
            let existing = sqlx::query_as::<_, (i64, Option<DateTime<Utc>>)>(
                "SELECT user_id, deleted_at FROM posts WHERE post_id = $1",
            )
            .bind(i64::from(post_id))
            .fetch_optional(&mut *conn)
            .await?;

            match existing {
                None => return Err(UpdatePostError::NotFound),
                Some((owner_id, deleted_at))
                    if UserId::from(owner_id) != editor_user_id || deleted_at.is_some() =>
                {
                    return Err(UpdatePostError::Unauthorized);
                }
                Some(_) => {}
            }

            sqlx::query(
                "INSERT INTO post_revisions (post_id, user_id, title, slug, body, format, rendered_html, edited_at)
                 SELECT post_id, user_id, title, slug, body, format, rendered_html, $1
                 FROM posts WHERE post_id = $2",
            )
            .bind(now)
            .bind(i64::from(post_id))
            .execute(&mut *conn)
            .await?;

            let row = sqlx::query_as::<_, PostRow>(
                "UPDATE posts
                 SET title = $1,
                     slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END,
                     body = $3,
                     format = $4,
                     rendered_html = $5,
                     published_at = CASE
                         WHEN $6 THEN NULL
                         WHEN $7 IS NOT NULL THEN $8
                         ELSE COALESCE(published_at, $9)
                     END,
                     updated_at = $10,
                     summary = $11
                 WHERE post_id = $12
                 RETURNING post_id, user_id,
                           (SELECT username FROM users WHERE user_id = posts.user_id) AS username,
                           title, slug, body, format, rendered_html,
                           created_at, updated_at, published_at, deleted_at, summary,
                           COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]') AS tags",
            )
            // `Option::as_ref` → `Option<&PostTitle>` (a typed newtype bind, not an
            // `AsRef<str>` strip); the sqlx bridge encodes `Option<&PostTitle>`.
            .bind(input.title.as_ref())
            .bind(&input.slug)
            .bind(&input.body)
            .bind(input.format.to_string())
            .bind(&input.rendered_html)
            // $6 unpublish, $7/$8 explicit_published_at (bound twice: NULL-test
            // then value), $9 now (COALESCE fallback), $10 now (updated_at),
            // $11 summary.
            .bind(input.unpublish)
            .bind(input.explicit_published_at)
            .bind(input.explicit_published_at)
            .bind(now)
            .bind(now)
            // `Option::as_ref` → `Option<&PostSummary>` (a typed newtype bind via the
            // ADR-0071 sqlx bridge, not an `AsRef<str>` strip). Persists a summary
            // edit/clear — the column was previously omitted from the SET clause, so
            // an edited summary was silently dropped (surfaced by #545's clear e2e).
            .bind(input.summary.as_ref())
            .bind(i64::from(post_id))
            .fetch_one(&mut *conn)
            .await?;

            crate::posts::replace_post_audiences::<Sqlite>(&mut *conn, post_id, &input.audiences)
                .await?;

            Ok(row)
        }
        .await;

        match result {
            Ok(row) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                post_record_from_row(row).map_err(UpdatePostError::Internal)
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    async fn tag_post(
        pool: &Pool<Sqlite>,
        post_id: PostId,
        tag: &TagLabel,
    ) -> Result<(), TaggingError> {
        // The slug is the canonical key; the label carries the author's casing.
        let slug = tag.slug();

        // ADR-0021: take the write lock up front with BEGIN IMMEDIATE rather than a
        // deferred BEGIN, so the SELECT->INSERT step performs no shared->reserved lock
        // upgrade (the SQLITE_BUSY-on-upgrade failure mode). sqlx's Transaction issues
        // its own deferred BEGIN, so drive the transaction manually on a raw connection,
        // mirroring update_post / create_user_with_invite / sqlite/backup.rs.
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), TaggingError> = async {
            let post_exists: bool =
                sqlx::query_scalar("SELECT COUNT(*) > 0 FROM posts WHERE post_id = $1")
                    .bind(i64::from(post_id))
                    .fetch_one(&mut *conn)
                    .await?;

            if !post_exists {
                return Err(TaggingError::PostNotFound);
            }

            sqlx::query("INSERT OR IGNORE INTO tags (tag_slug) VALUES ($1)")
                .bind(&slug)
                .execute(&mut *conn)
                .await?;

            let tag_id: i64 =
                sqlx::query_scalar::<_, i64>("SELECT tag_id FROM tags WHERE tag_slug = $1")
                    .bind(&slug)
                    .fetch_one(&mut *conn)
                    .await?;

            match sqlx::query(
                "INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES ($1, $2, $3)",
            )
            .bind(i64::from(post_id))
            .bind(tag_id)
            .bind(tag)
            .execute(&mut *conn)
            .await
            {
                Ok(_) => Ok(()),
                Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                    Err(TaggingError::AlreadyTagged)
                }
                Err(e) => Err(TaggingError::Internal(e)),
            }
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                Err(error)
            }
        }
    }

    async fn untag_post(
        pool: &Pool<Sqlite>,
        post_id: PostId,
        tag_slug: &Tag,
    ) -> Result<(), TaggingError> {
        let rows_deleted = sqlx::query(
            "DELETE FROM post_tags
             WHERE post_id = $1 AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = $2)",
        )
        .bind(i64::from(post_id))
        .bind(tag_slug)
        .execute(pool)
        .await?
        .rows_affected();

        if rows_deleted == 0 {
            Err(TaggingError::TagNotFound)
        } else {
            Ok(())
        }
    }
}
