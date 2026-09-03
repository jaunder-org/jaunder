//! Post lifecycle bookkeeping, revisions, and shared mutation support.

use chrono::Duration;
use sha2::{Digest, Sha256};
use sqlx::{Database, Decode, Encode, Executor, Pool, Result, Row, Type};

use crate::posts::cursors::PostRevisionCursor;
use crate::posts::errors::{CreatePostError, UpdatePostError};
use crate::posts::media;
use crate::posts::models::{
    CreatePostInput, PostLifecycle, PostRevisionMetadata, PostRevisionPage, PublishUpdate,
    RenderedHtml, UpdatePostInput,
};
use crate::posts::store::PostDialect;
use crate::posts::tags;
use crate::posts::visibility;
use crate::sql::{QueryStorageExt, RowCount};
use common::idempotency_key::IdempotencyKey;
use common::ids::{AudienceId, PostId, RevisionId, UserId};
use common::pagination::{PageSize, RowLimit};
use common::post_body::PostBody;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::render::PostFormat;
use common::slug::Slug;
use common::tag::TagLabel;
use common::time::UtcInstant;
use host::etag;

const IDEMPOTENCY_REPLAY_WINDOW_HOURS: i64 = 1;

pub(crate) fn idempotency_replay_cutoff(now: UtcInstant) -> UtcInstant {
    UtcInstant::from(now.value() - Duration::hours(IDEMPOTENCY_REPLAY_WINDOW_HOURS))
}

/// `PostgreSQL`'s signed advisory-lock key for one user/idempotency-key pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, macros::SqlxBridge)]
pub(crate) struct IdempotencyAdvisoryLockKey(i64);

/// Derives the `PostgreSQL` advisory-lock key for one user's `Idempotency Key`.
///
/// A collision only serializes unrelated creates; it cannot change behavior.
#[must_use]
pub(crate) fn idempotency_advisory_lock_key(
    user_id: UserId,
    key: &IdempotencyKey,
) -> IdempotencyAdvisoryLockKey {
    let mut digest = Sha256::new();
    digest.update(i64::from(user_id).to_be_bytes());
    digest.update(key.as_ref().as_bytes());
    let digest: [u8; 32] = digest.finalize().into();
    IdempotencyAdvisoryLockKey(i64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]))
}

/// Captures every scalar field belonging to a complete immutable prior Post
/// state. Child snapshot writes intentionally remain with Task 2's transaction.
/// Bind order: `captured_at, post_id`.
pub(crate) const INSERT_COMPLETE_POST_REVISION: &str = "INSERT INTO post_revisions
     (post_id, user_id, title, slug, body, format, rendered_html, summary,
      created_at, updated_at, published_at, deleted_at, captured_at)
     SELECT post_id, user_id, title, slug, body, format, rendered_html, summary,
            created_at, updated_at, published_at, deleted_at, $1
     FROM posts WHERE post_id = $2
     RETURNING revision_id";

/// The locked pre-write columns needed for final-state and content expectations.
#[derive(sqlx::FromRow)]
pub(crate) struct PostBookkeepingRow {
    pub user_id: UserId,
    pub deleted_at: Option<UtcInstant>,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub body: PostBody,
    pub format: PostFormat,
    pub rendered_html: RenderedHtml,
    pub summary: Option<PostSummary>,
    pub published_at: Option<UtcInstant>,
}

/// This is decoded explicitly rather than through a positional `SQLx` tuple so
/// persisted values always cross the storage boundary as domain types.
pub(crate) struct RevisionDetailRow {
    pub(crate) revision_id: RevisionId,
    pub(crate) post_id: PostId,
    pub(crate) user_id: UserId,
    pub(crate) title: Option<PostTitle>,
    pub(crate) slug: Slug,
    pub(crate) body: PostBody,
    pub(crate) format: PostFormat,
    pub(crate) rendered_html: RenderedHtml,
    pub(crate) summary: Option<PostSummary>,
    pub(crate) created_at: UtcInstant,
    pub(crate) updated_at: UtcInstant,
    pub(crate) published_at: Option<UtcInstant>,
    pub(crate) deleted_at: Option<UtcInstant>,
    pub(crate) captured_at: UtcInstant,
}

