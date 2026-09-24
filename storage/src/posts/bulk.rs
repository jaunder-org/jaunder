//! Set-based atomic Post management mutations.

use std::collections::{BTreeSet, HashSet};

use common::ids::{AudienceId, PostId, RevisionId, UserId};
use common::media::{ContentHash, Filename, MediaRef, MediaSource};
use common::time::UtcInstant;
use common::visibility::{AudienceTarget, TargetKind};
use host::feed::{self, FeedPath};
use sqlx::{Database, Decode, Encode, Executor, QueryBuilder, Type};

use crate::posts::PostDialect;
use crate::posts::management::{
    BulkLockedPost, BulkPostMutationError, BulkPostMutationResult, BulkPostOperation,
    ManagementSelectionSnapshot,
};
use crate::posts::models::POST_RECORD_COLUMNS;
use crate::sql::{QueryBuilderStorageExt, SET_OPERATION_BIND_BATCH};

/// Storage result plus feed invalidations earned by the committed mutation.
#[derive(Debug)]
pub struct BulkPostMutationEvidence {
    pub result: BulkPostMutationResult,
    pub feed_paths: Vec<FeedPath>,
}

fn canonical_audiences(audiences: &[AudienceTarget]) -> Vec<(TargetKind, Option<AudienceId>)> {
    let mut targets: Vec<_> = audiences
        .iter()
        .filter_map(|target| match target {
            AudienceTarget::Public => Some((TargetKind::Public, None)),
            AudienceTarget::Subscribers => Some((TargetKind::Subscribers, None)),
            AudienceTarget::Named(id) => Some((TargetKind::Named, Some(*id))),
            AudienceTarget::Private => None,
        })
        .collect();
    targets.sort_by_key(|(kind, audience_id)| {
        let kind = match kind {
            TargetKind::Public => 0,
            TargetKind::Subscribers => 1,
            TargetKind::Named => 2,
        };
        (kind, audience_id.map(i64::from).unwrap_or_default())
    });
    targets.dedup();
    targets
}

fn changed_ids(locked: &[BulkLockedPost], operation: &BulkPostOperation) -> Vec<PostId> {
    let desired = match operation {
        BulkPostOperation::ChangeAudience(audiences) => Some(canonical_audiences(audiences)),
        BulkPostOperation::Delete => None,
    };
    locked
        .iter()
        .filter(|post| {
            desired
                .as_ref()
                .is_none_or(|desired| canonical_audiences(&post.audiences) != *desired)
        })
        .map(|post| post.record.post_id)
        .collect()
}

fn feed_paths(
    locked: &[BulkLockedPost],
    changed: &HashSet<PostId>,
    operation: &BulkPostOperation,
    now: UtcInstant,
) -> Vec<FeedPath> {
    let desired_public = matches!(operation, BulkPostOperation::ChangeAudience(audiences) if audiences.iter().any(|target| matches!(target, AudienceTarget::Public)));
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for post in locked
        .iter()
        .filter(|post| changed.contains(&post.record.post_id))
    {
        let was_public = post.record.published_at.is_some_and(|at| at <= now)
            && post
                .audiences
                .iter()
                .any(|target| matches!(target, AudienceTarget::Public));
        let is_public = desired_public && post.record.published_at.is_some_and(|at| at <= now);
        if !was_public && !is_public {
            continue;
        }
        for path in feed::affected_feed_urls(
            &post.record.author_username,
            post.record.tags.iter().map(|tag| &tag.tag_slug),
        ) {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }
    paths
}

fn push_ids<DB>(query: &mut QueryBuilder<DB>, ids: &[PostId])
where
    DB: Database,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
{
    let mut separated = query.separated(", ");
    for id in ids {
        separated.push_storage_bind(*id);
    }
}

async fn lock_snapshot<DB>(
    conn: &mut DB::Connection,
    user_id: UserId,
    snapshot: &ManagementSelectionSnapshot,
) -> Result<Vec<BulkLockedPost>, BulkPostMutationError>
where
    DB: PostDialect,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> UserId: Encode<'q, DB> + Type<DB>,
    for<'q> super::PostMutationVersion: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'r> BulkLockedPost: sqlx::FromRow<'r, DB::Row>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    if snapshot.targets.is_empty() {
        return Ok(Vec::new());
    }
    let mut rows = Vec::with_capacity(snapshot.targets.len());
    for targets in snapshot.targets.chunks(SET_OPERATION_BIND_BATCH) {
        let mut query = QueryBuilder::<DB>::new("WITH selection(post_id, mutation_version) AS (");
        query.push_values(targets, |mut row, target| {
            row.push_storage_bind(target.post_id)
                .push_storage_bind(target.mutation_version);
        });
        query
            .push(") SELECT ")
            .push(POST_RECORD_COLUMNS)
            .push(", p.mutation_version, ");
        query.push(DB::TAGS_SUBQUERY).push(" AS tags, ");
        query
            .push(DB::MANAGED_AUDIENCES_SUBQUERY)
            .push(" AS audiences FROM selection s JOIN posts p ON p.post_id = s.post_id AND p.mutation_version = s.mutation_version JOIN users u ON u.user_id = p.user_id WHERE p.user_id = ")
            .push_storage_bind(user_id)
            .push(" AND p.deleted_at IS NULL ORDER BY p.post_id")
            .push(DB::BULK_LOCK_SUFFIX);
        rows.extend(
            query
                .build_query_as::<BulkLockedPost>()
                .fetch_all(&mut *conn)
                .await?,
        );
    }
    rows.sort_by_key(|row| row.record.post_id);
    let exact = rows.len() == snapshot.targets.len()
        && rows.iter().zip(&snapshot.targets).all(|(row, target)| {
            row.record.post_id == target.post_id && row.mutation_version == target.mutation_version
        });
    if !exact {
        return Err(BulkPostMutationError::SnapshotConflict);
    }
    Ok(rows)
}

