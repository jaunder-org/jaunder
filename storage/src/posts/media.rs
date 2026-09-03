//! Persisted media-reference ownership, backfill, and locking helpers.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};
use sqlx::{Database, Encode, Executor, Pool, QueryBuilder, Result, Type};

use crate::InstanceId;
use crate::posts::models::RenderedHtml;
use crate::posts::store::PostDialect;
use crate::sql::{QueryBuilderStorageExt, QueryStorageExt};
use common::ids::{PostId, RevisionId, UserId};
use common::media::{MediaRef, MediaReference, MediaReferenceForm, MediaReferenceKind};

/// Exact retained state that names a media reference.
///
/// A revision identity is part of the persisted key: treating it as merely
/// another copy of a Post's current references would let current-row evidence
/// authorize deletion while a concurrent historical row remains protected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PersistedMediaSubject {
    /// The current durable Post state.
    Current,
    /// One immutable prior state of the Post.
    Revision(RevisionId),
}

/// The exact stored discriminator for a retained media reference subject.
#[macros::text_enum(
    sqlx,
    error = InvalidPersistedMediaSubjectKind,
    message = "invalid persisted media subject kind"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum PersistedMediaSubjectKind {
    Current,
    Revision,
}

impl PersistedMediaSubject {
    #[must_use]
    pub(crate) const fn kind(self) -> PersistedMediaSubjectKind {
        match self {
            Self::Current => PersistedMediaSubjectKind::Current,
            Self::Revision(_) => PersistedMediaSubjectKind::Revision,
        }
    }

    #[must_use]
    pub(crate) fn revision_id(self) -> RevisionId {
        match self {
            Self::Current => RevisionId::from(0),
            Self::Revision(revision_id) => revision_id,
        }
    }
}

/// The exact persisted spelling of one media reference in one retained Post subject.
///
/// This is deliberately the database key, rather than a lossy media identity:
/// foreign ownership evidence must not authorize a similarly named row.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PersistedMediaReference {
    post_id: PostId,
    subject: PersistedMediaSubject,
    owner_id: Option<UserId>,
    media: MediaRef,
    kind: MediaReferenceKind,
    reference_form: MediaReferenceForm,
}

impl PersistedMediaReference {
    #[must_use]
    pub fn new(
        post_id: PostId,
        media: MediaRef,
        kind: MediaReferenceKind,
        reference_form: MediaReferenceForm,
    ) -> Self {
        Self::for_subject(
            post_id,
            PersistedMediaSubject::Current,
            media,
            kind,
            reference_form,
        )
    }

    #[must_use]
    pub fn for_subject(
        post_id: PostId,
        subject: PersistedMediaSubject,
        media: MediaRef,
        kind: MediaReferenceKind,
        reference_form: MediaReferenceForm,
    ) -> Self {
        Self {
            post_id,
            subject,
            owner_id: None,
            media,
            kind,
            reference_form,
        }
    }

    #[must_use]
    pub fn post_id(&self) -> PostId {
        self.post_id
    }
    #[must_use]
    pub fn subject(&self) -> PersistedMediaSubject {
        self.subject
    }
    #[must_use]
    pub fn owner_id(&self) -> Option<UserId> {
        self.owner_id
    }
    pub(crate) fn with_owner(mut self, owner_id: UserId) -> Self {
        self.owner_id = Some(owner_id);
        self
    }
    #[must_use]
    pub fn media(&self) -> &MediaRef {
        &self.media
    }
    #[must_use]
    pub fn kind(&self) -> MediaReferenceKind {
        self.kind
    }
    #[must_use]
    pub fn reference_form(&self) -> &MediaReferenceForm {
        &self.reference_form
    }
}

/// Maximum exact-reference rows examined for one live ownership decision.
///
/// The sentinel-bearing snapshot prevents a media deletion from materializing
/// unbounded author-controlled rows. Rows beyond this limit receive no foreign
/// evidence and therefore remain conservatively live.
pub const MAX_MEDIA_REFERENCE_SNAPSHOT: usize = 128;

/// The sentinel-bearing maximum row count passed to the media-reference snapshot query.
#[derive(Clone, Copy, Debug, macros::SqlxBridge)]
pub(crate) struct MediaReferenceSnapshotLimit(i64);

pub(crate) const MEDIA_REFERENCE_SNAPSHOT_QUERY_LIMIT: MediaReferenceSnapshotLimit =
    MediaReferenceSnapshotLimit(129);

/// A bounded exact-reference snapshot for one media identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaReferenceSnapshot {
    references: Vec<PersistedMediaReference>,
    has_unexamined_references: bool,
}