/// The typed columns required to render one immutable revision in a history list.
pub(crate) struct RevisionMetadataRow {
    revision_id: RevisionId,
    post_id: PostId,
    title: Option<PostTitle>,
    slug: Slug,
    captured_at: UtcInstant,
    deleted_at: Option<UtcInstant>,
    published_at: Option<UtcInstant>,
    current_deleted_at: Option<UtcInstant>,
}

/// Decodes a raw SQL row into a storage-internal typed projection.
pub(crate) trait DecodeRawRow<DB: Database>: Sized {
    fn decode(row: DB::Row) -> Result<Self>;
}

impl<DB> DecodeRawRow<DB> for RevisionDetailRow
where
    DB: Database,
    for<'r> RevisionId: Decode<'r, DB> + Type<DB>,
    for<'r> PostId: Decode<'r, DB> + Type<DB>,
    for<'r> UserId: Decode<'r, DB> + Type<DB>,
    for<'r> Option<PostTitle>: Decode<'r, DB> + Type<DB>,
    for<'r> Slug: Decode<'r, DB> + Type<DB>,
    for<'r> PostBody: Decode<'r, DB> + Type<DB>,
    for<'r> PostFormat: Decode<'r, DB> + Type<DB>,
    for<'r> RenderedHtml: Decode<'r, DB> + Type<DB>,
    for<'r> Option<PostSummary>: Decode<'r, DB> + Type<DB>,
    for<'r> UtcInstant: Decode<'r, DB> + Type<DB>,
    for<'r> Option<UtcInstant>: Decode<'r, DB> + Type<DB>,
    for<'r> &'r str: sqlx::ColumnIndex<DB::Row>,
{
    fn decode(row: DB::Row) -> Result<Self> {
        Ok(Self {
            revision_id: row.try_get::<RevisionId, _>("revision_id")?,
            post_id: row.try_get::<PostId, _>("post_id")?,
            user_id: row.try_get::<UserId, _>("user_id")?,
            title: row.try_get::<Option<PostTitle>, _>("title")?,
            slug: row.try_get::<Slug, _>("slug")?,
            body: row.try_get::<PostBody, _>("body")?,
            format: row.try_get::<PostFormat, _>("format")?,
            rendered_html: row.try_get::<RenderedHtml, _>("rendered_html")?,
            summary: row.try_get::<Option<PostSummary>, _>("summary")?,
            created_at: row.try_get::<UtcInstant, _>("created_at")?,
            updated_at: row.try_get::<UtcInstant, _>("updated_at")?,
            published_at: row.try_get::<Option<UtcInstant>, _>("published_at")?,
            deleted_at: row.try_get::<Option<UtcInstant>, _>("deleted_at")?,
            captured_at: row.try_get::<UtcInstant, _>("captured_at")?,
        })
    }
}

impl<DB> DecodeRawRow<DB> for RevisionMetadataRow
where
    DB: Database,
    for<'r> RevisionId: Decode<'r, DB> + Type<DB>,
    for<'r> PostId: Decode<'r, DB> + Type<DB>,
    for<'r> Option<PostTitle>: Decode<'r, DB> + Type<DB>,
    for<'r> Slug: Decode<'r, DB> + Type<DB>,
    for<'r> UtcInstant: Decode<'r, DB> + Type<DB>,
    for<'r> Option<UtcInstant>: Decode<'r, DB> + Type<DB>,
    for<'r> &'r str: sqlx::ColumnIndex<DB::Row>,
{
    fn decode(row: DB::Row) -> Result<Self> {
        Ok(Self {
            revision_id: row.try_get::<RevisionId, _>("revision_id")?,
            post_id: row.try_get::<PostId, _>("post_id")?,
            title: row.try_get::<Option<PostTitle>, _>("title")?,
            slug: row.try_get::<Slug, _>("slug")?,
            captured_at: row.try_get::<UtcInstant, _>("captured_at")?,
            deleted_at: row.try_get::<Option<UtcInstant>, _>("deleted_at")?,
            published_at: row.try_get::<Option<UtcInstant>, _>("published_at")?,
            current_deleted_at: row.try_get::<Option<UtcInstant>, _>("current_deleted_at")?,
        })
    }
}

