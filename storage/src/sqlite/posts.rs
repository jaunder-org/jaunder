use async_trait::async_trait;
use sqlx::{Pool, QueryBuilder, Sqlite};

use crate::helpers;
use crate::posts::{
    lifecycle::{self, PostBookkeepingRow},
    media::{self, MediaReferenceEvidence, PostMediaReferenceBackfill},
    models::PostPublicationClear,
    tags::{self, PostTag, PostTagDiff},
    visibility,
};
use crate::sql::{QueryBuilderStorageExt, QueryStorageExt};
use crate::{
    InstanceId, PostDialect, PostMutation, PostRecord, PostStore, PublishUpdate, RenderedHtml,
    TaggingError, UpdatePostError, UpdatePostInput, WriteTransaction, sqlite_connection,
};
use common::idempotency_key::IdempotencyKey;
use common::ids::{PostId, TagId, UserId};
use common::tag::TagLabel;
use common::time::UtcInstant;
type MediaRefRow = (
    common::media::MediaSource,
    common::media::ContentHash,
    common::media::Filename,
    common::media::MediaReferenceKind,
    common::media::MediaReferenceForm,
);

/// SQLite-backed post storage.
pub type SqlitePostStorage = PostStore<Sqlite>;

async fn fetch_post(
    conn: &mut sqlx::SqliteConnection,
    post_id: PostId,
) -> Result<PostRecord, sqlx::Error> {
    sqlx::query_as::<_, PostRecord>(
        "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format,
                p.rendered_html, p.created_at, p.updated_at, p.published_at, p.deleted_at,
                p.summary,
                COALESCE((
                    SELECT json_group_array(json_object(
                        'tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display
                    ) ORDER BY t.tag_slug)
                    FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id
                ), '[]') AS tags
         FROM posts p JOIN users u ON u.user_id = p.user_id WHERE p.post_id = $1",
    )
    .bind_storage(post_id)
    .fetch_one(&mut *conn)
    .await
}