impl MediaReferenceSnapshot {
    #[must_use]
    pub fn new(references: Vec<PersistedMediaReference>, has_unexamined_references: bool) -> Self {
        debug_assert!(references.len() <= MAX_MEDIA_REFERENCE_SNAPSHOT);
        Self {
            references,
            has_unexamined_references,
        }
    }
    #[must_use]
    pub fn references(&self) -> &[PersistedMediaReference] {
        &self.references
    }
    #[must_use]
    pub fn has_unexamined_references(&self) -> bool {
        self.has_unexamined_references
    }
}

/// A foreign instance's signed/verified ownership assertion for one exact row.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProvenForeignReference {
    reference: PersistedMediaReference,
    expected_instance_id: InstanceId,
}

impl ProvenForeignReference {
    #[must_use]
    pub(crate) fn new(
        reference: PersistedMediaReference,
        expected_instance_id: InstanceId,
    ) -> Self {
        Self {
            reference,
            expected_instance_id,
        }
    }
    #[must_use]
    pub fn reference(&self) -> &PersistedMediaReference {
        &self.reference
    }
    #[must_use]
    pub fn expected_instance_id(&self) -> &InstanceId {
        &self.expected_instance_id
    }
}

/// Completed, network-free foreign ownership evidence for one storage decision.
#[derive(Clone, Debug)]
pub struct MediaReferenceEvidence {
    expected_instance_id: InstanceId,
    pub(crate) references: BTreeSet<ProvenForeignReference>,
}

impl MediaReferenceEvidence {
    #[must_use]
    pub fn new(expected_instance_id: InstanceId) -> Self {
        Self {
            expected_instance_id,
            references: BTreeSet::new(),
        }
    }
    pub fn insert(&mut self, proof: ProvenForeignReference) -> bool {
        if proof.expected_instance_id != self.expected_instance_id {
            return false;
        }
        self.references.insert(proof)
    }
    #[must_use]
    pub fn expected_instance_id(&self) -> &InstanceId {
        &self.expected_instance_id
    }
    #[must_use]
    pub fn references(&self) -> &BTreeSet<ProvenForeignReference> {
        &self.references
    }
    #[must_use]
    pub fn proves_foreign(&self, reference: &PersistedMediaReference) -> bool {
        self.references
            .iter()
            .any(|proof| proof.reference() == reference)
    }
}

/// A rendered-HTML snapshot and the references derived from it before a backfill write.
///
/// This keeps HTML extraction out of the backend's writer lock. The dialect re-reads the
/// snapshot while holding its write discipline before it installs these rows.
#[derive(Debug)]
pub struct PostMediaReferenceBackfill {
    pub(crate) post_id: PostId,
    pub(crate) rendered_html: String,
    pub(crate) references: Vec<MediaReference>,
}

/// Appends a dynamic evidence relation for one ownership decision.
pub(crate) fn push_media_reference_evidence_cte<'a, DB>(
    query: &mut QueryBuilder<'a, DB>,
    evidence: &'a MediaReferenceEvidence,
) where
    DB: Database,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q InstanceId: Encode<'q, DB> + Type<DB>,
{
    query.push("WITH foreign_evidence \
         (post_id, subject_kind, revision_id, source, sha256, filename, reference_kind, reference_form, expected_instance_id) AS (");
    if evidence.references.is_empty() {
        query.push(
            "SELECT CAST(NULL AS BIGINT), CAST(NULL AS TEXT), CAST(NULL AS BIGINT), \
             CAST(NULL AS TEXT), CAST(NULL AS TEXT), CAST(NULL AS TEXT), CAST(NULL AS TEXT), \
             CAST(NULL AS TEXT), CAST(NULL AS TEXT) WHERE FALSE",
        );
    } else {
        query.push("VALUES ");
        for (index, proof) in evidence.references.iter().enumerate() {
            if index > 0 {
                query.push(", ");
            }
            let reference = proof.reference();
            query
                .push("(")
                .push_storage_bind(reference.post_id())
                .push(", ")
                .push_storage_bind(reference.subject().kind())
                .push(", ")
                .push_storage_bind(reference.subject().revision_id())
                .push(", ")
                .push_storage_bind(reference.media().source)
                .push(", ")
                .push_storage_bind(reference.media().sha256.clone())
                .push(", ")
                .push_storage_bind(reference.media().filename.clone())
                .push(", ")
                .push_storage_bind(reference.kind())
                .push(", ")
                .push_storage_bind(reference.reference_form().clone())
                .push(", ")
                .push_storage_bind(proof.expected_instance_id())
                .push(")");
        }
    }
    query.push(") ");
}