/// Derives the stable lifecycle label from a state snapshot and an explicit
/// clock. Revision callers pass `captured_at`; current-summary callers pass
/// their request clock.
pub(crate) fn post_lifecycle(
    deleted_at: Option<UtcInstant>,
    published_at: Option<UtcInstant>,
    now: UtcInstant,
) -> PostLifecycle {
    if deleted_at.is_some() {
        PostLifecycle::Deleted
    } else if published_at.is_none() {
        PostLifecycle::Draft
    } else if published_at.is_some_and(|published_at| published_at > now) {
        PostLifecycle::Scheduled
    } else {
        PostLifecycle::Published
    }
}

pub(crate) fn revision_page(
    mut revisions: Vec<PostRevisionMetadata>,
    page_size: PageSize,
) -> PostRevisionPage {
    let has_more = page_size.has_more(revisions.len());
    revisions.truncate(page_size.page_len());
    let next_cursor = has_more
        .then(|| {
            revisions.last().map(|revision| PostRevisionCursor {
                revision_id: revision.revision_id,
            })
        })
        .flatten();
    PostRevisionPage {
        revisions,
        next_cursor,
    }
}

pub(crate) async fn revision_metadata_rows<DB>(
    pool: &Pool<DB>,
    user_id: UserId,
    post_id: Option<PostId>,
    cursor: Option<PostRevisionCursor>,
    limit: RowLimit,
) -> Result<Vec<PostRevisionMetadata>>
where
    DB: Database,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'q> UserId: Encode<'q, DB> + Type<DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Encode<'q, DB> + Type<DB>,
    for<'q> RowLimit: Encode<'q, DB> + Type<DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
    RevisionMetadataRow: DecodeRawRow<DB>,
{
    let sql = if post_id.is_some() {
        "SELECT r.revision_id, r.post_id, r.title, r.slug, r.captured_at,
                r.deleted_at, r.published_at, p.deleted_at AS current_deleted_at
         FROM post_revisions r
         JOIN posts p ON p.post_id = r.post_id
         WHERE r.user_id = $1 AND r.post_id = $2 AND r.revision_id < $3
         ORDER BY r.revision_id DESC LIMIT $4"
    } else {
        "SELECT r.revision_id, r.post_id, r.title, r.slug, r.captured_at,
                r.deleted_at, r.published_at, p.deleted_at AS current_deleted_at
         FROM post_revisions r
         JOIN posts p ON p.post_id = r.post_id
         WHERE r.user_id = $1 AND r.revision_id < $2
         ORDER BY r.revision_id DESC LIMIT $3"
    };
    let after = cursor.map_or(RevisionId::from(i64::MAX), |cursor| cursor.revision_id);
    let rows = if let Some(post_id) = post_id {
        sqlx::query(sql)
            .bind_storage(user_id)
            .bind_storage(post_id)
            .bind_storage(after)
            .bind_storage(limit)
            .fetch_all(pool)
            .await?
    } else {
        sqlx::query(sql)
            .bind_storage(user_id)
            .bind_storage(after)
            .bind_storage(limit)
            .fetch_all(pool)
            .await?
    };
    Ok(rows
        .into_iter()
        .map(RevisionMetadataRow::decode)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|row| PostRevisionMetadata {
            revision_id: row.revision_id,
            post_id: row.post_id,
            title: row.title,
            slug: row.slug,
            captured_at: row.captured_at,
            snapshot_lifecycle: post_lifecycle(row.deleted_at, row.published_at, row.captured_at),
            current_deleted: row.current_deleted_at.is_some(),
        })
        .collect())
}

pub(crate) fn create_expectations_match(input: &CreatePostInput) -> bool {
    let expected = &input.expectations;
    expected
        .slug
        .as_ref()
        .is_none_or(|slug| slug == &input.slug)
        && expected.format.is_none_or(|format| format == input.format)
        && expected
            .published_at
            .is_none_or(|published_at| published_at == input.published_at)
}

