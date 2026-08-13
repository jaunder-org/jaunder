use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Pool, Postgres};

use crate::helpers::{PostRow, post_record_from_row};
use crate::posts::{
    DELETE_POST_TAG_BY_SLUG, INSERT_POST_TAG, SELECT_POST_TAGS, UPSERT_TAG_RETURNING_ID,
    post_tag_diff, post_tags_from_rows,
};
use crate::{PostDialect, PostRecord, PostStore, TaggingError, UpdatePostError, UpdatePostInput};
use common::ids::{PostId, TagId, UserId};
use common::tag::{Tag, TagLabel};

/// Postgres-backed post storage.
pub type PostgresPostStorage = PostStore<Postgres>;

#[async_trait]
impl PostDialect for Postgres {
    /// `ORDER BY t.tag_slug COLLATE "C"` is what makes [`PostRecord::tags`]
    /// slug-ordered (#772). The `COLLATE` is load-bearing — see
    /// [`PostDialect::TAGS_SUBQUERY`] for why — and must stay in sync with the
    /// `SQLite` twin.
    const TAGS_SUBQUERY: &'static str = "COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display) ORDER BY t.tag_slug COLLATE \"C\") FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text";

    const PERMALINK_DATE_CLAUSE: &'static str =
        "date(COALESCE(p.published_at, p.created_at) AT TIME ZONE 'UTC') = $3::date";

    const DELETE_POST_AUDIENCES: &'static str = "DELETE FROM post_audiences WHERE post_id = $1";

    // Bind order: post_id, audience_id, kind_name (matches `replace_post_audiences`).
    const INSERT_POST_AUDIENCE: &'static str = "INSERT INTO post_audiences \
         (post_id, audience_id, target_kind_id) \
         VALUES ($1, $2, (SELECT kind_id FROM target_kinds WHERE name = $3))";

    const DELETE_POST_MEDIA: &'static str = "DELETE FROM post_media WHERE post_id = $1";

    // Bind order: post_id, source, sha256, filename (matches `replace_post_media`).
    const INSERT_POST_MEDIA: &'static str = "INSERT INTO post_media \
         (post_id, source, sha256, filename) \
         VALUES ($1, $2, $3, $4)";

    async fn update_post(
        pool: &Pool<Postgres>,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        let mut tx = pool.begin().await?;
        let now = Utc::now();

        // FOR UPDATE locks the row for the read-then-write: it stops a concurrent
        // edit from slipping between this ownership/liveness check and the UPDATE
        // below (ADR-0021 / #52). SQLite needs no equivalent — its transaction
        // already serializes writers.
        let existing = sqlx::query_as::<_, (UserId, Option<DateTime<Utc>>)>(
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
                       COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]'::json)::text AS tags",
        )
        // `Option::as_ref` → `Option<&PostTitle>` (a typed newtype bind, not an
        // `AsRef<str>` strip); the sqlx bridge encodes `Option<&PostTitle>`.
        .bind(input.title.as_ref())
        .bind(&input.slug)
        .bind(&input.body)
        .bind(input.format)
        .bind(input.rendered.html())
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
        // edit/clear — omitting the column from the SET clause silently drops an
        // edited summary (#545's clear e2e).
        .bind(input.summary.as_ref())
        .bind(post_id)
        .fetch_one(&mut *tx)
        .await?;

        crate::posts::replace_post_audiences::<Postgres>(&mut tx, post_id, &input.audiences)
            .await?;
        crate::posts::replace_post_media::<Postgres>(&mut tx, post_id, input.rendered.media())
            .await?;

        tx.commit().await?;
        post_record_from_row(row).map_err(UpdatePostError::Internal)
    }

    async fn set_post_tags(
        pool: &Pool<Postgres>,
        post_id: PostId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError> {
        let mut tx = pool.begin().await?;

        // FOR UPDATE locks the post row for the whole read-diff-write, so a
        // concurrent set_post_tags cannot interleave under READ COMMITTED
        // (ADR-0021; mirrors update_post). It doubles as the existence check.
        // No `deleted_at` filter: soft-deleted posts stay taggable.
        let exists = sqlx::query_scalar::<_, PostId>(
            "SELECT post_id FROM posts WHERE post_id = $1 FOR UPDATE",
        )
        .bind(post_id)
        .fetch_optional(&mut *tx)
        .await?;
        if exists.is_none() {
            tx.rollback().await.ok();
            return Err(TaggingError::PostNotFound);
        }

        let rows = sqlx::query_as::<_, (PostId, TagId, Tag, TagLabel)>(SELECT_POST_TAGS)
            .bind(post_id)
            .fetch_all(&mut *tx)
            .await?;
        let existing = post_tags_from_rows(rows);
        let diff = post_tag_diff(&existing, desired);

        for label in diff.to_add {
            let slug = label.slug();
            // `fetch_one`, not a read-back: the upsert's no-op `DO UPDATE`
            // returns the id on the conflict path too, so a no-row result
            // cannot occur (#883).
            let tag_id = sqlx::query_scalar::<_, TagId>(UPSERT_TAG_RETURNING_ID)
                .bind(&slug)
                .fetch_one(&mut *tx)
                .await?;
            sqlx::query(INSERT_POST_TAG)
                .bind(post_id)
                .bind(tag_id)
                .bind(label)
                .execute(&mut *tx)
                .await?;
        }

        for slug in diff.to_remove {
            // rows_affected is deliberately not checked: the slug came from
            // `existing`, read in this same transaction, so "no row deleted" is
            // not an error condition.
            sqlx::query(DELETE_POST_TAG_BY_SLUG)
                .bind(post_id)
                .bind(slug)
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        Ok(())
    }
}
