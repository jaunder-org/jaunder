use async_trait::async_trait;
use sqlx::{Pool, Postgres, QueryBuilder};

use crate::posts::{
    DELETE_POST_TAG_BY_SLUG, INSERT_POST_TAG, MediaReferenceEvidence, PostBookkeepingRow,
    PostMediaReferenceBackfill, PostTagRow, SELECT_POST_TAGS, UPSERT_TAG_RETURNING_ID,
    media_advisory_lock_keys, media_lock_set, post_tag_diff, post_tags_from_rows,
    push_live_media_reference_predicate, push_media_reference_evidence_cte,
    push_owner_media_reference_from_where, replace_legacy_post_media, update_expectation_error,
};
use crate::{
    InstanceId, PostDialect, PostRecord, PostStore, PublishUpdate, RenderedHtml, TaggingError,
    UpdatePostError, UpdatePostInput,
};
use common::ids::{PostId, TagId, UserId};
use common::tag::TagLabel;

type MediaRefRow = (
    common::media::MediaSource,
    common::media::ContentHash,
    common::media::Filename,
);

pub(crate) fn finish_post_update_rejection(
    primary: Result<PostRecord, UpdatePostError>,
    rollback: Result<(), sqlx::Error>,
) -> Result<PostRecord, UpdatePostError> {
    crate::helpers::preserve_after_secondary(
        primary,
        rollback,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        "storage.postgres.post_update.rollback_domain_rejection",
    )
}