pub(crate) fn update_scalar_is_noop(
    existing: &PostBookkeepingRow,
    input: &UpdatePostInput,
) -> bool {
    let published_at = match input.publish {
        PublishUpdate::Unpublish => None,
        PublishUpdate::Publish { at: Some(at) } => Some(at),
        PublishUpdate::Publish { at: None } => existing.published_at.or(Some(input.request_clock)),
    };
    existing.title == input.title
        && (existing.published_at.is_some() || existing.slug == input.slug)
        && existing.body == input.body
        && existing.format == input.format
        && existing.rendered_html.as_ref() == input.rendered.html().as_ref()
        && existing.summary == input.summary
        && existing.published_at == published_at
}

pub(crate) fn update_expectation_error(
    post_id: PostId,
    existing: &PostBookkeepingRow,
    tags: &[TagLabel],
    input: &UpdatePostInput,
) -> Option<UpdatePostError> {
    let expected = &input.expectations;
    if expected
        .post_id
        .is_some_and(|expected_id| expected_id != post_id)
    {
        return Some(UpdatePostError::BookkeepingMismatch);
    }
    let final_slug = if existing.published_at.is_some() {
        &existing.slug
    } else {
        &input.slug
    };
    let final_published_at = match input.publish {
        PublishUpdate::Unpublish => None,
        PublishUpdate::Publish { at: Some(at) } => Some(at),
        PublishUpdate::Publish { at: None } => existing.published_at.or(Some(input.request_clock)),
    };
    if expected
        .slug
        .as_ref()
        .is_some_and(|slug| slug != final_slug)
        || expected.format.is_some_and(|format| format != input.format)
        || expected
            .published_at
            .is_some_and(|published_at| published_at != final_published_at)
    {
        return Some(UpdatePostError::BookkeepingMismatch);
    }

    let current_etag = etag::post_content_etag(
        existing.title.as_ref(),
        &existing.body,
        &existing.format,
        existing.summary.as_ref(),
        tags.iter(),
        existing.published_at.is_none(),
    );
    expected
        .content_etag
        .as_ref()
        .is_some_and(|etag| etag != &current_etag)
        .then_some(UpdatePostError::StaleContent)
}

/// Writes one post row and its audience rows onto a caller-supplied transaction
/// connection, so it joins whatever transaction is open.
///
/// This is the single place that knows the post `INSERT` and the
/// unique-violation → [`CreatePostError::SlugConflict`] mapping: both
/// `create_post` (write one) and `create_posts` (write many in one transaction)
/// are pure transaction orchestration over it, so the row-write logic lives once
/// rather than being duplicated per arity.
pub(crate) async fn write_post_in_tx<DB>(
    conn: &mut DB::Connection,
    input: &CreatePostInput,
    now: UtcInstant,
) -> Result<(PostId, bool), CreatePostError>
where
    DB: PostDialect,
    for<'q> i64: Decode<'q, DB> + Encode<'q, DB> + Type<DB>,
    for<'q> RowCount: Decode<'q, DB> + Type<DB>,
    for<'q> Option<AudienceId>: Encode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'q> Option<&'q str>: Encode<'q, DB> + Type<DB>,
    for<'q> Option<String>: Encode<'q, DB> + Type<DB>,
    for<'q> &'q IdempotencyKey: Encode<'q, DB> + Type<DB>,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'q> Option<UtcInstant>: Encode<'q, DB> + Type<DB>,
    // `Slug`/`PostBody` bind as themselves and `PostTitle` as `Option<&PostTitle>`
    // via the ADR-0071 sqlx bridge (the `Option<&…>` pair covers the nullable
    // `title` bind).
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> Option<&'q PostTitle>: Encode<'q, DB> + Type<DB>,
    // `summary` binds as `Option<&PostSummary>` via the ADR-0071 sqlx bridge on
    // the create paths, mirroring the `Option<&PostTitle>` bound above.
    for<'q> Option<&'q PostSummary>: Encode<'q, DB> + Type<DB>,
    (PostId,): for<'r> sqlx::FromRow<'r, DB::Row>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    DB::lock_media_references(conn, &media::media_lock_set(input.rendered.media())).await?;

    let idempotency_key_expired = if let Some(key) = input.idempotency_key.as_ref() {
        let cutoff = idempotency_replay_cutoff(now);
        if let Some(post_id) =
            DB::lock_live_idempotency_mapping(conn, input.user_id, key, cutoff).await?
        {
            return Err(CreatePostError::IdempotencyConflict(post_id));
        }
        sqlx::query_scalar::<_, RowCount>(
            "DELETE FROM idempotency_keys
             WHERE user_id = $1 AND key = $2 AND created_at <= $3
             RETURNING CAST(1 AS BIGINT)",
        )
        .bind_storage(input.user_id)
        .bind_storage(key)
        .bind_storage(cutoff)
        .fetch_optional(&mut *conn)
        .await
        .map_err(CreatePostError::Internal)?
        .is_some()
    } else {
        false
    };

    let post_id = sqlx::query_scalar::<_, PostId>(
        "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at, summary)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING post_id",
    )
    .bind_storage(input.user_id)
    // `Option::as_ref` → `Option<&PostTitle>` (a typed newtype bind, not an
    // `AsRef<str>` strip); the sqlx bridge encodes `Option<&PostTitle>`.
    .bind_storage(input.title.as_ref())
    .bind_storage(&input.slug)
    .bind_storage(&input.body)
    .bind_storage(input.format)
    .bind_storage(input.rendered.html())
    .bind_storage(now)
    .bind_storage(now)
    .bind_storage(input.published_at)
    // `Option::as_ref` → `Option<&PostSummary>` (a typed newtype bind via the
    // ADR-0071 sqlx bridge, not an `AsRef<str>` strip); the `sqlx-newtype-bind`
    // gate forbids stripping to `&str` here.
    .bind_storage(input.summary.as_ref())
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => CreatePostError::SlugConflict,
        e => CreatePostError::Internal(e),
    })?;

    if !create_expectations_match(input) {
        return Err(CreatePostError::BookkeepingMismatch);
    }

    visibility::replace_post_audiences::<DB>(conn, post_id, &input.audiences).await?;
    media::replace_post_media::<DB>(conn, post_id, input.rendered.media()).await?;
    tags::insert_post_tags::<DB>(conn, post_id, &input.tags).await?;

    if let Some(key) = input.idempotency_key.as_ref() {
        sqlx::query(
            "INSERT INTO idempotency_keys (user_id, key, post_id, created_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind_storage(input.user_id)
        .bind_storage(key)
        .bind_storage(post_id)
        .bind_storage(now)
        .execute(&mut *conn)
        .await
        .map_err(CreatePostError::Internal)?;
    }

    Ok((post_id, idempotency_key_expired))
}