pub(crate) fn push_live_media_reference_predicate<'a, DB>(
    query: &mut QueryBuilder<'a, DB>,
    current_instance_id: &'a InstanceId,
) where
    DB: Database,
    for<'q> &'q InstanceId: Encode<'q, DB> + Type<DB>,
{
    query.push(
        " AND NOT EXISTS (\
           SELECT 1 FROM foreign_evidence evidence \
           WHERE evidence.post_id = pm.post_id \
             AND evidence.subject_kind = pm.subject_kind \
             AND evidence.revision_id = pm.revision_id \
             AND evidence.source = pm.source \
             AND evidence.sha256 = pm.sha256 \
             AND evidence.filename = pm.filename \
             AND evidence.reference_kind = pm.reference_kind \
             AND evidence.reference_form = pm.reference_form \
             AND evidence.expected_instance_id = ",
    );
    query.push_storage_bind(current_instance_id);
    query.push(")");
}

pub(crate) fn push_owner_media_reference_from_where<DB>(
    query: &mut QueryBuilder<'_, DB>,
    user_id: UserId,
    media: &MediaRef,
) where
    DB: Database,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
{
    query
        .push(" FROM post_media pm JOIN posts p ON p.post_id = pm.post_id WHERE p.user_id = ")
        .push_storage_bind(user_id)
        .push(" AND pm.source = ")
        .push_storage_bind(media.source)
        .push(" AND pm.sha256 = ")
        .push_storage_bind(media.sha256.clone())
        .push(" AND pm.filename = ")
        .push_storage_bind(media.filename.clone());
}

pub(crate) fn push_any_media_reference_from_where<DB>(
    query: &mut QueryBuilder<'_, DB>,
    media: &MediaRef,
) where
    DB: Database,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
{
    query
        .push(" FROM post_media pm JOIN posts p ON p.post_id = pm.post_id WHERE pm.source = ")
        .push_storage_bind(media.source)
        .push(" AND pm.sha256 = ")
        .push_storage_bind(media.sha256.clone())
        .push(" AND pm.filename = ")
        .push_storage_bind(media.filename.clone());
}

pub(crate) fn push_other_owner_media_reference_from_where<DB>(
    query: &mut QueryBuilder<'_, DB>,
    user_id: UserId,
    media: &MediaRef,
) where
    DB: Database,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
{
    query
        .push(" FROM post_media pm JOIN posts p ON p.post_id = pm.post_id WHERE p.user_id <> ")
        .push_storage_bind(user_id)
        .push(" AND pm.source = ")
        .push_storage_bind(media.source)
        .push(" AND pm.sha256 = ")
        .push_storage_bind(media.sha256.clone())
        .push(" AND pm.filename = ")
        .push_storage_bind(media.filename.clone());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, macros::SqlxBridge)]
pub(crate) struct MediaAdvisoryLockKey(i64);

#[must_use]
pub(crate) fn media_advisory_lock_key(media: &MediaRef) -> MediaAdvisoryLockKey {
    let mut digest = Sha256::new();
    digest.update(media.source.to_string().as_bytes());
    digest.update([0]);
    digest.update(media.sha256.to_string().as_bytes());
    digest.update([0]);
    digest.update(media.filename.to_string().as_bytes());
    let digest: [u8; 32] = digest.finalize().into();
    MediaAdvisoryLockKey(i64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]))
}

#[must_use]
pub(crate) fn media_advisory_lock_keys(
    media: impl IntoIterator<Item = MediaRef>,
) -> Vec<MediaAdvisoryLockKey> {
    media
        .into_iter()
        .map(|media| media_advisory_lock_key(&media))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[must_use]
pub(crate) fn media_lock_set(references: &[MediaReference]) -> BTreeSet<MediaRef> {
    references
        .iter()
        .map(MediaReference::media)
        .cloned()
        .collect()
}

pub(crate) async fn backfill_post_media_references<DB>(pool: &Pool<DB>) -> Result<()>
where
    DB: PostDialect,
    (PostId, RenderedHtml): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let posts: Vec<(PostId, RenderedHtml)> = sqlx::query_as(
        "SELECT p.post_id, p.rendered_html
         FROM posts p
         WHERE EXISTS (
             SELECT 1 FROM post_media pm
             WHERE pm.post_id = p.post_id AND pm.reference_kind = 'legacy'
         )
         ORDER BY p.post_id",
    )
    .fetch_all(pool)
    .await?;
    let candidates: Vec<PostMediaReferenceBackfill> = posts
        .into_iter()
        .map(|(post_id, rendered_html)| PostMediaReferenceBackfill {
            references: host::render::extract_media_refs(rendered_html.as_ref()),
            post_id,
            rendered_html: rendered_html.to_string(),
        })
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }
    DB::apply_post_media_reference_backfill(pool, &candidates).await
}