async fn load_bulk_media<DB>(
    conn: &mut DB::Connection,
    ids: &[PostId],
) -> Result<BTreeSet<MediaRef>, sqlx::Error>
where
    DB: Database,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'r> (MediaSource, ContentHash, Filename): sqlx::FromRow<'r, DB::Row>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    let mut media = BTreeSet::new();
    for ids in ids.chunks(SET_OPERATION_BIND_BATCH) {
        let mut query = QueryBuilder::<DB>::new(
            "SELECT source, sha256, filename FROM post_media WHERE subject_kind = 'current' AND revision_id = 0 AND post_id IN (",
        );
        push_ids(&mut query, ids);
        query.push(") ORDER BY source, sha256, filename");
        media.extend(
            query
                .build_query_as::<(MediaSource, ContentHash, Filename)>()
                .fetch_all(&mut *conn)
                .await?
                .into_iter()
                .map(|(source, sha256, filename)| MediaRef {
                    source,
                    sha256,
                    filename,
                }),
        );
    }
    Ok(media)
}

async fn capture_revisions<DB>(
    conn: &mut DB::Connection,
    ids: &[PostId],
    now: UtcInstant,
) -> Result<Vec<(PostId, RevisionId)>, sqlx::Error>
where
    DB: Database,
    for<'q> PostId: Encode<'q, DB> + Decode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Decode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
{
    let mut revisions = Vec::with_capacity(ids.len());
    for ids in ids.chunks(SET_OPERATION_BIND_BATCH) {
        let mut query = QueryBuilder::<DB>::new(
            "INSERT INTO post_revisions (post_id, user_id, title, rendered_title, slug, body, format, rendered_html, summary, created_at, updated_at, published_at, deleted_at, captured_at) SELECT post_id, user_id, title, rendered_title, slug, body, format, rendered_html, summary, created_at, updated_at, published_at, deleted_at, ",
        );
        query
            .push_storage_bind(now)
            .push(" FROM posts WHERE post_id IN (");
        push_ids(&mut query, ids);
        query.push(") ORDER BY post_id RETURNING post_id, revision_id");
        revisions.extend(
            query
                .build_query_as::<(PostId, RevisionId)>()
                .fetch_all(&mut *conn)
                .await?,
        );
    }
    revisions.sort_by_key(|(post_id, _)| *post_id);
    Ok(revisions)
}

fn push_revision_map<DB>(query: &mut QueryBuilder<DB>, revisions: &[(PostId, RevisionId)])
where
    DB: Database,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Encode<'q, DB> + Type<DB>,
{
    query.push_values(revisions, |mut row, (post_id, revision_id)| {
        row.push_storage_bind(*post_id)
            .push_storage_bind(*revision_id);
    });
}

async fn capture_revision_children<DB>(
    conn: &mut DB::Connection,
    revisions: &[(PostId, RevisionId)],
) -> Result<(), sqlx::Error>
where
    DB: Database,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    for revisions in revisions.chunks(SET_OPERATION_BIND_BATCH) {
        let mut tags = QueryBuilder::<DB>::new("WITH revisions(post_id, revision_id) AS (");
        push_revision_map(&mut tags, revisions);
        tags.push(") INSERT INTO post_revision_tags (revision_id, tag_slug, tag_display) SELECT r.revision_id, t.tag_slug, pt.tag_display FROM revisions r JOIN post_tags pt ON pt.post_id = r.post_id JOIN tags t ON t.tag_id = pt.tag_id");
        tags.build().execute(&mut *conn).await?;

        let mut audiences = QueryBuilder::<DB>::new("WITH revisions(post_id, revision_id) AS (");
        push_revision_map(&mut audiences, revisions);
        audiences.push(") INSERT INTO post_revision_audiences (revision_id, target_kind, audience_id) SELECT r.revision_id, tk.name, pa.audience_id FROM revisions r JOIN post_audiences pa ON pa.post_id = r.post_id JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id");
        audiences.build().execute(&mut *conn).await?;

        let mut media = QueryBuilder::<DB>::new("WITH revisions(post_id, revision_id) AS (");
        push_revision_map(&mut media, revisions);
        media.push(") INSERT INTO post_media (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form) SELECT pm.post_id, 'revision', r.revision_id, pm.source, pm.sha256, pm.filename, pm.reference_kind, pm.reference_form FROM revisions r JOIN post_media pm ON pm.post_id = r.post_id WHERE pm.subject_kind = 'current' AND pm.revision_id = 0");
        media.build().execute(&mut *conn).await?;
    }
    Ok(())
}