/// Captures the locked current state and every normalized child before mutation.
///
/// The copies are SQL-to-SQL rather than reconstructed from an application read:
/// this preserves the exact current media spelling and keeps immutable history
/// independent of later tag/audience lookup changes.
pub(crate) async fn capture_complete_post_revision<DB>(
    conn: &mut DB::Connection,
    post_id: PostId,
    captured_at: UtcInstant,
) -> Result<RevisionId>
where
    DB: Database,
    for<'q> UtcInstant: Encode<'q, DB> + Type<DB>,
    for<'q> PostId: Encode<'q, DB> + Type<DB>,
    for<'q> RevisionId: Decode<'q, DB> + Type<DB>,
    for<'q> i64: Decode<'q, DB> + Encode<'q, DB> + Type<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let revision_id = sqlx::query_scalar::<_, RevisionId>(INSERT_COMPLETE_POST_REVISION)
        .bind_storage(captured_at)
        .bind_storage(post_id)
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO post_revision_tags (revision_id, tag_slug, tag_display)
         SELECT $1, t.tag_slug, pt.tag_display
         FROM post_tags pt JOIN tags t ON t.tag_id = pt.tag_id
         WHERE pt.post_id = $2",
    )
    .bind_storage(revision_id)
    .bind_storage(post_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO post_revision_audiences (revision_id, target_kind, audience_id)
         SELECT $1, tk.name, pa.audience_id
         FROM post_audiences pa JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id
         WHERE pa.post_id = $2",
    )
    .bind_storage(revision_id)
    .bind_storage(post_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO post_media
             (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form)
         SELECT post_id, 'revision', $1, source, sha256, filename, reference_kind, reference_form
         FROM post_media
         WHERE post_id = $2 AND subject_kind = 'current' AND revision_id = 0",
    )
    .bind_storage(revision_id)
    .bind_storage(post_id)
    .execute(&mut *conn)
    .await?;
    Ok(revision_id)
}
