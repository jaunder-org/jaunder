use async_trait::async_trait;
use sqlx::{Pool, Postgres, QueryBuilder};

use crate::helpers;
use crate::posts::{
    lifecycle::{self, PostBookkeepingRow},
    media::{self, MediaReferenceEvidence, PostMediaReferenceBackfill},
    models::PostPublicationClear,
    tags::{self, PostTag, PostTagDiff},
    visibility,
};
use crate::sql::{Exists, QueryBuilderStorageExt, QueryStorageExt};
use crate::{
    InstanceId, PostDialect, PostMutation, PostRecord, PostStore, PublishUpdate, RenderedHtml,
    TaggingError, UpdatePostError, UpdatePostInput, WriteTransaction, postgres_connection,
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

async fn locked_update_expectation_error(
    connection: &mut sqlx::PgConnection,
    post_id: PostId,
    existing: &PostBookkeepingRow,
    input: &UpdatePostInput,
) -> Result<Option<UpdatePostError>, sqlx::Error> {
    let tags = sqlx::query_scalar::<_, TagLabel>(
        "SELECT pt.tag_display FROM post_tags pt \
         JOIN tags t ON t.tag_id = pt.tag_id \
         WHERE pt.post_id = $1 ORDER BY t.tag_slug COLLATE \"C\"",
    )
    .bind_storage(post_id)
    .fetch_all(&mut *connection)
    .await?;
    Ok(lifecycle::update_expectation_error(
        post_id, existing, &tags, input,
    ))
}

/// Postgres-backed post storage.
pub type PostgresPostStorage = PostStore<Postgres>;

/// Loads the exact current-subject media identities whose revision copies must
/// serialize with media deletion and reclamation (ADR-0154).
async fn load_current_post_media_lock_set(
    connection: &mut sqlx::PgConnection,
    post_id: PostId,
) -> Result<std::collections::BTreeSet<common::media::MediaRef>, sqlx::Error> {
    Ok(sqlx::query_as::<
        _,
        (
            common::media::MediaSource,
            common::media::ContentHash,
            common::media::Filename,
        ),
    >(
        "SELECT source, sha256, filename FROM post_media
         WHERE post_id = $1 AND subject_kind = 'current' AND revision_id = 0",
    )
    .bind_storage(post_id)
    .fetch_all(connection)
    .await?
    .into_iter()
    .map(|(source, sha256, filename)| common::media::MediaRef {
        source,
        sha256,
        filename,
    })
    .collect())
}

async fn apply_lifecycle_change(
    connection: &mut sqlx::PgConnection,
    post_id: PostId,
    publish: bool,
    delete: bool,
    now: UtcInstant,
) -> sqlx::Result<()> {
    let media = load_current_post_media_lock_set(&mut *connection, post_id).await?;
    <Postgres as PostDialect>::lock_media_references(connection, &media).await?;
    lifecycle::capture_complete_post_revision::<Postgres>(connection, post_id, now).await?;
    if delete {
        sqlx::query("UPDATE posts SET deleted_at = $1 WHERE post_id = $2")
            .bind_storage(now)
            .bind_storage(post_id)
            .execute(&mut *connection)
            .await?;
    } else if publish {
        sqlx::query("UPDATE posts SET published_at = $1, updated_at = $1 WHERE post_id = $2")
            .bind_storage(now)
            .bind_storage(post_id)
            .execute(&mut *connection)
            .await?;
    } else {
        sqlx::query("UPDATE posts SET published_at = NULL, updated_at = $1 WHERE post_id = $2")
            .bind_storage(now)
            .bind_storage(post_id)
            .execute(&mut *connection)
            .await?;
    }
    Ok(())
}

async fn lifecycle_post(
    transaction: &mut WriteTransaction,
    post_id: PostId,
    user_id: UserId,
    publish: bool,
    delete: bool,
    now: UtcInstant,
) -> Result<Option<PostMutation>, sqlx::Error> {
    let connection = postgres_connection(transaction)?;
    let state = sqlx::query_as::<
        _,
        (
            UserId,
            Option<common::time::UtcInstant>,
            Option<common::time::UtcInstant>,
        ),
    >(
        "SELECT user_id, deleted_at, published_at FROM posts WHERE post_id = $1 FOR UPDATE",
    )
    .bind_storage(post_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some((owner, deleted_at, published_at)) = state else {
        return Ok(None);
    };
    if owner != user_id || deleted_at.is_some() {
        return Ok(None);
    }
    let previous = fetch_post(connection, post_id).await?;
    let previous_has_public_audience = sqlx::query_scalar::<_, Exists>(
        "SELECT EXISTS(
            SELECT 1 FROM post_audiences pa
            JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id
            WHERE pa.post_id = $1 AND tk.name = 'public'
        )",
    )
    .bind_storage(post_id)
    .fetch_one(&mut *connection)
    .await?
    .into_bool();
    let changed =
        delete || (publish && published_at.is_none()) || (!publish && published_at.is_some());
    if changed {
        apply_lifecycle_change(connection, post_id, publish, delete, now).await?;
    }
    let record = fetch_post(connection, post_id).await?;
    Ok(Some(PostMutation {
        record,
        previous,
        previous_has_public_audience,
        changed,
    }))
}

async fn fetch_post(
    connection: &mut sqlx::PgConnection,
    post_id: PostId,
) -> Result<PostRecord, sqlx::Error> {
    sqlx::query_as::<_, PostRecord>(
        "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format,
                p.rendered_html, p.created_at, p.updated_at, p.published_at, p.deleted_at,
                p.summary,
                COALESCE((SELECT json_agg(json_build_object(
                    'tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display
                ) ORDER BY t.tag_slug COLLATE \"C\") FROM post_tags pt
                JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id),
                '[]'::json)::text AS tags
         FROM posts p JOIN users u ON u.user_id = p.user_id WHERE p.post_id = $1",
    )
    .bind_storage(post_id)
    .fetch_one(&mut *connection)
    .await
}

struct PostUpdateRelations {
    existing_tags: Vec<PostTag>,
    existing_audiences: Vec<(
        common::visibility::TargetKind,
        Option<common::ids::AudienceId>,
    )>,
    old_media: Vec<MediaRefRow>,
    desired_media: std::collections::BTreeSet<MediaRefRow>,
}

async fn load_post_update_relations(
    tx: &mut sqlx::PgConnection,
    post_id: PostId,
    input: &UpdatePostInput,
) -> sqlx::Result<PostUpdateRelations> {
    let existing_tags = sqlx::query_as::<_, PostTag>(tags::SELECT_POST_TAGS)
        .bind_storage(post_id)
        .fetch_all(&mut *tx)
        .await?;
    let existing_audiences = sqlx::query_as::<
        _,
        (
            common::visibility::TargetKind,
            Option<common::ids::AudienceId>,
        ),
    >(
        "SELECT tk.name, pa.audience_id FROM post_audiences pa
         JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id WHERE pa.post_id = $1",
    )
    .bind_storage(post_id)
    .fetch_all(&mut *tx)
    .await?;
    let old_media = sqlx::query_as::<_, MediaRefRow>(
        "SELECT source, sha256, filename, reference_kind, reference_form FROM post_media
         WHERE post_id = $1 AND subject_kind = 'current' AND revision_id = 0",
    )
    .bind_storage(post_id)
    .fetch_all(&mut *tx)
    .await?;
    let desired_media = input
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
    Ok(PostUpdateRelations {
        existing_tags,
        existing_audiences,
        old_media,
        desired_media,
    })
}

async fn apply_post_update(
    tx: &mut sqlx::PgConnection,
    post_id: PostId,
    input: &UpdatePostInput,
    tag_diff: PostTagDiff<'_>,
) -> Result<(), UpdatePostError> {
    let now = input.request_clock;
    lifecycle::capture_complete_post_revision::<Postgres>(tx, post_id, now).await?;
    let publication_clear = PostPublicationClear::for_update(input.publish);
    let explicit_published_at = match input.publish {
        PublishUpdate::Unpublish => None,
        PublishUpdate::Publish { at } => at,
    };
    sqlx::query(
        "UPDATE posts
         SET title = $1, slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END,
             body = $3, format = $4, rendered_html = $5,
             published_at = CASE WHEN $6 THEN NULL WHEN $7 IS NOT NULL THEN $8
                 ELSE COALESCE(published_at, $9) END,
             updated_at = $10, summary = $11
         WHERE post_id = $12",
    )
    .bind_storage(input.title.as_ref())
    .bind_storage(&input.slug)
    .bind_storage(&input.body)
    .bind_storage(input.format)
    .bind_storage(input.rendered.html())
    .bind_storage(publication_clear)
    .bind_storage(explicit_published_at)
    .bind_storage(explicit_published_at)
    .bind_storage(now)
    .bind_storage(now)
    .bind_storage(input.summary.as_ref())
    .bind_storage(post_id)
    .execute(&mut *tx)
    .await?;
    visibility::replace_post_audiences::<Postgres>(tx, post_id, &input.audiences).await?;
    for label in tag_diff.to_add {
        let tag_id = sqlx::query_scalar::<_, TagId>(tags::UPSERT_TAG_RETURNING_ID)
            .bind_storage(label.slug())
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(tags::INSERT_POST_TAG)
            .bind_storage(post_id)
            .bind_storage(tag_id)
            .bind_storage(label)
            .execute(&mut *tx)
            .await?;
    }
    for slug in tag_diff.to_remove {
        sqlx::query(tags::DELETE_POST_TAG_BY_SLUG)
            .bind_storage(post_id)
            .bind_storage(slug)
            .execute(&mut *tx)
            .await?;
    }
    media::replace_post_media::<Postgres>(tx, post_id, input.rendered.media()).await?;
    Ok(())
}

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
        for key in media::media_advisory_lock_keys(media.iter().cloned()) {
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind_storage(key)
                .execute(&mut *conn)
                .await?;
        }
        Ok(())
    }

    async fn lock_live_idempotency_mapping(
        conn: &mut <Self as sqlx::Database>::Connection,
        user_id: UserId,
        key: &IdempotencyKey,
        cutoff: UtcInstant,
    ) -> sqlx::Result<Option<PostId>> {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind_storage(lifecycle::idempotency_advisory_lock_key(user_id, key))
            .execute(&mut *conn)
            .await?;
        sqlx::query_scalar(
            "SELECT post_id FROM idempotency_keys
             WHERE user_id = $1 AND key = $2 AND created_at > $3
             FOR UPDATE",
        )
        .bind_storage(user_id)
        .bind_storage(key)
        .bind_storage(cutoff)
        .fetch_optional(&mut *conn)
        .await
    }

    const DELETE_POST_MEDIA: &'static str = "DELETE FROM post_media WHERE post_id = $1 AND subject_kind = 'current' AND revision_id = 0";

    async fn update_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostMutation, UpdatePostError> {
        let connection = postgres_connection(transaction)?;
        let existing = sqlx::query_as::<_, PostBookkeepingRow>(
            "SELECT user_id, deleted_at, title, slug, body, format, rendered_html, summary, published_at
             FROM posts WHERE post_id = $1 FOR UPDATE",
        )
        .bind_storage(post_id)
        .fetch_optional(&mut *connection)
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
        if let Some(error) =
            locked_update_expectation_error(connection, post_id, &existing, input).await?
        {
            return Err(error);
        }
        let relations = load_post_update_relations(connection, post_id, input).await?;
        let previous = fetch_post(connection, post_id).await?;
        let previous_has_public_audience = relations
            .existing_audiences
            .iter()
            .any(|(kind, _)| matches!(kind, common::visibility::TargetKind::Public));
        let tag_diff = tags::post_tag_diff(&relations.existing_tags, &input.tags);
        let mut locked_media = media::media_lock_set(input.rendered.media());
        let old_media_set: std::collections::BTreeSet<_> =
            relations.old_media.iter().cloned().collect();
        locked_media.extend(
            relations
                .old_media
                .iter()
                .map(|(source, sha256, filename, _, _)| common::media::MediaRef {
                    source: *source,
                    sha256: sha256.clone(),
                    filename: filename.clone(),
                }),
        );
        if lifecycle::update_scalar_is_noop(&existing, input)
            && tag_diff.to_add.is_empty()
            && tag_diff.to_remove.is_empty()
            && visibility::audiences_are_equal(&relations.existing_audiences, &input.audiences)
            && old_media_set == relations.desired_media
        {
            return Ok(PostMutation {
                record: previous.clone(),
                previous,
                previous_has_public_audience,
                changed: false,
            });
        }
        Self::lock_media_references(connection, &locked_media).await?;
        apply_post_update(connection, post_id, input, tag_diff).await?;
        let record = fetch_post(connection, post_id).await?;
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
        lifecycle_post(transaction, post_id, user_id, true, false, now).await
    }

    async fn soft_delete_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
        now: UtcInstant,
    ) -> Result<Option<PostMutation>, sqlx::Error> {
        lifecycle_post(transaction, post_id, user_id, false, true, now).await
    }

    async fn unpublish_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
        now: UtcInstant,
    ) -> Result<Option<PostMutation>, sqlx::Error> {
        lifecycle_post(transaction, post_id, user_id, false, false, now).await
    }

    async fn set_post_tags(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
        desired: &[TagLabel],
    ) -> Result<(), TaggingError> {
        let connection = postgres_connection(transaction)?;
        let post = sqlx::query_as::<_, (UserId, Option<common::time::UtcInstant>)>(
            "SELECT user_id, deleted_at FROM posts WHERE post_id = $1 FOR UPDATE",
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
        let media = load_current_post_media_lock_set(&mut *connection, post_id).await?;
        <Postgres as PostDialect>::lock_media_references(&mut *connection, &media).await?;
        lifecycle::capture_complete_post_revision::<Postgres>(
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
            return helpers::preserve_after_secondary(
                Err(sqlx::Error::Protocol(
                    "post rendered HTML changed during media-reference backfill".to_owned(),
                )),
                tx.rollback().await,
                host::error::ErrorKind::Storage,
                host::error::ErrorClass::Transient,
                "storage.postgres.post_media_reference_backfill.rollback",
            );
        }
        media::replace_legacy_post_media::<Postgres>(&mut tx, candidates).await?;
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
        let mut query = QueryBuilder::<Postgres>::new(String::new());
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
    use crate::test_support::{
        Backend, SeedUser, create_post_via_service, media_ref_for, media_url_for,
        set_post_tags_confirmed,
    };
    use common::test_support::{parse_post_body, parse_tag_label};
    use std::{sync::Arc, time::Duration};

    /// A tag mutation captures a complete revision, including current media. On
    /// `PostgreSQL` copy must wait for the ordinary media lock rather than
    /// racing a guarded delete or reclaim (ADR-0154).
    // guard:low-level-db — exercises a held PostgreSQL advisory lock directly
    #[tokio::test]
    async fn postgres_tag_revision_capture_waits_for_current_media_lock() {
        let env = Backend::Postgres.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let media = media_ref_for("tag-revision-lock.jpg");
        let post = create_post_via_service(
            &env.state,
            user,
            parse_post_body(&format!(
                "<img src=\"{}\">",
                media_url_for("tag-revision-lock.jpg")
            )),
        )
        .await;
        let held = env
            .base
            .pool()
            .lock_media_reference_for_write(&media)
            .await
            .expect("take the current media lock");
        let posts = Arc::clone(&env.state.posts);
        let write_scope = env.state.write_scope.clone();
        let mut tag_update = tokio::spawn(async move {
            set_post_tags_confirmed(
                &write_scope,
                posts,
                post,
                user,
                &[parse_tag_label("locked")],
            )
            .await
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut tag_update)
                .await
                .is_err(),
            "tag revision capture completed while its current media lock was held"
        );

        held.rollback()
            .await
            .expect("release the current media lock");
        tag_update
            .await
            .expect("tag update task panicked")
            .expect("tag update failed after lock release");
        assert_eq!(
            env.base
                .pool()
                .scalar_i64(&format!(
                    "SELECT COUNT(*) FROM post_revisions WHERE post_id = {post}"
                ))
                .await
                .expect("count captured revisions"),
            1,
            "the deferred tag mutation captures exactly one prior-state revision"
        );
    }
}
