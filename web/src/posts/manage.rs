//! Manage Posts wire models and owner-only read/snapshot endpoints.

use serde::{Deserialize, Serialize};

use crate::posts::UnpublishedPostLabel;
use common::ids::{AudienceId, PostId};
use common::post_title::PostTitle;
use common::slug::Slug;
use common::time::UtcInstant;
use common::visibility::AudienceSelection;

#[cfg(feature = "server")]
use {
    crate::{auth, error::InternalError},
    common::pagination::PageSize,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{
        self, AudienceStorage, FeedEventStorage, ManagedPostRecord, PostManagementRequest,
        PostSelectionIntent, PostStorage, ResolvePostSelectionError, WriteScope,
    },
};

/// Publication-state facet offered by Manage Posts.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagePublicationState {
    #[default]
    All,
    Draft,
    Scheduled,
    Published,
}

/// Audience-target facet offered by Manage Posts.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "audience_id", rename_all = "snake_case")]
pub enum ManageAudienceFilter {
    #[default]
    All,
    Public,
    Subscribers,
    Private,
    Named(AudienceId),
}

/// Complete persisted target set for one compact row.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "audience_id", rename_all = "snake_case")]
pub enum ManagedAudienceTarget {
    Public,
    Subscribers,
    Named(AudienceId),
}

/// Stable keyset cursor for the management ordering.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagePostsCursor {
    pub updated_at: UtcInstant,
    pub post_id: PostId,
}

/// Request-clock-relative lifecycle shown in one management row.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedPostLifecycle {
    Draft,
    Scheduled,
    Published,
}

/// Compact owner-only Post row.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedPost {
    pub post_id: PostId,
    pub mutation_version: i64,
    pub title: Option<PostTitle>,
    pub fallback_label: UnpublishedPostLabel,
    pub slug: Slug,
    pub lifecycle: ManagedPostLifecycle,
    pub audiences: Vec<ManagedAudienceTarget>,
    pub updated_at: UtcInstant,
}

/// One bounded management page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagePostsPage {
    pub posts: Vec<ManagedPost>,
    pub next_cursor: Option<ManagePostsCursor>,
    pub has_more: bool,
}

/// Selection source resolved when confirmation opens.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManageSelectionIntent {
    Explicit {
        post_ids: Vec<PostId>,
    },
    AllMatching {
        state: ManagePublicationState,
        audience: ManageAudienceFilter,
        search: String,
    },
}

/// One exact Post identity and concurrency token.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BulkSelectionTarget {
    pub post_id: PostId,
    pub mutation_version: i64,
}

/// Immutable target set confirmed before execution.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagementSelectionSnapshot {
    pub targets: Vec<BulkSelectionTarget>,
    pub selected_count: usize,
}

/// Operation confirmed against an immutable selection snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BulkManageOperation {
    ChangeAudience { audience: AudienceSelection },
    Delete { confirmed_count: Option<usize> },
}

/// Counts for a transaction confirmed committed by storage.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BulkManageResult {
    pub selected_count: usize,
    pub changed_count: usize,
}

#[cfg(feature = "server")]
fn storage_state(state: ManagePublicationState) -> storage::PostManagementStateFilter {
    match state {
        ManagePublicationState::All => storage::PostManagementStateFilter::All,
        ManagePublicationState::Draft => storage::PostManagementStateFilter::Draft,
        ManagePublicationState::Scheduled => storage::PostManagementStateFilter::Scheduled,
        ManagePublicationState::Published => storage::PostManagementStateFilter::Published,
    }
}

#[cfg(feature = "server")]
fn storage_audience(audience: ManageAudienceFilter) -> storage::PostManagementAudienceFilter {
    match audience {
        ManageAudienceFilter::All => storage::PostManagementAudienceFilter::All,
        ManageAudienceFilter::Public => storage::PostManagementAudienceFilter::Public,
        ManageAudienceFilter::Subscribers => storage::PostManagementAudienceFilter::Subscribers,
        ManageAudienceFilter::Private => storage::PostManagementAudienceFilter::Private,
        ManageAudienceFilter::Named(audience_id) => {
            storage::PostManagementAudienceFilter::Named(audience_id)
        }
    }
}

