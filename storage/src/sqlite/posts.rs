use async_trait::async_trait;
use sqlx::{Pool, QueryBuilder, Sqlite, SqliteConnection};

use crate::posts::{
    DELETE_POST_TAG_BY_SLUG, INSERT_POST_TAG, MediaReferenceEvidence, PostBookkeepingRow,
    PostMediaReferenceBackfill, PostOwnershipRow, PostTagRow, SELECT_POST_TAGS,
    UPSERT_TAG_RETURNING_ID, post_tag_diff, post_tags_from_rows,
    push_live_media_reference_predicate, push_media_reference_evidence_cte,
    push_owner_media_reference_from_where, replace_legacy_post_media, update_expectation_error,
};
use crate::{
    InstanceId, PostDialect, PostRecord, PostStore, PublishUpdate, RenderedHtml, TaggingError,
    UpdatePostError, UpdatePostInput,
};
use common::ids::{PostId, TagId, UserId};
use common::tag::TagLabel;
use common::time::UtcInstant;

pub(crate) fn finish_post_update(
    primary: Result<PostRecord, UpdatePostError>,
    rollback: Result<(), sqlx::Error>,
) -> Result<PostRecord, UpdatePostError> {
    crate::helpers::preserve_after_secondary(
        primary,
        rollback,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        "storage.sqlite.post_update.rollback",
    )
}

pub(crate) fn finish_post_tags(
    primary: Result<(), TaggingError>,
    rollback: Result<(), sqlx::Error>,
) -> Result<(), TaggingError> {
    crate::helpers::preserve_after_secondary(
        primary,
        rollback,
        host::error::ErrorKind::Storage,
        host::error::ErrorClass::Transient,
        "storage.sqlite.post_tags.rollback",
    )
}
async fn lock_updated_media(
    conn: &mut SqliteConnection,
    post_id: PostId,
    input: &UpdatePostInput,
) -> sqlx::Result<()> {
    let old_media: Vec<(
        common::media::MediaSource,
        common::media::ContentHash,
        common::media::Filename,
    )> = sqlx::query_as("SELECT source, sha256, filename FROM post_media WHERE post_id = $1")
        .bind(post_id)
        .fetch_all(&mut *conn)
        .await?;
    let mut locked_media = crate::posts::media_lock_set(input.rendered.media());
    locked_media.extend(old_media.into_iter().map(|(source, sha256, filename)| {
        common::media::MediaRef {
            source,
            sha256,
            filename,
        }
    }));
    <Sqlite as PostDialect>::lock_media_references(conn, &locked_media).await
}

/// SQLite-backed post storage.
pub type SqlitePostStorage = PostStore<Sqlite>;