async fn change_audiences<DB>(
    conn: &mut DB::Connection,
    ids: &[PostId],
    audiences: &[AudienceTarget],
    now: UtcInstant,
) -> Result<(), sqlx::Error>
where
    DB: Database,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> AudienceId: Encode<'q, DB> + Type<DB>,
    for<'q> Option<AudienceId>: Encode<'q, DB> + Type<DB>,
    for<'q> TargetKind: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    let targets = canonical_audiences(audiences);
    for ids in ids.chunks(SET_OPERATION_BIND_BATCH) {
        let mut delete = QueryBuilder::<DB>::new("DELETE FROM post_audiences WHERE post_id IN (");
        push_ids(&mut delete, ids);
        delete.push(")").build().execute(&mut *conn).await?;

        for targets in targets.chunks(SET_OPERATION_BIND_BATCH) {
            let mut insert = QueryBuilder::<DB>::new("WITH changed(post_id) AS (");
            insert.push_values(ids, |mut row, id| {
                row.push_storage_bind(*id);
            });
            insert.push("), targets(target_kind, audience_id) AS (");
            insert.push_values(targets, |mut row, (kind, audience_id)| {
                row.push_storage_bind(*kind).push_storage_bind(*audience_id);
            });
            insert.push(") INSERT INTO post_audiences (post_id, target_kind_id, audience_id) SELECT c.post_id, tk.kind_id, t.audience_id FROM changed c CROSS JOIN targets t JOIN target_kinds tk ON tk.name = t.target_kind");
            insert.build().execute(&mut *conn).await?;
        }

        let mut update = QueryBuilder::<DB>::new("UPDATE posts SET updated_at = ");
        update
            .push_storage_bind(now)
            .push(", mutation_version = mutation_version + 1 WHERE post_id IN (");
        push_ids(&mut update, ids);
        update.push(")").build().execute(&mut *conn).await?;
    }
    Ok(())
}

async fn delete_posts<DB>(
    conn: &mut DB::Connection,
    ids: &[PostId],
    now: UtcInstant,
) -> Result<(), sqlx::Error>
where
    DB: Database,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
{
    for ids in ids.chunks(SET_OPERATION_BIND_BATCH) {
        let mut query = QueryBuilder::<DB>::new("UPDATE posts SET deleted_at = ");
        query
            .push_storage_bind(now)
            .push(", mutation_version = mutation_version + 1 WHERE post_id IN (");
        push_ids(&mut query, ids);
        query.push(")").build().execute(&mut *conn).await?;
    }
    Ok(())
}

/// Applies one exact snapshot with a fixed set of set-based write statements.
pub(crate) async fn apply_bulk_post_operation<DB>(
    conn: &mut DB::Connection,
    user_id: UserId,
    snapshot: &ManagementSelectionSnapshot,
    operation: &BulkPostOperation,
    now: UtcInstant,
) -> Result<BulkPostMutationEvidence, BulkPostMutationError>
where
    DB: PostDialect,
    for<'q> PostId: Encode<'q, DB> + Decode<'q, DB> + Type<DB>,
    for<'q> UserId: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Encode<'q, DB> + Decode<'q, DB> + Type<DB>,
    for<'q> AudienceId: Encode<'q, DB> + Type<DB>,
    for<'q> Option<AudienceId>: Encode<'q, DB> + Type<DB>,
    for<'q> TargetKind: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'q> super::PostMutationVersion: Encode<'q, DB> + Type<DB>,
    for<'r> BulkLockedPost: sqlx::FromRow<'r, DB::Row>,
    for<'r> (MediaSource, ContentHash, Filename): sqlx::FromRow<'r, DB::Row>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
{
    let locked = lock_snapshot::<DB>(conn, user_id, snapshot).await?;
    let ids = changed_ids(&locked, operation);
    let selected_count = snapshot.selected_count();
    if ids.is_empty() {
        return Ok(BulkPostMutationEvidence {
            result: BulkPostMutationResult {
                selected_count,
                changed_count: 0,
            },
            feed_paths: Vec::new(),
        });
    }
    let media = load_bulk_media::<DB>(conn, &ids).await?;
    DB::lock_media_references(conn, &media).await?;
    let revisions = capture_revisions::<DB>(conn, &ids, now).await?;
    capture_revision_children::<DB>(conn, &revisions).await?;
    match operation {
        BulkPostOperation::ChangeAudience(audiences) => {
            change_audiences::<DB>(conn, &ids, audiences, now).await?;
        }
        BulkPostOperation::Delete => delete_posts::<DB>(conn, &ids, now).await?,
    }
    let changed: HashSet<_> = ids.iter().copied().collect();
    Ok(BulkPostMutationEvidence {
        result: BulkPostMutationResult {
            selected_count,
            changed_count: ids.len(),
        },
        feed_paths: feed_paths(&locked, &changed, operation, now),
    })
}