#[cfg(feature = "server")]
fn storage_request(
    state: ManagePublicationState,
    audience: ManageAudienceFilter,
    search: &str,
    cursor: Option<ManagePostsCursor>,
    page_size: PageSize,
    now: UtcInstant,
) -> PostManagementRequest {
    PostManagementRequest::new(
        storage_state(state),
        storage_audience(audience),
        search,
        cursor.map(|cursor| storage::CollectionCursor {
            updated_at: cursor.updated_at,
            post_id: cursor.post_id,
        }),
        page_size,
        now,
    )
}

#[cfg(feature = "server")]
fn managed_post(post: ManagedPostRecord) -> ManagedPost {
    let lifecycle = match post.lifecycle {
        storage::PostLifecycle::Draft => ManagedPostLifecycle::Draft,
        storage::PostLifecycle::Scheduled => ManagedPostLifecycle::Scheduled,
        storage::PostLifecycle::Published => ManagedPostLifecycle::Published,
        storage::PostLifecycle::Deleted => {
            unreachable!("management storage excludes Deleted Posts")
        }
    };
    let audiences = post
        .audiences
        .into_iter()
        .filter_map(|audience| match audience {
            common::visibility::AudienceTarget::Public => Some(ManagedAudienceTarget::Public),
            common::visibility::AudienceTarget::Subscribers => {
                Some(ManagedAudienceTarget::Subscribers)
            }
            common::visibility::AudienceTarget::Named(audience_id) => {
                Some(ManagedAudienceTarget::Named(audience_id))
            }
            common::visibility::AudienceTarget::Private => None,
        })
        .collect();
    ManagedPost {
        post_id: post.post_id,
        mutation_version: post.mutation_version.value(),
        title: post.title,
        fallback_label: super::api::compact_fallback_label(
            post.summary,
            &post.rendered_html,
            &post.slug,
        ),
        slug: post.slug,
        lifecycle,
        audiences,
        updated_at: post.updated_at,
    }
}

/// Lists one bounded, fully storage-filtered page for the authenticated User.
#[cfg(feature = "server")]
pub(super) async fn list_managed_posts_impl(
    state: ManagePublicationState,
    audience: ManageAudienceFilter,
    search: String,
    cursor: Option<ManagePostsCursor>,
    limit: Option<PageSize>,
) -> Result<ManagePostsPage, InternalError> {
    let auth = auth::require_auth().await?;
    let now = UtcInstant::now();
    let page = expect_context::<Arc<dyn PostStorage>>()
        .list_managed_posts(
            auth.user_id,
            &storage_request(
                state,
                audience,
                &search,
                cursor,
                limit.unwrap_or_default(),
                now,
            ),
        )
        .await
        .map_err(InternalError::storage)?;
    let has_more = page.next_cursor.is_some();
    Ok(ManagePostsPage {
        posts: page.posts.into_iter().map(managed_post).collect(),
        next_cursor: page.next_cursor.map(|cursor| ManagePostsCursor {
            updated_at: cursor.updated_at,
            post_id: cursor.post_id,
        }),
        has_more,
    })
}

