use async_trait::async_trait;
use sqlx::{Pool, Postgres, QueryBuilder};

use crate::helpers;
use crate::posts::{
    self, MediaReferenceEvidence, PostBookkeepingRow, PostMediaReferenceBackfill, PostTag,
    PostTagDiff, PostTagRow,
};
use crate::{
    InstanceId, PostDialect, PostRecord, PostStore, PublishUpdate, RenderedHtml, TaggingError,
    UpdatePostError, UpdatePostInput, WriteTransaction, postgres_connection,
};
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
    .bind(post_id)
    .fetch_all(&mut *connection)
    .await?;
    Ok(posts::update_expectation_error(
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
    .bind(post_id)
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
) -> sqlx::Result<()> {
    let now = UtcInstant::now();
    let media = load_current_post_media_lock_set(&mut *connection, post_id).await?;
    <Postgres as PostDialect>::lock_media_references(connection, &media).await?;
    posts::capture_complete_post_revision::<Postgres>(connection, post_id, now).await?;
    if delete {
        sqlx::query("UPDATE posts SET deleted_at = $1 WHERE post_id = $2")
            .bind(now)
            .bind(post_id)
            .execute(&mut *connection)
            .await?;
    } else if publish {
        sqlx::query("UPDATE posts SET published_at = $1, updated_at = $1 WHERE post_id = $2")
            .bind(now)
            .bind(post_id)
            .execute(&mut *connection)
            .await?;
    } else {
        sqlx::query("UPDATE posts SET published_at = NULL, updated_at = $1 WHERE post_id = $2")
            .bind(now)
            .bind(post_id)
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
) -> Result<Option<PostRecord>, sqlx::Error> {
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
    .bind(post_id)
    .fetch_optional(&mut *connection)
    .await?;
    let Some((owner, deleted_at, published_at)) = state else {
        return Ok(None);
    };
    if owner != user_id || deleted_at.is_some() {
        return Ok(None);
    }
    let changed =
        delete || (publish && published_at.is_none()) || (!publish && published_at.is_some());
    if changed {
        apply_lifecycle_change(connection, post_id, publish, delete).await?;
    }
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
    .bind(post_id)
    .fetch_one(&mut *connection)
    .await
    .map(Some)
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
    let tag_rows = sqlx::query_as::<_, PostTagRow>(posts::SELECT_POST_TAGS)
        .bind(post_id)
        .fetch_all(&mut *tx)
        .await?;
    let existing_tags = posts::post_tags_from_rows(tag_rows);
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
    .bind(post_id)
    .fetch_all(&mut *tx)
    .await?;
    let old_media = sqlx::query_as::<_, MediaRefRow>(
        "SELECT source, sha256, filename, reference_kind, reference_form FROM post_media
         WHERE post_id = $1 AND subject_kind = 'current' AND revision_id = 0",
    )
    .bind(post_id)
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
) -> Result<PostRecord, UpdatePostError> {
    let now = input.request_clock;
    posts::capture_complete_post_revision::<Postgres>(tx, post_id, now).await?;
    let (unpublish, explicit_published_at) = match input.publish {
        PublishUpdate::Unpublish => (true, None),
        PublishUpdate::Publish { at } => (false, at),
    };
    let row = sqlx::query_as::<_, PostRecord>(
        "UPDATE posts
         SET title = $1, slug = CASE WHEN published_at IS NULL THEN $2 ELSE slug END,
             body = $3, format = $4, rendered_html = $5,
             published_at = CASE WHEN $6 THEN NULL WHEN $7 IS NOT NULL THEN $8
                 ELSE COALESCE(published_at, $9) END,
             updated_at = $10, summary = $11
         WHERE post_id = $12
         RETURNING post_id, user_id,
                   (SELECT username FROM users WHERE user_id = posts.user_id) AS username,
                   title, slug, body, format, rendered_html,
                   created_at, updated_at, published_at, deleted_at, summary,
                   COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display)) FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = posts.post_id), '[]'::json)::text AS tags",
    )
    .bind(input.title.as_ref())
    .bind(&input.slug)
    .bind(&input.body)
    .bind(input.format)
    .bind(input.rendered.html())
    .bind(unpublish)
    .bind(explicit_published_at)
    .bind(explicit_published_at)
    .bind(now)
    .bind(now)
    .bind(input.summary.as_ref())
    .bind(post_id)
    .fetch_one(&mut *tx)
    .await?;
    posts::replace_post_audiences::<Postgres>(tx, post_id, &input.audiences).await?;
    for label in tag_diff.to_add {
        let tag_id = sqlx::query_scalar::<_, TagId>(posts::UPSERT_TAG_RETURNING_ID)
            .bind(label.slug())
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(posts::INSERT_POST_TAG)
            .bind(post_id)
            .bind(tag_id)
            .bind(label)
            .execute(&mut *tx)
            .await?;
    }
    for slug in tag_diff.to_remove {
        sqlx::query(posts::DELETE_POST_TAG_BY_SLUG)
            .bind(post_id)
            .bind(slug)
            .execute(&mut *tx)
            .await?;
    }
    posts::replace_post_media::<Postgres>(tx, post_id, input.rendered.media()).await?;
    Ok(row)
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
        for key in posts::media_advisory_lock_keys(media.iter().cloned()) {
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(key)
                .execute(&mut *conn)
                .await?;
        }
        Ok(())
    }

    const DELETE_POST_MEDIA: &'static str = "DELETE FROM post_media WHERE post_id = $1 AND subject_kind = 'current' AND revision_id = 0";

    async fn update_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        editor_user_id: UserId,
        input: &UpdatePostInput,
    ) -> Result<PostRecord, UpdatePostError> {
        let connection = postgres_connection(transaction)?;
        let existing = sqlx::query_as::<_, PostBookkeepingRow>(
            "SELECT user_id, deleted_at, title, slug, body, format, rendered_html, summary, published_at
             FROM posts WHERE post_id = $1 FOR UPDATE",
        )
        .bind(post_id)
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
        let tag_diff = posts::post_tag_diff(&relations.existing_tags, &input.tags);
        let mut locked_media = posts::media_lock_set(input.rendered.media());
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
        if posts::update_scalar_is_noop(&existing, input)
            && tag_diff.to_add.is_empty()
            && tag_diff.to_remove.is_empty()
            && posts::audiences_are_equal(&relations.existing_audiences, &input.audiences)
            && old_media_set == relations.desired_media
        {
            return sqlx::query_as::<_, PostRecord>(
                "SELECT p.post_id, p.user_id, u.username, p.title, p.slug, p.body, p.format, p.rendered_html,
                        p.created_at, p.updated_at, p.published_at, p.deleted_at, p.summary,
                        COALESCE((SELECT json_agg(json_build_object('tag_id', t.tag_id, 'tag_slug', t.tag_slug, 'tag_display', pt.tag_display) ORDER BY t.tag_slug COLLATE \"C\") FROM post_tags pt JOIN tags t ON pt.tag_id = t.tag_id WHERE pt.post_id = p.post_id), '[]'::json)::text AS tags
                 FROM posts p JOIN users u ON u.user_id = p.user_id WHERE p.post_id = $1",
            )
            .bind(post_id)
            .fetch_one(&mut *connection)
            .await
            .map_err(UpdatePostError::from);
        }
        Self::lock_media_references(connection, &locked_media).await?;
        apply_post_update(connection, post_id, input, tag_diff).await
    }

    async fn publish_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<Option<PostRecord>, sqlx::Error> {
        lifecycle_post(transaction, post_id, user_id, true, false).await
    }

    async fn soft_delete_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<bool, sqlx::Error> {
        Ok(lifecycle_post(transaction, post_id, user_id, false, true)
            .await?
            .is_some())
    }

    async fn unpublish_post(
        transaction: &mut WriteTransaction,
        post_id: PostId,
        user_id: UserId,
    ) -> Result<Option<PostRecord>, sqlx::Error> {
        lifecycle_post(transaction, post_id, user_id, false, false).await
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
        .bind(post_id)
        .fetch_optional(&mut *connection)
        .await?;
        match post {
            None | Some((_, Some(_))) => return Err(TaggingError::PostNotFound),
            Some((owner, None)) if owner != user_id => return Err(TaggingError::Unauthorized),
            Some(_) => {}
        }
        let rows = sqlx::query_as::<_, PostTagRow>(posts::SELECT_POST_TAGS)
            .bind(post_id)
            .fetch_all(&mut *connection)
            .await?;
        let existing = posts::post_tags_from_rows(rows);
        let diff = posts::post_tag_diff(&existing, desired);
        if diff.to_add.is_empty() && diff.to_remove.is_empty() {
            return Ok(());
        }
        let media = load_current_post_media_lock_set(&mut *connection, post_id).await?;
        <Postgres as PostDialect>::lock_media_references(&mut *connection, &media).await?;
        posts::capture_complete_post_revision::<Postgres>(
            &mut *connection,
            post_id,
            UtcInstant::now(),
        )
        .await?;
        for label in diff.to_add {
            let tag_id = sqlx::query_scalar::<_, TagId>(posts::UPSERT_TAG_RETURNING_ID)
                .bind(label.slug())
                .fetch_one(&mut *connection)
                .await?;
            sqlx::query(posts::INSERT_POST_TAG)
                .bind(post_id)
                .bind(tag_id)
                .bind(label)
                .execute(&mut *connection)
                .await?;
        }
        for slug in diff.to_remove {
            sqlx::query(posts::DELETE_POST_TAG_BY_SLUG)
                .bind(post_id)
                .bind(slug)
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
        posts::replace_legacy_post_media::<Postgres>(&mut tx, candidates).await?;
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
        posts::push_media_reference_evidence_cte(&mut query, evidence);
        query.push("SELECT DISTINCT pm.post_id");
        posts::push_owner_media_reference_from_where(&mut query, user_id, media);
        posts::push_live_media_reference_predicate(&mut query, current_instance_id);
        query.push(" ORDER BY pm.post_id");
        query.build_query_scalar::<PostId>().fetch_all(pool).await
    }
}