async fn apply_post_update(
    conn: &mut sqlx::SqliteConnection,
    post_id: PostId,
    input: &UpdatePostInput,
    tag_diff: PostTagDiff<'_>,
) -> Result<(), UpdatePostError> {
    let now = input.request_clock;
    lifecycle::capture_complete_post_revision::<Sqlite>(conn, post_id, now).await?;
    let publication_clear = PostPublicationClear::for_update(input.publish);
    let explicit_published_at = match input.publish {
        PublishUpdate::Unpublish => None,
        PublishUpdate::Publish { at } => at,
    };
    sqlx::query(
        "UPDATE posts SET title = $1, slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END,
         body = $3, format = $4, rendered_html = $5,
         published_at = CASE WHEN $6 THEN NULL WHEN $7 IS NOT NULL THEN $8 ELSE COALESCE(published_at, $9) END,
         updated_at = $10, summary = $11 WHERE post_id = $12",
    )
    .bind_storage(input.title.as_ref()).bind_storage(&input.slug).bind_storage(&input.body).bind_storage(input.format)
    .bind_storage(input.rendered.html()).bind_storage(publication_clear).bind_storage(explicit_published_at)
    .bind_storage(explicit_published_at).bind_storage(now).bind_storage(now).bind_storage(input.summary.as_ref()).bind_storage(post_id)
    .execute(&mut *conn).await?;
    visibility::replace_post_audiences::<Sqlite>(&mut *conn, post_id, &input.audiences).await?;
    for label in tag_diff.to_add {
        let tag_id = sqlx::query_scalar::<_, TagId>(tags::UPSERT_TAG_RETURNING_ID)
            .bind_storage(label.slug())
            .fetch_one(&mut *conn)
            .await?;
        sqlx::query(tags::INSERT_POST_TAG)
            .bind_storage(post_id)
            .bind_storage(tag_id)
            .bind_storage(label)
            .execute(&mut *conn)
            .await?;
    }
    for slug in tag_diff.to_remove {
        sqlx::query(tags::DELETE_POST_TAG_BY_SLUG)
            .bind_storage(post_id)
            .bind_storage(slug)
            .execute(&mut *conn)
            .await?;
    }
    media::replace_post_media::<Sqlite>(&mut *conn, post_id, input.rendered.media()).await?;
    Ok(())
}

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

    const LIFECYCLE_STATE_SQL: &'static str =
        "SELECT user_id, deleted_at, published_at FROM posts WHERE post_id = $1";

    async fn fetch_lifecycle_post(
        conn: &mut <Self as sqlx::Database>::Connection,
        post_id: PostId,
    ) -> sqlx::Result<PostRecord> {
        fetch_post(conn, post_id).await
    }

    async fn lock_lifecycle_media_references(
        _conn: &mut <Self as sqlx::Database>::Connection,
        _post_id: PostId,
    ) -> sqlx::Result<()> {
        Ok(())
    }

    async fn lock_media_references(
        _conn: &mut <Self as sqlx::Database>::Connection,
        _media: &std::collections::BTreeSet<common::media::MediaRef>,
    ) -> sqlx::Result<()> {
        Ok(())
    }

    async fn lock_live_idempotency_mapping(
        conn: &mut <Self as sqlx::Database>::Connection,
        user_id: UserId,
        key: &IdempotencyKey,
        cutoff: UtcInstant,
    ) -> sqlx::Result<Option<PostId>> {
        // WriteScope starts SQLite mutations with BEGIN IMMEDIATE, so the
        // database writer lock serializes both present and absent mappings.
        sqlx::query_scalar(
            "SELECT post_id FROM idempotency_keys
             WHERE user_id = $1 AND key = $2 AND created_at > $3",
        )
        .bind_storage(user_id)
        .bind_storage(key)
        .bind_storage(cutoff)
        .fetch_optional(&mut *conn)
        .await
    }

    const DELETE_POST_MEDIA: &'static str =
        "DELETE FROM post_media WHERE post_id = ? AND subject_kind = 'current' AND revision_id = 0";

    async fn update_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostMutation, UpdatePostError> {
        let conn = sqlite_connection(transaction)?;
        let existing = sqlx::query_as::<_, PostBookkeepingRow>(
            "SELECT user_id, deleted_at, title, slug, body, format, rendered_html, summary, published_at
             FROM posts WHERE post_id = $1",
        )
        .bind_storage(post_id)
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
        .bind_storage(post_id)
        .fetch_all(&mut *conn)
        .await?;
        let desired_tags = input.tags.as_deref().unwrap_or(&tags);
        if let Some(error) = lifecycle::update_expectation_error(post_id, &existing, &tags, input) {
            return Err(error);
        }
        let previous = fetch_post(conn, post_id).await?;
        let existing_tags = previous.tags.clone();
        let tag_diff = tags::post_tag_diff(&existing_tags, desired_tags);
        let existing_audiences = sqlx::query_as::<
            _,
            (
                common::visibility::TargetKind,
                Option<common::ids::AudienceId>,
            ),
        >(
            "SELECT tk.name, pa.audience_id FROM post_audiences pa
             JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id
             WHERE pa.post_id = $1",
        )
        .bind_storage(post_id)
        .fetch_all(&mut *conn)
        .await?;
        let previous_has_public_audience = existing_audiences
            .iter()
            .any(|(kind, _)| matches!(kind, common::visibility::TargetKind::Public));
        let old_media: Vec<MediaRefRow> = sqlx::query_as(
            "SELECT source, sha256, filename, reference_kind, reference_form FROM post_media
             WHERE post_id = $1 AND subject_kind = 'current' AND revision_id = 0",
        )
        .bind_storage(post_id)
        .fetch_all(&mut *conn)
        .await?;
        let old_media_set: std::collections::BTreeSet<_> = old_media.iter().cloned().collect();
        let desired_media_set: std::collections::BTreeSet<_> = input
            .rendered
            .media()
            .iter()
            .map(|reference| {
                (
                    reference.media().source,
                    reference.media().sha256.clone(),
                    reference.media().filename.clone(),
                    reference.kind(),
                    reference.reference_form().clone(),
                )
            })
            .collect();
        if lifecycle::update_scalar_is_noop(&existing, input)
            && tag_diff.to_add.is_empty()
            && tag_diff.to_remove.is_empty()
            && visibility::audiences_are_equal(&existing_audiences, &input.audiences)
            && old_media_set == desired_media_set
        {
            return Ok(PostMutation {
                record: previous.clone(),
                previous,
                previous_has_public_audience,
                changed: false,
            });
        }
        apply_post_update(conn, post_id, input, tag_diff).await?;
        let record = fetch_post(conn, post_id).await?;
        Ok(PostMutation {
            record,
            previous,
            previous_has_public_audience,
            changed: true,
        })
    }

    async fn publish_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
        now: UtcInstant,
    ) -> Result<Option<PostMutation>, sqlx::Error> {
        lifecycle::lifecycle_post::<Self>(
            transaction,
            post_id,
            user_id,
            lifecycle::PostLifecycleChange::Publish,
            now,
        )
        .await
    }

    async fn soft_delete_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
        now: UtcInstant,
    ) -> Result<Option<PostMutation>, sqlx::Error> {
        lifecycle::lifecycle_post::<Self>(
            transaction,
            post_id,
            user_id,
            lifecycle::PostLifecycleChange::SoftDelete,
            now,
        )
        .await
    }

    async fn unpublish_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
        now: UtcInstant,
    ) -> Result<Option<PostMutation>, sqlx::Error> {
        lifecycle::lifecycle_post::<Self>(
            transaction,
            post_id,
            user_id,
            lifecycle::PostLifecycleChange::Unpublish,
            now,
        )
        .await
    }

    async fn set_post_tags(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError> {
        let connection = sqlite_connection(transaction)?;
        let post = sqlx::query_as::<_, (UserId, Option<common::time::UtcInstant>)>(
            "SELECT user_id, deleted_at FROM posts WHERE post_id = $1",
        )
        .bind_storage(post_id)
        .fetch_optional(&mut *connection)
        .await?;
        match post {
            None | Some((_, Some(_))) => return Err(TaggingError::PostNotFound),
            Some((owner, None)) if owner != user_id => return Err(TaggingError::Unauthorized),
            Some(_) => {}
        }
        let existing = sqlx::query_as::<_, PostTag>(tags::SELECT_POST_TAGS)
            .bind_storage(post_id)
            .fetch_all(&mut *connection)
            .await?;
        let diff = tags::post_tag_diff(&existing, desired);
        if diff.to_add.is_empty() && diff.to_remove.is_empty() {
            return Ok(());
        }
        lifecycle::capture_complete_post_revision::<Sqlite>(
            &mut *connection,
            post_id,
            UtcInstant::now(),
        )
        .await?;
        for label in diff.to_add {
            let tag_id = sqlx::query_scalar::<_, TagId>(tags::UPSERT_TAG_RETURNING_ID)
                .bind_storage(label.slug())
                .fetch_one(&mut *connection)
                .await?;
            sqlx::query(tags::INSERT_POST_TAG)
                .bind_storage(post_id)
                .bind_storage(tag_id)
                .bind_storage(label)
                .execute(&mut *connection)
                .await?;
        }
        for slug in diff.to_remove {
            sqlx::query(tags::DELETE_POST_TAG_BY_SLUG)
                .bind_storage(post_id)
                .bind_storage(slug)
                .execute(&mut *connection)
                .await?;
        }
        Ok(())
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
            media::replace_legacy_post_media::<Sqlite>(&mut conn, candidates).await
        }
        .await;

        match result {
            Ok(()) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
                Ok(())
            }
            Err(error) => helpers::preserve_after_secondary(
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
                .push_storage_bind(post_id)
                .push_storage_bind(media.source)
                .push_storage_bind(media.sha256)
                .push_storage_bind(media.filename)
                .push_storage_bind(kind)
                .push_storage_bind(form);
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
        media::push_media_reference_evidence_cte(&mut query, evidence);
        query.push("SELECT DISTINCT pm.post_id");
        media::push_owner_media_reference_from_where(&mut query, user_id, media);
        media::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(" ORDER BY pm.post_id");
        query.build_query_scalar::<PostId>().fetch_all(pool).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posts::media::PostMediaReferenceBackfill;
    use crate::test_support::{Backend, CloseablePool, SeedRawPost, SeedUser, sqlite_only};
    use rstest::*;
    use rstest_reuse::*;

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