async fn locked_update_expectation_error(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    post_id: PostId,
    existing: &PostBookkeepingRow,
    input: &UpdatePostInput,
) -> Result<Option<UpdatePostError>, sqlx::Error> {
    let tags = sqlx::query_scalar::<_, TagLabel>(
        "SELECT pt.tag_display FROM post_tags pt \
         JOIN tags t ON t.tag_id = pt.tag_id \
         WHERE pt.post_id = $1 ORDER BY t.tag_slug COLLATE \"C\"",
    )
    .bind(post_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(update_expectation_error(post_id, existing, &tags, input))
}

pub(crate) fn finish_post_tags_not_found(
    primary: Result<(), TaggingError>,
    rollback: Result<(), sqlx::Error>,
) -> Result<(), TaggingError> {
    crate::helpers::preserve_after_secondary(
        primary,
        rollback,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        "storage.postgres.post_tags.rollback_not_found",
    )
}

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

    async fn lock_media_references(
        conn: &mut <Self as sqlx::Database>::Connection,
        media: &std::collections::BTreeSet<common::media::MediaRef>,
    ) -> sqlx::Result<()> {
        for key in media_advisory_lock_keys(media.iter().cloned()) {
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(key)
                .execute(&mut *conn)
                .await?;
        }
        Ok(())
    }

    const DELETE_POST_MEDIA: &'static str = "DELETE FROM post_media WHERE post_id = $1";

    async fn update_post(
        pool: &Pool<Postgres>,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        let mut tx = pool.begin().await?;
        let now = input.request_clock;

        // FOR UPDATE locks the row for the read-then-write: it stops a concurrent
        // edit from slipping between this ownership/liveness check and the UPDATE
        // below (ADR-0021 / #52). SQLite needs no equivalent — its transaction
        // already serializes writers.
        let existing = sqlx::query_as::<_, PostBookkeepingRow>(
            "SELECT user_id, deleted_at, title, slug, body, format, summary, published_at
             FROM posts WHERE post_id = $1 FOR UPDATE",
        )
        .bind(post_id)
        .fetch_optional(&mut *tx)
        .await?;

        match existing {
            None => {
                return finish_post_update_rejection(
                    Err(UpdatePostError::NotFound),
                    tx.rollback().await,
                );
            }
            Some(existing)
                if existing.user_id != editor_user_id || existing.deleted_at.is_some() =>
            {
                return finish_post_update_rejection(
                    Err(UpdatePostError::Unauthorized),
                    tx.rollback().await,
                );
            }
            Some(existing) => {
                if let Some(error) =
                    locked_update_expectation_error(&mut tx, post_id, &existing, input).await?
                {
                    return finish_post_update_rejection(Err(error), tx.rollback().await);
                }
            }
        }
        let old_media: Vec<MediaRefRow> =
            sqlx::query_as("SELECT source, sha256, filename FROM post_media WHERE post_id = $1")
                .bind(post_id)
                .fetch_all(&mut *tx)
                .await?;
        let mut locked_media = media_lock_set(input.rendered.media());
        locked_media.extend(old_media.into_iter().map(|(source, sha256, filename)| {
            common::media::MediaRef {
                source,
                sha256,
                filename,
            }
        }));
        Self::lock_media_references(&mut *tx, &locked_media).await?;

        sqlx::query(
            "INSERT INTO post_revisions (post_id, user_id, title, slug, body, format, rendered_html, edited_at)
             SELECT post_id, user_id, title, slug, body, format, rendered_html, $1
             FROM posts WHERE post_id = $2",
        )
        .bind(now)
        .bind(post_id)
        .execute(&mut *tx)
        .await?;
        let (unpublish, explicit_published_at) = match input.publish {
            PublishUpdate::Unpublish => (true, None),
            PublishUpdate::Publish { at } => (false, at),
        };

        let row = sqlx::query_as::<_, PostRecord>(
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
        .bind(unpublish)
        .bind(explicit_published_at)
        .bind(explicit_published_at)
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
        Ok(row)
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
            return finish_post_tags_not_found(
                Err(TaggingError::PostNotFound),
                tx.rollback().await,
            );
        }

        let rows = sqlx::query_as::<_, PostTagRow>(SELECT_POST_TAGS)
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

    async fn apply_post_media_reference_backfill(
        pool: &Pool<Self>,
        candidates: &[PostMediaReferenceBackfill],
    ) -> sqlx::Result<()> {
        let mut tx = pool.begin().await?;
        let current: Vec<(PostId, RenderedHtml)> = sqlx::query_as(
            "SELECT p.post_id, p.rendered_html
             FROM posts p
             WHERE EXISTS (
                 SELECT 1 FROM post_media pm
                 WHERE pm.post_id = p.post_id AND pm.reference_kind = 'legacy'
             )
             ORDER BY p.post_id
             FOR UPDATE",
        )
        .fetch_all(&mut *tx)
        .await?;
        let unchanged = current.len() == candidates.len()
            && current
                .iter()
                .zip(candidates)
                .all(|((post_id, html), candidate)| {
                    *post_id == candidate.post_id
                        && html.as_ref() == candidate.rendered_html.as_str()
                });
        if !unchanged {
            return crate::helpers::preserve_after_secondary(
                Err(sqlx::Error::Protocol(
                    "post rendered HTML changed during media-reference backfill".to_owned(),
                )),
                tx.rollback().await,
                host::error::ErrorKind::Storage,
                host::error::ErrorClass::Transient,
                "storage.postgres.post_media_reference_backfill.rollback",
            );
        }
        replace_legacy_post_media::<Postgres>(&mut tx, candidates).await?;
        tx.commit().await
    }

    async fn insert_post_media_rows(
        conn: &mut Self::Connection,
        rows: std::collections::BTreeSet<(
            PostId,
            common::media::MediaRef,
            common::media::MediaReferenceKind,
            common::media::MediaReferenceForm,
        )>,
    ) -> sqlx::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut query = QueryBuilder::<Postgres>::new(
            "INSERT INTO post_media (post_id, source, sha256, filename, reference_kind, reference_form) ",
        );
        query.push_values(rows, |mut values, (post_id, media, kind, form)| {
            values
                .push_bind(post_id)
                .push_bind(media.source)
                .push_bind(media.sha256)
                .push_bind(media.filename)
                .push_bind(kind.to_string())
                .push_bind(form);
        });
        query.build().execute(&mut *conn).await?;
        Ok(())
    }

    async fn list_posts_referencing_media(
        pool: &Pool<Self>,
        user_id: UserId,
        media: &common::media::MediaRef,
        current_instance_id: &InstanceId,
        evidence: &MediaReferenceEvidence,
    ) -> sqlx::Result<Vec<PostId>> {
        let mut query = QueryBuilder::<Postgres>::new(String::new());
        push_media_reference_evidence_cte(&mut query, evidence);
        query.push("SELECT DISTINCT pm.post_id");
        push_owner_media_reference_from_where(&mut query, user_id, media);
        push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(" ORDER BY pm.post_id");
        query.build_query_scalar::<PostId>().fetch_all(pool).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_reporting_rollback_failures_preserve_post_domain_rejections_and_report_once() {
        for primary in [
            UpdatePostError::NotFound,
            UpdatePostError::Unauthorized,
            UpdatePostError::BookkeepingMismatch,
            UpdatePostError::StaleContent,
        ] {
            let expected = match primary {
                UpdatePostError::NotFound => "not-found",
                UpdatePostError::Unauthorized => "unauthorized",
                UpdatePostError::BookkeepingMismatch => "bookkeeping-mismatch",
                UpdatePostError::StaleContent => "stale-content",
                UpdatePostError::Internal(_) => unreachable!("test variants are domain errors"),
            };
            let (result, trace) = crate::helpers::swallowed_test::capture(|| {
                finish_post_update_rejection(Err(primary), Err(sqlx::Error::PoolClosed))
            });
            assert!(
                matches!(
                    (&result, expected),
                    (Err(UpdatePostError::NotFound), "not-found")
                        | (Err(UpdatePostError::Unauthorized), "unauthorized")
                        | (
                            Err(UpdatePostError::BookkeepingMismatch),
                            "bookkeeping-mismatch"
                        )
                        | (Err(UpdatePostError::StaleContent), "stale-content")
                ),
                "primary variant changed: {result:?}"
            );
            crate::helpers::swallowed_test::assert_one_report(
                &trace,
                "storage.postgres.post_update.rollback_domain_rejection",
            );
        }

        let (result, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_post_tags_not_found(
                Err(TaggingError::PostNotFound),
                Err(sqlx::Error::PoolClosed),
            )
        });
        assert!(matches!(result, Err(TaggingError::PostNotFound)));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.postgres.post_tags.rollback_not_found",
        );
    }
}