pub(crate) async fn replace_post_media<DB>(
    conn: &mut DB::Connection,
    post_id: PostId,
    media: &[MediaReference],
) -> Result<()>
where
    DB: PostDialect,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let rows = media_reference_rows([(post_id, media)]);
    sqlx::query(DB::DELETE_POST_MEDIA)
        .bind_storage(post_id)
        .execute(&mut *conn)
        .await?;
    DB::insert_post_media_rows(conn, rows).await
}

pub(crate) async fn replace_legacy_post_media<DB>(
    conn: &mut DB::Connection,
    candidates: &[PostMediaReferenceBackfill],
) -> Result<()>
where
    DB: PostDialect,
    for<'q> i64: Encode<'q, DB> + Type<DB>,
    String: Type<DB>,
    for<'q> String: Encode<'q, DB>,
    for<'q> &'q str: Encode<'q, DB> + Type<DB>,
    for<'c> &'c mut DB::Connection: Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    sqlx::query(
        "DELETE FROM post_media
         WHERE subject_kind = 'current' AND revision_id = 0
           AND post_id IN (
             SELECT post_id FROM post_media
             WHERE subject_kind = 'current' AND revision_id = 0 AND reference_kind = 'legacy'
           )",
    )
    .execute(&mut *conn)
    .await?;
    DB::insert_post_media_rows(
        conn,
        media_reference_rows(
            candidates
                .iter()
                .map(|candidate| (candidate.post_id, candidate.references.as_slice())),
        ),
    )
    .await
}

fn media_reference_rows<'a>(
    references: impl IntoIterator<Item = (PostId, &'a [MediaReference])>,
) -> BTreeSet<(PostId, MediaRef, MediaReferenceKind, MediaReferenceForm)> {
    references
        .into_iter()
        .flat_map(|(post_id, references)| {
            references.iter().map(move |reference| {
                (
                    post_id,
                    reference.media().clone(),
                    reference.kind(),
                    reference.reference_form().clone(),
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{media_ref_for, media_url_for};

    #[test]
    fn media_advisory_lock_keys_are_sorted_and_deduplicated() {
        let first = media_ref_for("first.jpg");
        let second = media_ref_for("second.jpg");
        let forward = media_advisory_lock_keys([first.clone(), second.clone()]);
        let reverse_with_duplicate =
            media_advisory_lock_keys([second.clone(), first.clone(), second]);

        assert_eq!(
            forward, reverse_with_duplicate,
            "opposite-order updates acquire precisely the same lock sequence"
        );
        assert_eq!(forward.len(), 2, "one lock per distinct media identity");
        assert!(
            forward.windows(2).all(|pair| pair[0] < pair[1]),
            "advisory lock keys are strictly ascending"
        );
    }

    #[test]
    fn foreign_evidence_rejects_another_instance_and_encodes_multiple_proofs() {
        let expected: InstanceId = "123e4567-e89b-12d3-a456-426614174000"
            .parse()
            .expect("canonical instance ID");
        let other: InstanceId = "123e4567-e89b-12d3-a456-426614174001"
            .parse()
            .expect("canonical instance ID");
        let parsed = common::media::parse_media_url(&media_url_for("evidence.jpg"))
            .expect("media form parses");
        let first = PersistedMediaReference::new(
            PostId::from(1),
            parsed.media().clone(),
            parsed.kind(),
            parsed.reference_form().clone(),
        );
        let second = PersistedMediaReference::new(
            PostId::from(2),
            parsed.media().clone(),
            parsed.kind(),
            parsed.reference_form().clone(),
        );
        let mut evidence = MediaReferenceEvidence::new(expected.clone());
        assert!(!evidence.insert(ProvenForeignReference::new(first, other)));
        assert!(evidence.insert(ProvenForeignReference::new(second, expected.clone())));
        assert!(evidence.insert(ProvenForeignReference::new(
            PersistedMediaReference::new(
                PostId::from(1),
                parsed.media().clone(),
                parsed.kind(),
                parsed.reference_form().clone(),
            ),
            expected,
        )));

        let mut query = QueryBuilder::<sqlx::Sqlite>::new("");
        push_media_reference_evidence_cte(&mut query, &evidence);
        assert!(
            query.sql().contains("), ("),
            "multiple proofs must be separate CTE rows"
        );
    }
}