/// Resolves the current intent into exact immutable targets for confirmation.
#[cfg(feature = "server")]
pub(super) async fn resolve_management_selection_impl(
    intent: ManageSelectionIntent,
) -> Result<ManagementSelectionSnapshot, InternalError> {
    let auth = auth::require_auth().await?;
    let storage_intent = match intent {
        ManageSelectionIntent::Explicit { post_ids } => PostSelectionIntent::Explicit(post_ids),
        ManageSelectionIntent::AllMatching {
            state,
            audience,
            search,
        } => PostSelectionIntent::AllMatching(storage_request(
            state,
            audience,
            &search,
            None,
            PageSize::default(),
            UtcInstant::now(),
        )),
    };
    let snapshot = expect_context::<Arc<dyn PostStorage>>()
        .resolve_post_selection(auth.user_id, &storage_intent)
        .await
        .map_err(|error| match error {
            ResolvePostSelectionError::Unavailable => {
                InternalError::conflict("Selection changed; refresh Manage Posts and try again")
            }
            ResolvePostSelectionError::Internal(error) => InternalError::storage(error),
        })?;
    let targets = snapshot
        .targets
        .into_iter()
        .map(|target| BulkSelectionTarget {
            post_id: target.post_id,
            mutation_version: target.mutation_version.value(),
        })
        .collect::<Vec<_>>();
    Ok(ManagementSelectionSnapshot {
        selected_count: targets.len(),
        targets,
    })
}

#[cfg(feature = "server")]
fn storage_snapshot(
    snapshot: ManagementSelectionSnapshot,
) -> Result<storage::ManagementSelectionSnapshot, InternalError> {
    let ordered = snapshot
        .targets
        .windows(2)
        .all(|pair| pair[0].post_id < pair[1].post_id);
    if snapshot.selected_count != snapshot.targets.len() || !ordered {
        return Err(InternalError::conflict(
            "Selection changed; refresh Manage Posts and try again",
        ));
    }
    let targets = snapshot
        .targets
        .into_iter()
        .map(|target| {
            let mutation_version = storage::PostMutationVersion::from_value(
                target.mutation_version,
            )
            .ok_or_else(|| {
                InternalError::conflict("Selection changed; refresh Manage Posts and try again")
            })?;
            Ok(storage::BulkSelectionTarget {
                post_id: target.post_id,
                mutation_version,
            })
        })
        .collect::<Result<Vec<_>, InternalError>>()?;
    Ok(storage::ManagementSelectionSnapshot { targets })
}

