//! Owner-only Manage Posts read models, filters, and selection snapshots.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use sqlx::{Decode, Result, Row, Type};

use crate::posts::cursors::CollectionCursor;
use crate::posts::lifecycle::post_lifecycle;
use crate::posts::models::PostLifecycle;
use crate::posts::search::{PostMutationVersion, StoredPostSearchText};
use common::ids::{AudienceId, PostId};
use common::pagination::PageSize;
use common::post_search::normalize_post_search_query;
use common::post_summary::PostSummary;
use common::post_title::PostTitle;
use common::render::RenderedHtml;
use common::slug::Slug;
use common::time::UtcInstant;
use common::visibility::{AudienceTarget, TargetKind};

/// Publication-state facet for an owner-only management query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PostManagementStateFilter {
    #[default]
    All,
    Draft,
    Scheduled,
    Published,
}

/// Audience-target facet for an owner-only management query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PostManagementAudienceFilter {
    #[default]
    All,
    Public,
    Subscribers,
    Private,
    Named(AudienceId),
}

/// Storage-filtered management request, including one bounded keyset page.
#[derive(Clone, Debug)]
pub struct PostManagementRequest {
    pub state: PostManagementStateFilter,
    pub audience: PostManagementAudienceFilter,
    pub cursor: Option<CollectionCursor>,
    pub page_size: PageSize,
    pub now: UtcInstant,
    pub(crate) search: Option<StoredPostSearchText>,
}

impl PostManagementRequest {
    #[must_use]
    pub fn new(
        state: PostManagementStateFilter,
        audience: PostManagementAudienceFilter,
        search: &str,
        cursor: Option<CollectionCursor>,
        page_size: PageSize,
        now: UtcInstant,
    ) -> Self {
        let search = normalize_post_search_query(search);
        Self {
            state,
            audience,
            cursor,
            page_size,
            now,
            search: (!search.is_empty()).then(|| search.into()),
        }
    }
}

/// Compact owner-only row backing the Manage Posts UI.
#[derive(Clone, Debug)]
pub struct ManagedPostRecord {
    pub post_id: PostId,
    pub mutation_version: PostMutationVersion,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub rendered_html: RenderedHtml,
    pub summary: Option<PostSummary>,
    pub lifecycle: PostLifecycle,
    pub audiences: Vec<AudienceTarget>,
    pub updated_at: UtcInstant,
}

/// One bounded page ordered by `updated_at DESC, post_id DESC`.
#[derive(Clone, Debug)]
pub struct PostManagementPage {
    pub posts: Vec<ManagedPostRecord>,
    pub next_cursor: Option<CollectionCursor>,
}

/// Immutable target identity carried from confirmation into execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BulkSelectionTarget {
    pub post_id: PostId,
    pub mutation_version: PostMutationVersion,
}

/// Exact canonically ordered targets resolved at confirmation time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagementSelectionSnapshot {
    pub targets: Vec<BulkSelectionTarget>,
}

impl ManagementSelectionSnapshot {
    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.targets.len()
    }
}

/// One atomic operation over an exact management snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BulkPostOperation {
    ChangeAudience(Vec<AudienceTarget>),
    Delete,
}

/// Counts returned only after the complete bulk transaction commits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BulkPostMutationResult {
    pub selected_count: usize,
    pub changed_count: usize,
}