#[async_trait]
impl PostDialect for Sqlite {
    /// `ORDER BY t.tag_slug` is what makes [`PostRecord::tags`] slug-ordered
    /// (#772); `SQLite`'s default BINARY collation is already byte order, so no
    /// `COLLATE` is needed here. See [`PostDialect::TAGS_SUBQUERY`] for why the
    /// Postgres twin does need one, and keep the two in sync.
    const TAGS_SUBQUERY: &'static str = "COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display) ORDER BY t.tag_slug) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]')";

    const PERMALINK_DATE_CLAUSE: &'static str = "date(COALESCE(p.published_at, p.created_at)) = $3";

    const DELETE_POST_AUDIENCES: &'static str = "DELETE FROM post_audiences WHERE post_id = ?";

    // Bind order: post_id, audience_id, kind_name (matches `replace_post_audiences`).
    const INSERT_POST_AUDIENCE: &'static str = "INSERT INTO post_audiences \
         (post_id, audience_id, target_kind_id) \
         VALUES (?, ?, (SELECT kind_id FROM target_kinds WHERE name = ?))";

    async fn lock_media_references(
        _conn: &mut <Self as sqlx::Database>::Connection,
        _media: &std::collections::BTreeSet<common::media::MediaRef>,
    ) -> sqlx::Result<()> {
        Ok(())
    }

    const DELETE_POST_MEDIA: &'static str = "DELETE FROM post_media WHERE post_id = ?";

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
        let now = input.request_clock;

        let result: Result<PostRecord, UpdatePostError> = async {
            let existing = sqlx::query_as::<_, PostBookkeepingRow>(
                "SELECT user_id, deleted_at, title, slug, body, format, summary, published_at
                 FROM posts WHERE post_id = $1",
            )
            .bind(post_id)
            .fetch_optional(&mut *conn)
            .await?;

            let existing = match existing {
                None => return Err(UpdatePostError::NotFound),
                Some(existing)
                    if existing.user_id != editor_user_id || existing.deleted_at.is_some() =>
                {
                    return Err(UpdatePostError::Unauthorized);
                }
                Some(existing) => existing,
            };
            let tags = sqlx::query_scalar::<_, TagLabel>(
                "SELECT pt.tag_display FROM post_tags pt
                 JOIN tags t ON t.tag_id = pt.tag_id
                 WHERE pt.post_id = $1 ORDER BY t.tag_slug",
            )
            .bind(post_id)
            .fetch_all(&mut *conn)
            .await?;
            if let Some(error) = update_expectation_error(post_id, &existing, &tags, input) {
                return Err(error);
            }
            lock_updated_media(&mut conn, post_id, input).await?;
            sqlx::query(
                "INSERT INTO post_revisions (post_id, user_id, title, slug, body, format, rendered_html, edited_at)
                 SELECT post_id, user_id, title, slug, body, format, rendered_html, $1
                 FROM posts WHERE post_id = $2",
            )
            .bind(now)
            .bind(post_id)
            .execute(&mut *conn)
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
                           COALESCE((SELECT json_group_array(json_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]') AS tags",
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
            .fetch_one(&mut *conn)
            .await?;

            crate::posts::replace_post_audiences::<Sqlite>(&mut *conn, post_id, &input.audiences)
                .await?;
            crate::posts::replace_post_media::<Sqlite>(&mut *conn, post_id, input.rendered.media())
                .await?;

            Ok(row)
        }
        .await;

        match result {
            Ok(row) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(row)
            }
            Err(error) => finish_post_update(
                Err(error),
                sqlx::query("ROLLBACK")
                    .execute(&mut *conn)
                    .await
                    .map(|_| ()),
            ),
        }
    }

    async fn set_post_tags(
        pool: &Pool<Sqlite>,
        post_id: PostId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError> {
        // ADR-0021: BEGIN IMMEDIATE takes the write lock up front, so the read
        // below is not a shared->reserved upgrade — and the whole read-diff-write
        // is serialized under one acquisition (ADR-0092), closing the TOCTOU a
        // separate autocommit read would leave open. sqlx's Transaction issues
        // its own deferred BEGIN, so drive the transaction manually on a raw
        // connection, mirroring update_post / create_user_with_invite.
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let result: Result<(), TaggingError> = async {
            // No `deleted_at` filter: soft-deleted posts stay taggable.
            let post_exists: bool =
                sqlx::query_scalar("SELECT COUNT(*) > 0 FROM posts WHERE post_id = $1")
                    .bind(post_id)
                    .fetch_one(&mut *conn)
                    .await?;
            if !post_exists {
                return Err(TaggingError::PostNotFound);
            }

            let rows = sqlx::query_as::<_, PostTagRow>(SELECT_POST_TAGS)
                .bind(post_id)
                .fetch_all(&mut *conn)
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
                    .fetch_one(&mut *conn)
                    .await?;
                sqlx::query(INSERT_POST_TAG)
                    .bind(post_id)
                    .bind(tag_id)
                    .bind(label)
                    .execute(&mut *conn)
                    .await?;
            }

            for slug in diff.to_remove {
                // rows_affected is deliberately not checked: the slug came from
                // `existing`, read in this same transaction, so "no row deleted"
                // is not an error condition.
                sqlx::query(DELETE_POST_TAG_BY_SLUG)
                    .bind(post_id)
                    .bind(slug)
                    .execute(&mut *conn)
                    .await?;
            }
            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => finish_post_tags(
                Err(error),
                sqlx::query("ROLLBACK")
                    .execute(&mut *conn)
                    .await
                    .map(|_| ()),
            ),
        }
    }

    async fn apply_post_media_reference_backfill(
        pool: &Pool<Self>,
        candidates: &[PostMediaReferenceBackfill],
    ) -> sqlx::Result<()> {
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        let result: sqlx::Result<()> = async {
            let current: Vec<(PostId, RenderedHtml)> = sqlx::query_as(
                "SELECT p.post_id, p.rendered_html
                 FROM posts p
                 WHERE EXISTS (
                     SELECT 1 FROM post_media pm
                     WHERE pm.post_id = p.post_id AND pm.reference_kind = 'legacy'
                 )
                 ORDER BY p.post_id",
            )
            .fetch_all(&mut *conn)
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
                return Err(sqlx::Error::Protocol(
                    "post rendered HTML changed during media-reference backfill".to_owned(),
                ));
            }
            replace_legacy_post_media::<Sqlite>(&mut conn, candidates).await
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => crate::helpers::preserve_after_secondary(
                Err(error),
                sqlx::query("ROLLBACK")
                    .execute(&mut *conn)
                    .await
                    .map(|_| ()),
                host::error::ErrorKind::Storage,
                host::error::ErrorClass::Transient,
                "storage.sqlite.post_media_reference_backfill.rollback",
            ),
        }
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
        let mut query = QueryBuilder::<Sqlite>::new(
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
        let mut query = QueryBuilder::<Sqlite>::new(String::new());
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
    use crate::posts::PostMediaReferenceBackfill;
    use crate::test_support::{Backend, CloseablePool, SeedRawPost, SeedUser, sqlite_only};
    use rstest::*;
    use rstest_reuse::*;

    #[test]
    fn continuation_reporting_rollback_failures_preserve_post_domain_errors_and_report_once() {
        let (update, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_post_update(
                Err(UpdatePostError::Unauthorized),
                Err(sqlx::Error::PoolClosed),
            )
        });
        assert!(matches!(update, Err(UpdatePostError::Unauthorized)));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.sqlite.post_update.rollback",
        );

        let (tagging, trace) = crate::helpers::swallowed_test::capture(|| {
            finish_post_tags(
                Err(TaggingError::PostNotFound),
                Err(sqlx::Error::PoolClosed),
            )
        });
        assert!(matches!(tagging, Err(TaggingError::PostNotFound)));
        crate::helpers::swallowed_test::assert_one_report(
            &trace,
            "storage.sqlite.post_tags.rollback",
        );
    }

    // reason: SQLite's immediate writer transaction is the dialect-specific snapshot guard.
    #[apply(sqlite_only)]
    #[tokio::test]
    async fn media_backfill_rejects_a_stale_rendered_html_snapshot(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post_id = SeedRawPost::new(user).seed(&env.state).await.post_id;
        env.base
            .pool()
            .execute(&format!(
                "INSERT INTO post_media \
                 (post_id, source, sha256, filename, reference_kind, reference_form) \
                 VALUES ({post_id}, 'upload', \
                 '0000000000000000000000000000000000000000000000000000000000000000', \
                 'snapshot.jpg', 'legacy', 'legacy')"
            ))
            .await
            .expect("seed legacy reference row");
        let CloseablePool::Sqlite(pool) = env.base.pool() else {
            unreachable!("SQLite setup yields a SQLite pool")
        };
        let error = <Sqlite as PostDialect>::apply_post_media_reference_backfill(
            pool,
            &[PostMediaReferenceBackfill {
                post_id,
                rendered_html: "stale HTML".to_owned(),
                references: Vec::new(),
            }],
        )
        .await
        .expect_err("a changed snapshot must not rewrite derived references");

        assert!(matches!(error, sqlx::Error::Protocol(_)));
    }
}