/// Executes one confirmed exact snapshot inside a single write scope.
#[cfg(feature = "server")]
pub(super) async fn execute_management_operation_impl(
    snapshot: ManagementSelectionSnapshot,
    operation: BulkManageOperation,
) -> Result<common::MutationOutcome<BulkManageResult>, InternalError> {
    let auth = auth::require_auth().await?;
    if let BulkManageOperation::Delete { confirmed_count } = &operation
        && snapshot.selected_count >= 10
        && *confirmed_count != Some(snapshot.selected_count)
    {
        return Err(InternalError::validation(
            "Enter the exact selected Post count to confirm deletion",
        ));
    }
    let operation = match operation {
        BulkManageOperation::ChangeAudience { audience } => {
            let targets = common::visibility::audience_selection_to_targets(&audience);
            let audiences = expect_context::<Arc<dyn AudienceStorage>>();
            storage::validate_named_audience_targets(audiences.as_ref(), auth.user_id, &targets)
                .await
                .map_err(InternalError::from)?;
            storage::BulkPostOperation::ChangeAudience(targets)
        }
        BulkManageOperation::Delete { .. } => storage::BulkPostOperation::Delete,
    };
    let snapshot = storage_snapshot(snapshot)?;
    let outcome = storage::perform_bulk_post_mutation(
        &expect_context::<WriteScope>(),
        expect_context::<Arc<dyn PostStorage>>(),
        expect_context::<Arc<dyn FeedEventStorage>>(),
        auth.user_id,
        snapshot,
        operation,
        UtcInstant::now(),
    )
    .await
    .map_err(|error| match error {
        storage::BulkPostMutationError::SnapshotConflict => {
            InternalError::conflict("Selection changed; refresh Manage Posts and try again")
        }
        storage::BulkPostMutationError::Db(error) => InternalError::storage(error),
    })?;
    Ok(outcome.map(|result| BulkManageResult {
        selected_count: result.selected_count,
        changed_count: result.changed_count,
    }))
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use common::test_support::{parse_post_title, parse_slug, parse_utc_instant, rendered_html};
    use common::visibility::AudienceTarget;

    #[test]
    fn management_filter_conversions_cover_every_variant() {
        assert_eq!(
            storage_state(ManagePublicationState::All),
            storage::PostManagementStateFilter::All
        );
        assert_eq!(
            storage_state(ManagePublicationState::Draft),
            storage::PostManagementStateFilter::Draft
        );
        assert_eq!(
            storage_state(ManagePublicationState::Scheduled),
            storage::PostManagementStateFilter::Scheduled
        );
        assert_eq!(
            storage_state(ManagePublicationState::Published),
            storage::PostManagementStateFilter::Published
        );
        assert_eq!(
            storage_audience(ManageAudienceFilter::All),
            storage::PostManagementAudienceFilter::All
        );
        assert_eq!(
            storage_audience(ManageAudienceFilter::Public),
            storage::PostManagementAudienceFilter::Public
        );
        assert_eq!(
            storage_audience(ManageAudienceFilter::Subscribers),
            storage::PostManagementAudienceFilter::Subscribers
        );
        assert_eq!(
            storage_audience(ManageAudienceFilter::Private),
            storage::PostManagementAudienceFilter::Private
        );
        let named = AudienceId::from(7);
        assert_eq!(
            storage_audience(ManageAudienceFilter::Named(named)),
            storage::PostManagementAudienceFilter::Named(named)
        );
    }

    #[test]
    fn storage_request_maps_cursor_and_managed_post_maps_lifecycle_and_audiences() {
        let now = parse_utc_instant("2026-09-23T12:00:00Z");
        let cursor = ManagePostsCursor {
            updated_at: now,
            post_id: PostId::from(3),
        };
        let request = storage_request(
            ManagePublicationState::Published,
            ManageAudienceFilter::Public,
            " query ",
            Some(cursor),
            PageSize::default(),
            now,
        );
        assert!(request.cursor.is_some());

        let named = AudienceId::from(8);
        let record = |lifecycle| ManagedPostRecord {
            post_id: PostId::from(4),
            mutation_version: storage::PostMutationVersion::initial(),
            title: Some(parse_post_title("Managed")),
            slug: parse_slug("managed"),
            rendered_html: rendered_html("<p>managed</p>"),
            summary: None,
            lifecycle,
            audiences: vec![
                AudienceTarget::Public,
                AudienceTarget::Subscribers,
                AudienceTarget::Named(named),
                AudienceTarget::Private,
            ],
            updated_at: now,
        };
        let draft = managed_post(record(storage::PostLifecycle::Draft));
        assert_eq!(draft.lifecycle, ManagedPostLifecycle::Draft);
        assert_eq!(
            draft.audiences,
            vec![
                ManagedAudienceTarget::Public,
                ManagedAudienceTarget::Subscribers,
                ManagedAudienceTarget::Named(named),
            ]
        );
        assert_eq!(
            managed_post(record(storage::PostLifecycle::Scheduled)).lifecycle,
            ManagedPostLifecycle::Scheduled
        );
        assert_eq!(
            managed_post(record(storage::PostLifecycle::Published)).lifecycle,
            ManagedPostLifecycle::Published
        );
    }

    #[test]
    fn storage_snapshot_requires_exact_ordered_positive_versions() {
        let target = |post_id, mutation_version| BulkSelectionTarget {
            post_id: PostId::from(post_id),
            mutation_version,
        };
        assert!(
            storage_snapshot(ManagementSelectionSnapshot {
                selected_count: 2,
                targets: vec![target(1, 1), target(2, 2)],
            })
            .is_ok()
        );
        assert!(
            storage_snapshot(ManagementSelectionSnapshot {
                selected_count: 1,
                targets: vec![target(2, 1), target(1, 1)],
            })
            .is_err()
        );
        assert!(
            storage_snapshot(ManagementSelectionSnapshot {
                selected_count: 1,
                targets: vec![target(1, 0)],
            })
            .is_err()
        );
    }
}