/// Why an exact management snapshot could not be applied.
#[derive(Debug, thiserror::Error)]
pub enum BulkPostMutationError {
    #[error("one or more selected Posts are missing, unavailable, or stale")]
    SnapshotConflict,
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Selection source resolved by storage under owner and active-row predicates.
#[derive(Clone, Debug)]
pub enum PostSelectionIntent {
    Explicit(Vec<PostId>),
    AllMatching(PostManagementRequest),
}

/// Failure to resolve an exact owner-scoped active selection.
#[derive(Debug, thiserror::Error)]
pub enum ResolvePostSelectionError {
    #[error("one or more selected Posts are unavailable")]
    Unavailable,
    #[error("failed to resolve selected Posts")]
    Internal(#[from] sqlx::Error),
}

/// Escaped literal substring pattern for SQL `LIKE ... ESCAPE '\\'`.
#[derive(Clone, Debug, macros::SqlxBridge)]
pub(crate) struct StoredPostSearchPattern(String);

impl StoredPostSearchPattern {
    pub(crate) fn from_search(search: &StoredPostSearchText) -> Self {
        let escaped = search
            .as_str()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        Self(format!("%{escaped}%"))
    }
}

/// Validated JSON aggregate of a Post's complete audience target set.
#[derive(Debug, macros::SqlxBridge)]
#[sqlx_bridge(text)]
struct SerializedPostAudiences(ParsedPostAudiences);

#[derive(Debug)]
struct ParsedPostAudiences(Vec<PostAudienceJson>);

impl fmt::Display for ParsedPostAudiences {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let json = serde_json::to_string(&self.0).map_err(|_| fmt::Error)?;
        formatter.write_str(&json)
    }
}

impl FromStr for SerializedPostAudiences {
    type Err = serde_json::Error;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(ParsedPostAudiences(serde_json::from_str(value)?)))
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct PostAudienceJson {
    target_kind: TargetKind,
    audience_id: Option<AudienceId>,
}

impl SerializedPostAudiences {
    fn into_targets(self) -> Vec<AudienceTarget> {
        self.0
            .0
            .into_iter()
            .filter_map(|row| match (row.target_kind, row.audience_id) {
                (TargetKind::Public, None) => Some(AudienceTarget::Public),
                (TargetKind::Subscribers, None) => Some(AudienceTarget::Subscribers),
                (TargetKind::Named, Some(audience_id)) => Some(AudienceTarget::Named(audience_id)),
                _ => None,
            })
            .collect()
    }
}

/// Complete locked pre-mutation evidence for one exact bulk target.
pub(crate) struct BulkLockedPost {
    pub(crate) record: super::PostRecord,
    pub(crate) mutation_version: PostMutationVersion,
    pub(crate) audiences: Vec<AudienceTarget>,
}

impl<'r, R> sqlx::FromRow<'r, R> for BulkLockedPost
where
    R: Row,
    &'r str: sqlx::ColumnIndex<R>,
    super::PostRecord: sqlx::FromRow<'r, R>,
    PostMutationVersion: Decode<'r, R::Database> + Type<R::Database>,
    SerializedPostAudiences: Decode<'r, R::Database> + Type<R::Database>,
{
    fn from_row(row: &'r R) -> Result<Self> {
        Ok(Self {
            record: super::PostRecord::from_row(row)?,
            mutation_version: row.try_get::<PostMutationVersion, _>("mutation_version")?,
            audiences: row
                .try_get::<SerializedPostAudiences, _>("audiences")?
                .into_targets(),
        })
    }
}

pub(crate) struct ManagedPostRow {
    pub(crate) post_id: PostId,
    mutation_version: PostMutationVersion,
    title: Option<PostTitle>,
    slug: Slug,
    rendered_html: RenderedHtml,
    summary: Option<PostSummary>,
    pub(crate) updated_at: UtcInstant,
    published_at: Option<UtcInstant>,
    audiences: SerializedPostAudiences,
}

impl ManagedPostRow {
    pub(crate) fn into_record(self, now: UtcInstant) -> ManagedPostRecord {
        ManagedPostRecord {
            post_id: self.post_id,
            mutation_version: self.mutation_version,
            title: self.title,
            slug: self.slug,
            rendered_html: self.rendered_html,
            summary: self.summary,
            lifecycle: post_lifecycle(None, self.published_at, now),
            audiences: self.audiences.into_targets(),
            updated_at: self.updated_at,
        }
    }
}

impl<'r, R> sqlx::FromRow<'r, R> for ManagedPostRow
where
    R: Row,
    &'r str: sqlx::ColumnIndex<R>,
    PostId: Decode<'r, R::Database> + Type<R::Database>,
    PostMutationVersion: Decode<'r, R::Database> + Type<R::Database>,
    PostTitle: Decode<'r, R::Database> + Type<R::Database>,
    Slug: Decode<'r, R::Database> + Type<R::Database>,
    RenderedHtml: Decode<'r, R::Database> + Type<R::Database>,
    PostSummary: Decode<'r, R::Database> + Type<R::Database>,
    UtcInstant: Decode<'r, R::Database> + Type<R::Database>,
    SerializedPostAudiences: Decode<'r, R::Database> + Type<R::Database>,
{
    fn from_row(row: &'r R) -> Result<Self> {
        Ok(Self {
            post_id: row.try_get::<PostId, _>("post_id")?,
            mutation_version: row.try_get::<PostMutationVersion, _>("mutation_version")?,
            title: row.try_get::<Option<PostTitle>, _>("title")?,
            slug: row.try_get::<Slug, _>("slug")?,
            rendered_html: row.try_get::<RenderedHtml, _>("rendered_html")?,
            summary: row.try_get::<Option<PostSummary>, _>("summary")?,
            updated_at: row.try_get::<UtcInstant, _>("updated_at")?,
            published_at: row.try_get::<Option<UtcInstant>, _>("published_at")?,
            audiences: row.try_get::<SerializedPostAudiences, _>("audiences")?,
        })
    }
}
