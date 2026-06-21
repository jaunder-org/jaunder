use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Pool, Postgres};

use crate::helpers::{post_record_from_row, PostRow};
use crate::{PostDialect, PostRecord, PostStore, TaggingError, UpdatePostError, UpdatePostInput};
use common::tag::Tag;

/// Postgres-backed post storage.
pub type PostgresPostStorage = PostStore<Postgres>;

#[async_trait]
impl PostDialect for Postgres {
    const TAGS_SUBQUERY: &'static str = "COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text";

    const PERMALINK_DATE_CLAUSE: &'static str =
        "date(p.published_at AT TIME ZONE 'UTC') = $3::date";

    const DELETE_POST_AUDIENCES: &'static str = "DELETE FROM post_audiences WHERE post_id = $1";

    // Bind order: post_id, audience_id, kind_name (matches `replace_post_audiences`).
    const INSERT_POST_AUDIENCE: &'static str = "INSERT INTO post_audiences \
         (post_id, audience_id, target_kind_id) \
         VALUES ($1, $2, (SELECT kind_id FROM target_kinds WHERE name = $3))";

    async fn update_post(
        pool: &Pool<Postgres>,
        post_id: i64,
        editor_user_id: i64,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        let mut tx = pool.begin().await?;
        let now = Utc::now();

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

        sqlx::query(
            "INSERT INTO post_revisions (post_id, user_id, title, slug, body, format, rendered_html, edited_at)
             SELECT post_id, user_id, title, slug, body, format, rendered_html, $1
             FROM posts WHERE post_id = $2",
        )
        .bind(now)
        .bind(post_id)
        .execute(&mut *tx)
        .await?;

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
                       created_at, updated_at, published_at, deleted_at, summary,
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

        crate::posts::replace_post_audiences::<Postgres>(&mut tx, post_id, &input.audiences)
            .await?;

        tx.commit().await?;
        post_record_from_row(row).map_err(UpdatePostError::Internal)
    }

    async fn tag_post(
        pool: &Pool<Postgres>,
        post_id: i64,
        tag_display: &str,
    ) -> Result<(), TaggingError> {
        let tag: Tag = tag_display.parse().map_err(|_| {
            TaggingError::Internal(sqlx::Error::Decode("invalid tag format".into()))
        })?;

        let mut tx = pool.begin().await?;

        let post_exists: bool =
            sqlx::query_scalar("SELECT COUNT(*) > 0 FROM posts WHERE post_id = $1")
                .bind(post_id)
                .fetch_one(&mut *tx)
                .await?;

        if !post_exists {
            tx.rollback().await.ok();
            return Err(TaggingError::PostNotFound);
        }

        sqlx::query("INSERT INTO tags (tag_slug) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(tag.as_str())
            .execute(&mut *tx)
            .await?;

        let tag_id: i64 =
            sqlx::query_scalar::<_, i64>("SELECT tag_id FROM tags WHERE tag_slug = $1")
                .bind(tag.as_str())
                .fetch_one(&mut *tx)
                .await?;

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

    async fn untag_post(
        pool: &Pool<Postgres>,
        post_id: i64,
        tag_slug: &Tag,
    ) -> Result<(), TaggingError> {
        let rows_deleted = sqlx::query(
            "DELETE FROM post_tags
             WHERE post_id = $1 AND tag_id = (SELECT tag_id FROM tags WHERE tag_slug = $2)",
        )
        .bind(post_id)
        .bind(tag_slug.as_str())
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
