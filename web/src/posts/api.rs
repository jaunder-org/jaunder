//! Posts wire types and `#[server]` endpoints (ADR-0070, amended #530).
//!
//! The single-post lifecycle DTOs and their `#[server]` fns live here; the
//! cursor-paginated listing surface is its own vertical (`crate::timeline`).
//! `posts/mod.rs` is wiring only and re-exports these under the stable
//! `crate::posts::…` paths that external call sites and the server-fn registrar
//! depend on.

use leptos::server_fn::codec::Json;
use serde::{Deserialize, Serialize};

use common::{
    MutationOutcome,
    ids::{PostId, RevisionId},
    pagination::PageSize,
    post_body::PostBody,
    post_summary::PostSummary,
    post_title::PostTitle,
    render::PostFormat,
    revision_history::RevisionHistoryDetail,
    root_relative_url::RootRelativeUrl,
    slug::Slug,
    tag::TagLabel,
    time::{PermalinkDate, UtcInstant},
    trace_field::TraceField,
    username::Username,
    visibility::AudienceSelection,
};

use common::seed::{AuthoredPost, Page, PageCursor};

use crate::error::WebResult;

// The audience-picker DTO and its converters live in `common::visibility` (beside
// `AudienceBase`/`AudienceTarget`); the server fn bodies below use these two to
// translate the wire `AudienceSelection` to/from the domain `AudienceTarget`s. The
// calls are server-only (inside the macro-supplied boundary), so the import is
// gated to match.
#[cfg(feature = "server")]
use common::revision_history::{RevisionHistoryAudience, RevisionHistoryTag};
#[cfg(feature = "server")]
use common::visibility::{self, AudienceTarget};

// Server-only imports for the #[server] fn bodies (gated on `feature = "server"`).
#[cfg(feature = "server")]
use {
    super::server,
    crate::{
        auth,
        error::{InternalError, from_write_scope_error},
        viewer,
    },
    common::{
        org::{
            self, OrgNormalization, OrgOperation, OrgStructuredMetadata, Presence, PublicationState,
        },
        tag::{self, Tag},
    },
    host::{feed, metrics},
    leptos::prelude::*,
    std::{collections::BTreeSet, sync::Arc},
    storage::{
        self, AudienceStorage, CurrentPostRevisionSummary, FeedEventStorage, MediaContentLocks,
        PerformUpdateError, PostBookkeepingExpectation, PostCreation, PostLifecycle, PostRecord,
        PostRevisionDetail, PostRevisionMetadata, PostStorage, PostUpdate, PublishUpdate,
        SiteConfigStorage, WriteScope,
    },
};

/// Builds structured lifecycle only when the transport explicitly supplied a
/// publication control. Omission lets an Org header lifecycle take effect.
#[cfg(feature = "server")]
fn structured_lifecycle(
    publish: Option<bool>,
    publish_at: Option<UtcInstant>,
    request_clock: UtcInstant,
) -> Presence<PublicationState> {
    match publish {
        None => Presence::Absent,
        Some(false) => Presence::Present(PublicationState::Draft),
        Some(true) => match publish_at {
            Some(at) if at.value() > request_clock.value() => {
                Presence::Present(PublicationState::Scheduled(at))
            }
            Some(at) => Presence::Present(PublicationState::Published(at)),
            None => Presence::Present(PublicationState::Published(request_clock)),
        },
    }
}

/// Normalizes an Org request after preserving the web wire's actual field
/// presence. Non-Org requests never reach this seam.
#[cfg(feature = "server")]
fn normalize_web_org(
    body: &PostBody,
    structured: OrgStructuredMetadata,
    operation: OrgOperation,
    request_clock: UtcInstant,
) -> Result<OrgNormalization, InternalError> {
    org::normalize_org(body.as_ref(), structured, operation, request_clock)
        .map_err(|error| InternalError::validation(error.to_string()))
}

#[cfg(feature = "server")]
async fn validate_org_audiences(
    targets: &[AudienceTarget],
    author_user_id: common::ids::UserId,
) -> Result<(), InternalError> {
    let audiences = expect_context::<Arc<dyn AudienceStorage>>();
    storage::validate_named_audience_targets(audiences.as_ref(), author_user_id, targets)
        .await
        .map_err(InternalError::from)
}

/// Authenticates History reads without disclosing whether a session or resource
/// exists. Explicit credential failures still retain their typed masked error.
#[cfg(feature = "server")]
async fn history_auth() -> Result<crate::auth::User, InternalError> {
    auth::optional_auth()
        .await?
        .ok_or_else(server::not_found_error)
}

#[cfg(feature = "server")]
fn revision_lifecycle(lifecycle: PostLifecycle) -> RevisionLifecycle {
    match lifecycle {
        PostLifecycle::Draft => RevisionLifecycle::Draft,
        PostLifecycle::Scheduled => RevisionLifecycle::Scheduled,
        PostLifecycle::Published => RevisionLifecycle::Published,
        PostLifecycle::Deleted => RevisionLifecycle::Deleted,
    }
}

#[cfg(feature = "server")]
fn revision_metadata(metadata: PostRevisionMetadata) -> RevisionHistoryMetadata {
    RevisionHistoryMetadata {
        revision_id: metadata.revision_id,
        post_id: metadata.post_id,
        title: metadata.title,
        slug: metadata.slug,
        captured_at: metadata.captured_at,
        snapshot_lifecycle: revision_lifecycle(metadata.snapshot_lifecycle),
        current_deleted: metadata.current_deleted,
    }
}

#[cfg(feature = "server")]
fn storage_revision_cursor(cursor: &RevisionHistoryCursor) -> storage::PostRevisionCursor {
    storage::PostRevisionCursor {
        revision_id: cursor.revision_id,
    }
}

#[cfg(feature = "server")]
fn revision_history_cursor(cursor: storage::PostRevisionCursor) -> RevisionHistoryCursor {
    RevisionHistoryCursor {
        revision_id: cursor.revision_id,
    }
}

#[cfg(feature = "server")]
fn current_post_history(summary: CurrentPostRevisionSummary) -> CurrentPostHistory {
    CurrentPostHistory {
        post_id: summary.post_id,
        title: summary.title,
        slug: summary.slug,
        format: summary.format,
        created_at: summary.created_at,
        updated_at: summary.updated_at,
        published_at: summary.published_at,
        deleted_at: summary.deleted_at,
        lifecycle: revision_lifecycle(summary.lifecycle),
    }
}

#[cfg(feature = "server")]
fn revision_detail(detail: PostRevisionDetail) -> RevisionHistoryDetail {
    let revision = detail.revision;
    RevisionHistoryDetail {
        revision_id: revision.revision_id,
        post_id: revision.post_id,
        title: revision.title,
        slug: revision.slug,
        body: revision.body,
        format: revision.format,
        rendered_html: revision.rendered_html,
        summary: revision.summary,
        created_at: revision.created_at,
        updated_at: revision.updated_at,
        published_at: revision.published_at,
        deleted_at: revision.deleted_at,
        captured_at: revision.captured_at,
        tags: revision
            .tags
            .into_iter()
            .map(|tag| RevisionHistoryTag {
                tag: tag.tag,
                display: tag.display,
            })
            .collect(),
        audiences: revision
            .audiences
            .into_iter()
            .map(|target| match target {
                AudienceTarget::Public => RevisionHistoryAudience {
                    kind: "public".to_owned(),
                    audience_id: None,
                },
                AudienceTarget::Private => RevisionHistoryAudience {
                    kind: "private".to_owned(),
                    audience_id: None,
                },
                AudienceTarget::Subscribers => RevisionHistoryAudience {
                    kind: "subscribers".to_owned(),
                    audience_id: None,
                },
                AudienceTarget::Named(audience_id) => RevisionHistoryAudience {
                    kind: "named".to_owned(),
                    audience_id: Some(audience_id),
                },
            })
            .collect(),
        media: revision
            .media
            .into_iter()
            .map(|reference| reference.reference_form().as_ref().to_owned())
            .collect(),
    }
}
#[cfg(feature = "server")]
fn unpublished_post_from_record(post: PostRecord) -> UnpublishedPost {
    let summary_label = post.fallback_summary_label();
    let permalink = post.permalink();
    UnpublishedPost {
        post: SavedPost {
            post_id: post.post_id,
            slug: post.slug,
            published_at: post.published_at,
            permalink,
        },
        title: post.title,
        summary_label,
        edit_url: crate::posts::edit_post_url(post.post_id),
    }
}

/// The saved post's identity, publication state, and where to find it.
///
/// One type for all four post-mutating endpoints ([`create`], [`update`],
/// [`publish`], [`unpublish`]): they answer the same question — what the post is
/// now — so a caller that handles one handles all of them. `published_at` is the
/// draft/published discriminant every consumer reads (never the instant itself),
/// which is why it stays `Option` even on the paths that always publish.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedPost {
    pub post_id: PostId,
    pub slug: Slug,
    pub published_at: Option<UtcInstant>,
    /// Canonical permalink, always present — for a draft it is the created_at-based
    /// URL the permalink view renders for the author.
    pub permalink: RootRelativeUrl,
}

/// The author-only payload used to seed the Post editor.
///
/// `fetched_at` is captured by the server with the Post so the client can
/// distinguish a scheduled Post from a live one without comparing against the
/// browser clock. The loaded editor keeps that classification for its lifetime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditPostPreview {
    pub post: AuthoredPost,
    pub fetched_at: UtcInstant,
}

/// One of the author's not-yet-public posts, as a row in a
/// [`Page<UnpublishedPost>`](Page).
///
/// Named for the predicate the listing selects on, not for "draft": `list_drafts`
/// returns true drafts (`published_at` NULL) **and** scheduled posts
/// (`published_at` in the future), so "draft" would be wrong for half the set.
///
/// The identity/publication-state quartet is the nested [`SavedPost`] every
/// post-mutating endpoint already answers with; what this type adds is what the
/// row needs to paint itself — the label and the edit action's target.
///
/// Nesting rather than collapsing the two types is deliberate: nothing converts
/// between them, and a flat union would put `title`, `summary_label`, and
/// `edit_url` on every mutation response, where nothing reads them. See
/// `docs/adr/0097-post-dto-content-weight-axis.md` (rule 3) before re-filing
/// the field overlap as duplication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnpublishedPost {
    pub post: SavedPost,
    pub title: Option<PostTitle>,
    pub summary_label: PostSummary,
    pub edit_url: RootRelativeUrl,
}

/// Opaque immutable-ID cursor for owner revision history. Unlike public-post
/// cursors, history ordering is wholly by revision ID.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionHistoryCursor {
    pub revision_id: RevisionId,
}

impl TraceField for RevisionHistoryCursor {
    type Value<'a> = RevisionId;

    fn trace_value(&self) -> Self::Value<'_> {
        self.revision_id
    }
}

/// Stable lifecycle label for history surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RevisionLifecycle {
    Draft,
    Scheduled,
    Published,
    Deleted,
}

/// One compact immutable snapshot row returned by either history list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionHistoryMetadata {
    pub revision_id: RevisionId,
    pub post_id: PostId,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub captured_at: UtcInstant,
    pub snapshot_lifecycle: RevisionLifecycle,
    pub current_deleted: bool,
}

/// A cursor-paginated owner history list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionHistoryPage {
    pub revisions: Vec<RevisionHistoryMetadata>,
    pub next_cursor: Option<RevisionHistoryCursor>,
    pub has_more: bool,
}

/// The current (not revision) heading for a Post history view.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CurrentPostHistory {
    pub post_id: PostId,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub format: PostFormat,
    pub created_at: UtcInstant,
    pub updated_at: UtcInstant,

    pub published_at: Option<UtcInstant>,
    pub deleted_at: Option<UtcInstant>,
    pub lifecycle: RevisionLifecycle,
}

/// The current heading and immutable list for one owner's Post.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostRevisionHistory {
    pub current: CurrentPostHistory,
    pub revisions: RevisionHistoryPage,
}

/// Lists every immutable revision owned by the authenticated user.
#[macros::server(input = Json)]
pub async fn list_history(
    cursor: Option<RevisionHistoryCursor>,
    limit: Option<PageSize>,
) -> WebResult<RevisionHistoryPage> {
    let auth = history_auth().await?;
    let page = expect_context::<Arc<dyn PostStorage>>()
        .list_owned_revision_history(
            auth.user_id,
            cursor.as_ref().map(storage_revision_cursor),
            limit.unwrap_or_default(),
        )
        .await?;
    let has_more = page.next_cursor.is_some();
    Ok(RevisionHistoryPage {
        revisions: page.revisions.into_iter().map(revision_metadata).collect(),
        next_cursor: page.next_cursor.map(revision_history_cursor),
        has_more,
    })
}

/// Returns one owned Post's current state and its immutable history, including
/// a Deleted Post. Foreign and absent Posts intentionally have the same error.
#[macros::server(input = Json)]
pub async fn get_post_history(
    post_id: PostId,
    cursor: Option<RevisionHistoryCursor>,
    limit: Option<PageSize>,
) -> WebResult<PostRevisionHistory> {
    let auth = history_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let now = UtcInstant::now();
    let current = posts
        .get_current_revision_summary(auth.user_id, post_id, now)
        .await?
        .ok_or_else(server::not_found_error)?;
    let page = posts
        .list_post_revision_history(
            auth.user_id,
            post_id,
            cursor.as_ref().map(storage_revision_cursor),
            limit.unwrap_or_default(),
        )
        .await?
        .ok_or_else(server::not_found_error)?;
    let has_more = page.next_cursor.is_some();
    Ok(PostRevisionHistory {
        current: current_post_history(current),
        revisions: RevisionHistoryPage {
            revisions: page.revisions.into_iter().map(revision_metadata).collect(),
            next_cursor: page.next_cursor.map(revision_history_cursor),
            has_more,
        },
    })
}

/// Returns one full immutable snapshot only when the authenticated user owns
/// both its Post and the requested revision.
#[macros::server(input = Json)]
pub async fn get_revision_history_detail(
    post_id: PostId,
    revision_id: RevisionId,
) -> WebResult<RevisionHistoryDetail> {
    let auth = history_auth().await?;
    let detail = expect_context::<Arc<dyn PostStorage>>()
        .get_post_revision_detail(auth.user_id, post_id, revision_id)
        .await?
        .ok_or_else(server::not_found_error)?;
    Ok(revision_detail(detail))
}

/// The author-supplied content of a post — the shared RPC input contract for
/// both [`create`] and [`update`], which differ only in whether a `post_id`
/// names an existing post. Bundling nests the JSON wire under the parameter
/// name, `post` (#299).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostInputs {
    pub body: PostBody,
    pub format: PostFormat,
    pub slug_override: Option<Slug>,
    pub publish: Option<bool>,
    pub publish_at: Option<UtcInstant>,
    pub tags: Option<Vec<TagLabel>>,
    pub summary: Option<PostSummary>,
    pub audience: Option<AudienceSelection>,
}

impl PostInputs {
    /// Constructs post inputs with only the required author-supplied fields.
    ///
    /// Every optional control remains omitted so endpoint lifecycle policy can
    /// distinguish absent input from an explicit value.
    #[must_use]
    pub fn new(body: PostBody, format: PostFormat) -> Self {
        Self {
            body,
            format,
            slug_override: None,
            publish: None,
            publish_at: None,
            tags: None,
            summary: None,
            audience: None,
        }
    }
}

/// Creates a post for the authenticated user.
///
/// `publish_at` is an optional UTC instant supplied by the compose form's
/// datetime control, carried as a [`UtcInstant`] (serde-transparent over an
/// RFC 3339 wire string; expressible in the `#[server]` signature on both the
/// server and the wasm client). The browser converts the author's local
/// `datetime-local` value to UTC before sending.
#[macros::server(input = Json, skip_all)]
pub async fn create(post: PostInputs) -> WebResult<MutationOutcome<SavedPost>> {
    let request_clock = UtcInstant::now();
    let PostInputs {
        body,
        format,
        slug_override,
        publish,
        publish_at,
        tags,
        summary,
        audience,
    } = post;
    let auth = auth::require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();

    // The wire delivers `Vec<TagLabel>` directly: each tag is validated at
    // arg-decode (ADR-0065) and a `TagLabel` is never empty, so the body only
    // dedups and enforces the per-post cap.
    let structured_tags = tags.map(tag::parse_and_validate_tags).transpose()?;
    let structured_audiences = audience
        .as_ref()
        .map(|selection| visibility::audience_targets_or_public(Some(selection)));

    let (body, title, summary, audiences, published_at, expectations, validated_tags) = if format
        == PostFormat::Org
    {
        let normalized = normalize_web_org(
            &body,
            OrgStructuredMetadata {
                title: Presence::Absent,
                summary: summary.map_or(Presence::Absent, Presence::Present),
                tags: structured_tags.map_or(Presence::Absent, Presence::Present),
                audiences: structured_audiences.map_or(Presence::Absent, Presence::Present),
                lifecycle: structured_lifecycle(publish, publish_at, request_clock),
            },
            OrgOperation::Create,
            request_clock,
        )?;
        let metadata = normalized.metadata;
        let audiences = match metadata.audiences {
            Presence::Present(audiences) => audiences,
            Presence::Absent => visibility::audience_targets_or_public(None),
        };
        validate_org_audiences(&audiences, auth.user_id).await?;
        let published_at = match metadata.lifecycle {
            Presence::Present(state) => state.published_at(),
            Presence::Absent => None,
        };
        let tags = match metadata.tags {
            Presence::Present(tags) => tags,
            Presence::Absent => Vec::new(),
        };
        (
            normalized.body,
            match metadata.title {
                Presence::Present(title) => Some(title),
                Presence::Absent => None,
            },
            match metadata.summary {
                Presence::Present(summary) => Some(summary),
                Presence::Absent => None,
            },
            audiences,
            published_at,
            normalized.bookkeeping.into(),
            tags,
        )
    } else {
        // Non-Org writes require explicit lifecycle control; unlike Org,
        // they have no header metadata from which to derive it.
        let publish = publish
            .ok_or_else(|| InternalError::validation("missing required structured lifecycle"))?;
        let published_at = if publish {
            Some(publish_at.unwrap_or(request_clock))
        } else {
            None
        };
        (
            body,
            None,
            summary,
            structured_audiences.unwrap_or_else(|| visibility::audience_targets_or_public(None)),
            published_at,
            PostBookkeepingExpectation::default(),
            structured_tags.unwrap_or_default(),
        )
    };

    let write_scope = expect_context::<WriteScope>();
    let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
    let content_locks = expect_context::<Arc<MediaContentLocks>>();
    let outcome = storage::perform_post_creation(
        &write_scope,
        content_locks.as_ref(),
        Arc::clone(&posts),
        Arc::clone(&feed_events),
        PostCreation {
            user_id: auth.user_id,
            body,
            title: title.as_ref(),
            format,
            slug_override: slug_override.as_ref(),
            published_at,
            max_attempts: 100,
            summary,
            audiences,
            idempotency_key: None,
            tags: validated_tags,
            expectations,
        },
    )
    .await?;

    let outcome = outcome.map(|record| SavedPost {
        post_id: record.post_id,
        slug: record.slug.clone(),
        published_at: record.published_at,
        permalink: record.permalink(),
    });
    if matches!(&outcome, MutationOutcome::Confirmed(_)) {
        metrics::post(metrics::PostEvent::Created);
    }
    Ok(outcome)
}

/// Retrieves a post by its permalink.
#[macros::server]
pub async fn get(username: Username, date: PermalinkDate, slug: Slug) -> WebResult<AuthoredPost> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let now = UtcInstant::now();

    let viewer = viewer::viewer_identity().await?;
    if let Some(post) =
        storage::fetch_post_record(posts.as_ref(), &viewer, &username, date, &slug, now).await?
    {
        let is_author = auth::require_auth()
            .await
            .is_ok_and(|auth| auth.user_id == post.user_id);
        return Ok(server::authored_post(post, is_author));
    }

    // The visibility-filtered lookup above found nothing public at this
    // permalink. The only remaining legitimate resolution is the author
    // viewing their own unpublished post, so require auth and confirm the
    // requester owns the namespace; everyone else gets an indistinguishable
    // 404 (never a 403 that would leak the post's existence).
    let auth = auth::require_auth()
        .await
        .map_err(|e| server::private_post_not_found_error(&e))?;
    if auth.username != username {
        return Err(server::not_found_error());
    }

    let post = posts
        .get_unpublished_post_by_permalink(auth.user_id, date, &slug, now)
        .await?
        .ok_or_else(server::not_found_error)?;

    Ok(server::authored_post(post, true))
}

/// Retrieves a Post and a same-response time snapshot for its authenticated
/// author to edit.
#[macros::server]
pub async fn get_preview(post_id: PostId) -> WebResult<EditPostPreview> {
    let auth = auth::require_auth()
        .await
        .map_err(|e| server::private_post_not_found_error(&e))?;
    let posts = expect_context::<Arc<dyn PostStorage>>();

    let post = posts
        .get_post_by_id(post_id, &viewer::viewer_identity().await?)
        .await?
        .ok_or_else(server::not_found_error)?;

    if post.deleted_at.is_some() || post.user_id != auth.user_id {
        return Err(server::not_found_error());
    }

    let fetched_at = UtcInstant::now();
    Ok(EditPostPreview {
        post: server::authored_post(post, true),
        fetched_at,
    })
}

/// Updates an existing post for the authenticated author.
///
/// `publish_at` is an optional UTC instant from the editor's datetime control.
/// See `create` for why it crosses the boundary as a [`UtcInstant`].
#[macros::server(input = Json, skip_all)]
pub async fn update(post_id: PostId, post: PostInputs) -> WebResult<MutationOutcome<SavedPost>> {
    let request_clock = UtcInstant::now();
    let PostInputs {
        body,
        format,
        slug_override,
        publish,
        publish_at,
        tags,
        summary,
        audience,
    } = post;
    let auth = auth::require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();

    let old = posts
        .get_post_by_id(post_id, &viewer::viewer_identity().await?)
        .await?;
    let old_tags: Vec<TagLabel> = old
        .as_ref()
        .map(|post| {
            post.tags
                .iter()
                .map(|tag| tag.tag_display.clone())
                .collect()
        })
        .unwrap_or_default();
    let old_tag_slugs: Vec<Tag> = old
        .as_ref()
        .map(|post| post.tags.iter().map(|tag| tag.tag_slug.clone()).collect())
        .unwrap_or_default();

    // Validate tags up-front so a malformed input rejects before any post
    // mutation lands. `None` preserves the current update surface behavior.
    let structured_tags = tags.map(tag::parse_and_validate_tags).transpose()?;
    let structured_audiences = audience
        .as_ref()
        .map(|selection| visibility::audience_targets_or_public(Some(selection)));

    let (body, title, summary, audiences, publish, expectations, new_tags) = if format
        == PostFormat::Org
    {
        let normalized = normalize_web_org(
            &body,
            OrgStructuredMetadata {
                title: Presence::Absent,
                summary: summary.map_or(Presence::Absent, Presence::Present),
                tags: structured_tags.map_or(Presence::Absent, Presence::Present),
                audiences: structured_audiences.map_or(Presence::Absent, Presence::Present),
                lifecycle: structured_lifecycle(publish, publish_at, request_clock),
            },
            OrgOperation::Update { post_id },
            request_clock,
        )?;
        let metadata = normalized.metadata;
        let audiences = match metadata.audiences {
            Presence::Present(audiences) => audiences,
            Presence::Absent => visibility::audience_targets_or_public(None),
        };
        validate_org_audiences(&audiences, auth.user_id).await?;
        let publish = match metadata.lifecycle {
            Presence::Present(state) => state.into(),
            Presence::Absent => PublishUpdate::Unpublish,
        };
        (
            normalized.body,
            match metadata.title {
                Presence::Present(title) => Some(title),
                Presence::Absent => None,
            },
            match metadata.summary {
                Presence::Present(summary) => Some(summary),
                Presence::Absent => None,
            },
            audiences,
            publish,
            normalized.bookkeeping.into(),
            match metadata.tags {
                Presence::Present(tags) => Some(tags),
                Presence::Absent => None,
            },
        )
    } else {
        let publish = publish
            .ok_or_else(|| InternalError::validation("missing required structured lifecycle"))?;
        (
            body,
            None,
            summary,
            structured_audiences.unwrap_or_else(|| visibility::audience_targets_or_public(None)),
            if publish {
                PublishUpdate::Publish { at: publish_at }
            } else {
                PublishUpdate::Unpublish
            },
            PostBookkeepingExpectation::default(),
            structured_tags,
        )
    };
    let new_tags = new_tags.unwrap_or(old_tags);

    let write_scope = expect_context::<WriteScope>();
    let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
    let content_locks = expect_context::<Arc<MediaContentLocks>>();
    let outcome = storage::perform_post_update(
        &write_scope,
        content_locks.as_ref(),
        Arc::clone(&posts),
        Arc::clone(&feed_events),
        PostUpdate {
            post_id,
            editor_user_id: auth.user_id,
            body,
            title: title.as_ref(),
            format,
            slug_override: slug_override.as_ref(),
            publish,
            summary,
            audiences,
            tags: new_tags,
            previous_tag_slugs: old_tag_slugs,
            request_clock,
            expectations,
        },
    )
    .await
    .map_err(|error| match error {
        PerformUpdateError::StaleContent => InternalError::conflict(error.to_string()),
        error => error.into(),
    })?;
    let outcome = outcome.map(|record| SavedPost {
        post_id,
        slug: record.slug.clone(),
        published_at: record.published_at,
        permalink: record.permalink(),
    });
    if matches!(&outcome, MutationOutcome::Confirmed(_)) {
        metrics::post(metrics::PostEvent::Updated);
    }
    Ok(outcome)
}

/// Returns the audience-picker selection for a new post: the site-wide
/// default audience. Used to initialize the editor on the create page.
#[macros::server]
pub async fn get_default_audience_selection() -> WebResult<AudienceSelection> {
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    auth::require_auth().await?;
    let default: AudienceTarget = site_config.get_default_audience().await?.into();
    Ok(visibility::targets_to_audience_selection(
        std::slice::from_ref(&default),
    ))
}

/// Returns the audience-picker selection for an existing post (its current
/// targeting). Owner-only. Used to pre-select the editor on the edit page.
#[macros::server]
pub async fn get_audience_selection(post_id: PostId) -> WebResult<AudienceSelection> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let auth = auth::require_auth()
        .await
        .map_err(|e| server::private_post_not_found_error(&e))?;

    let post = posts
        .get_post_by_id(post_id, &viewer::viewer_identity().await?)
        .await?
        .ok_or_else(server::not_found_error)?;
    if post.deleted_at.is_some() || post.user_id != auth.user_id {
        return Err(server::not_found_error());
    }

    let targets = posts.get_post_audiences(post_id).await?;
    Ok(visibility::targets_to_audience_selection(&targets))
}

/// Lists the authenticated user's unpublished posts (drafts and scheduled).
// The JSON input codec, unlike the crate's flat-scalar endpoints: a nested
// `PageCursor` cannot travel through the default form-urlencoded one. Same rule
// that already puts `create` and `update` on JSON — the other server fns taking
// a struct.
#[macros::server(input = Json)]
pub async fn list_drafts(
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<Page<UnpublishedPost>> {
    let auth = auth::require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();

    let parsed_cursor = storage::keyset_cursor(cursor);
    let page_size = limit.unwrap_or_default();
    let mut rows = posts
        .list_drafts_by_user(
            auth.user_id,
            parsed_cursor.as_ref(),
            page_size.fetch_limit(),
            UtcInstant::now(),
        )
        .await?;

    // The same derivation `crate::timeline`'s `page_from_rows` performs, spelled
    // here only because that helper derives a rendered-post page. Both halves of
    // the has-more rule still come off `PageSize` rather than hand-rolled arithmetic,
    // so the two sites cannot drift apart (#696).
    let has_more = page_size.has_more(rows.len());
    rows.truncate(page_size.page_len());
    let next_cursor = has_more
        .then(|| rows.last().map(storage::to_post_cursor))
        .flatten()
        .map(|c| storage::wire_cursor(&c));

    let unpublished = rows.into_iter().map(unpublished_post_from_record).collect();

    Ok(Page {
        posts: unpublished,
        next_cursor,
        has_more,
    })
}

/// Lists the authenticated user's scheduled posts only.
///
/// The wire cursor reuses [`PageCursor`]'s timestamp field for the scheduled
/// ordering key: on this endpoint it carries `published_at`, and rows are ordered
/// `published_at ASC, post_id ASC`.
#[macros::server(input = Json)]
pub async fn list_scheduled(
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<Page<UnpublishedPost>> {
    let auth = auth::require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();

    let parsed_cursor = storage::scheduled_keyset_cursor(cursor);
    let page_size = limit.unwrap_or_default();
    let mut rows = posts
        .list_scheduled_by_user(
            auth.user_id,
            parsed_cursor.as_ref(),
            page_size.fetch_limit(),
            UtcInstant::now(),
        )
        .await?;

    let has_more = page_size.has_more(rows.len());
    rows.truncate(page_size.page_len());
    let next_cursor = if has_more {
        rows.last()
            .map(storage::to_scheduled_post_cursor)
            .transpose()?
            .map(|c| storage::wire_scheduled_cursor(&c))
    } else {
        None
    };

    let scheduled = rows.into_iter().map(unpublished_post_from_record).collect();

    Ok(Page {
        posts: scheduled,
        next_cursor,
        has_more,
    })
}

/// Publishes an existing draft owned by the authenticated user.
#[macros::server]
pub async fn publish(post_id: PostId) -> WebResult<MutationOutcome<SavedPost>> {
    let auth = auth::require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                let updated = posts
                    .publish_post(transaction, post_id, auth.user_id)
                    .await
                    .map_err(|error| InternalError::from(PerformUpdateError::from(error)))?;
                if updated.published_at.is_none() {
                    return Err(InternalError::not_found("Post"));
                }
                let tag_slugs: BTreeSet<Tag> = updated
                    .tags
                    .iter()
                    .map(|tag| tag.tag_slug.clone())
                    .collect();
                feed_events
                    .enqueue_many(
                        transaction,
                        &feed::affected_feed_urls(&updated.author_username, &tag_slugs),
                    )
                    .await
                    .map_err(InternalError::storage)?;
                Ok(updated)
            })
        })
        .await
        .map_err(from_write_scope_error)?;
    let outcome = outcome.map(|updated| SavedPost {
        post_id: updated.post_id,
        slug: updated.slug.clone(),
        published_at: updated.published_at,
        permalink: updated.permalink(),
    });
    if matches!(&outcome, MutationOutcome::Confirmed(_)) {
        metrics::post(metrics::PostEvent::Published);
    }
    Ok(outcome)
}

/// Soft-deletes a post owned by the authenticated user.
#[macros::server]
pub async fn delete(post_id: PostId) -> WebResult<MutationOutcome<()>> {
    let auth = auth::require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let existing = posts
        .get_post_by_id(post_id, &viewer::viewer_identity().await?)
        .await?
        .ok_or_else(|| InternalError::not_found("Post"))?;
    if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
        return Err(InternalError::not_found("Post"));
    }
    let feed_paths = existing.published_at.map(|_| {
        let tag_slugs: BTreeSet<Tag> = existing
            .tags
            .iter()
            .map(|tag| tag.tag_slug.clone())
            .collect();
        feed::affected_feed_urls(&existing.author_username, &tag_slugs)
    });
    let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                posts
                    .soft_delete_post(transaction, post_id, auth.user_id)
                    .await
                    .map_err(|error| InternalError::from(PerformUpdateError::from(error)))?;
                if let Some(feed_paths) = feed_paths {
                    feed_events
                        .enqueue_many(transaction, &feed_paths)
                        .await
                        .map_err(InternalError::storage)?;
                }
                Ok(())
            })
        })
        .await
        .map_err(from_write_scope_error)?;
    if matches!(outcome, MutationOutcome::Confirmed(())) {
        metrics::post(metrics::PostEvent::Deleted);
    }
    Ok(outcome)
}

/// Reverts a published post owned by the authenticated user back to draft status.
#[macros::server]
pub async fn unpublish(post_id: PostId) -> WebResult<MutationOutcome<SavedPost>> {
    let auth = auth::require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
    let write_scope = expect_context::<WriteScope>();
    let outcome = write_scope
        .run(move |transaction| {
            Box::pin(async move {
                let updated = posts
                    .unpublish_post(transaction, post_id, auth.user_id)
                    .await
                    .map_err(|error| InternalError::from(PerformUpdateError::from(error)))?;
                let tag_slugs: BTreeSet<Tag> = updated
                    .tags
                    .iter()
                    .map(|tag| tag.tag_slug.clone())
                    .collect();
                feed_events
                    .enqueue_many(
                        transaction,
                        &feed::affected_feed_urls(&updated.author_username, &tag_slugs),
                    )
                    .await
                    .map_err(InternalError::storage)?;
                Ok(updated)
            })
        })
        .await
        .map_err(from_write_scope_error)?;
    Ok(outcome.map(|updated| SavedPost {
        post_id: updated.post_id,
        slug: updated.slug.clone(),
        published_at: updated.published_at,
        permalink: updated.permalink(),
    }))
}

#[cfg(test)]
mod tests {
    use common::slug::Slug;
    use common::test_support::{parse_post_body, parse_username};
    use storage::candidate_slug;
    // A wire DTO's `rendered_html` survives a serde round-trip: `Serialize` writes
    // the raw string, and the `deserialize_with` trusted-rebuild reconstructs a
    // `RenderedHtml` (the type has no blanket `Deserialize`). Covers the sole wire
    // reconstruction door.
    #[test]
    fn rendered_post_round_trips_rendered_html_via_server_rebuild() {
        use common::ids::PostId;
        use common::seed::RenderedPost;
        use common::test_support::{parse_root_relative_url, parse_utc_instant};
        let original = RenderedPost {
            post_id: PostId::from(1),
            username: parse_username("alice"),
            title: Some(common::test_support::parse_post_title("T")),
            summary: None,
            slug: "hello".parse::<Slug>().unwrap(),
            rendered_html: common::test_support::rendered_html("<p>hi</p>"),
            created_at: parse_utc_instant("2026-01-01T00:00:00Z"),
            published_at: Some(parse_utc_instant("2026-01-01T00:00:00Z")),
            permalink: Some(parse_root_relative_url("/~alice/2026/01/01/hello")),
            is_author: false,
            tags: vec![],
        };
        let json = serde_json::to_string(&original).unwrap();
        let round_tripped: RenderedPost = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped.rendered_html.as_ref(), "<p>hi</p>");
        assert!(
            !json.contains("\"is_draft\""),
            "RenderedPost must not serialize redundant draft state: {json}"
        );
        assert!(!round_tripped.is_draft());
        assert_eq!(round_tripped, original);
    }

    // The typed `RootRelativeUrl` permalink field pins the wire grammar: a
    // root-relative value round-trips, and an absolute URL is rejected at
    // JSON decode by the newtype's validating serde bridge (no in-body parse).
    #[test]
    fn saved_post_permalink_wire_is_root_relative() {
        use super::SavedPost;
        use common::ids::PostId;
        use common::test_support::{parse_root_relative_url, parse_utc_instant};

        let original = SavedPost {
            post_id: PostId::from(1),
            slug: "hello".parse::<Slug>().unwrap(),
            published_at: Some(parse_utc_instant("2026-01-01T00:00:00Z")),
            permalink: parse_root_relative_url("/~alice/2026/01/01/hello"),
        };
        let json = serde_json::to_string(&original).unwrap();
        // A root-relative permalink round-trips over the wire.
        assert_eq!(serde_json::from_str::<SavedPost>(&json).unwrap(), original);
        // Swapping the field to an absolute URL is rejected at decode.
        let absolute = json.replace("/~alice/2026/01/01/hello", "https://evil.example/x");
        assert!(serde_json::from_str::<SavedPost>(&absolute).is_err());
    }

    // The drafts wire nests the identity/publication quartet under `post` rather than
    // flattening it, so a client reads the same `SavedPost` shape here as it does from
    // create/update/publish. Round-trip the page to pin that nesting, and the
    // cursor/has-more envelope that lets the surface turn a page at all.
    #[test]
    fn unpublished_page_wire_nests_the_saved_post() {
        use super::{SavedPost, UnpublishedPost};
        use common::ids::PostId;
        use common::seed::{Page, PageCursor};
        use common::test_support::{
            parse_post_summary, parse_root_relative_url, parse_utc_instant,
        };

        let page = Page {
            posts: vec![UnpublishedPost {
                post: SavedPost {
                    post_id: PostId::from(1),
                    slug: "hello".parse::<Slug>().unwrap(),
                    published_at: Some(parse_utc_instant("2099-01-01T00:00:00Z")),
                    permalink: parse_root_relative_url("/~alice/2099/01/01/hello"),
                },
                title: None,
                summary_label: parse_post_summary("fallback label"),
                edit_url: parse_root_relative_url("/posts/1/edit"),
            }],
            next_cursor: Some(PageCursor {
                created_at: parse_utc_instant("2026-01-01T00:00:00Z"),
                post_id: PostId::from(1),
            }),
            has_more: true,
        };
        let json = serde_json::to_string(&page).unwrap();
        assert_eq!(
            json,
            r#"{"posts":[{"post":{"post_id":1,"slug":"hello","published_at":"2099-01-01T00:00:00Z","permalink":"/~alice/2099/01/01/hello"},"title":null,"summary_label":"fallback label","edit_url":"/posts/1/edit"}],"next_cursor":{"created_at":"2026-01-01T00:00:00Z","post_id":1},"has_more":true}"#
        );
        assert_eq!(
            serde_json::from_str::<Page<UnpublishedPost>>(&json).unwrap(),
            page
        );
    }

    #[test]
    fn candidate_slug_returns_seed_for_first_attempt() {
        let base: Slug = "hello-world".parse().unwrap();
        assert_eq!(candidate_slug(&base, 0).unwrap().as_ref(), "hello-world");
    }

    #[test]
    fn candidate_slug_appends_numeric_suffix_after_conflict() {
        let base: Slug = "hello-world".parse().unwrap();
        assert_eq!(candidate_slug(&base, 1).unwrap().as_ref(), "hello-world-2");
        assert_eq!(candidate_slug(&base, 2).unwrap().as_ref(), "hello-world-3");
    }

    #[test]
    fn post_inputs_new_preserves_required_fields_and_omits_optional_fields() {
        use super::PostInputs;
        use common::render::PostFormat;

        let body = parse_post_body("valid body");
        let post = PostInputs::new(body.clone(), PostFormat::Markdown);

        assert_eq!(post.body, body);
        assert_eq!(post.format, PostFormat::Markdown);
        assert_eq!(post.slug_override, None);
        assert_eq!(post.publish, None);
        assert_eq!(post.publish_at, None);
        assert_eq!(post.tags, None);
        assert_eq!(post.summary, None);
        assert_eq!(post.audience, None);
    }

    // #498: the create/update RPC input contracts carry `format` as a typed
    // `PostFormat`, so an out-of-domain token is rejected at JSON wire-decode (the
    // `input = Json` codec) — no in-body parse. Build a valid value, serialize, then
    // corrupt only the format token so the test never hardcodes the full wire shape.
    #[test]
    fn post_inputs_rejects_unknown_format_token() {
        use super::PostInputs;
        use common::render::PostFormat;
        let post = PostInputs {
            body: parse_post_body("hi"),
            format: PostFormat::Markdown,
            slug_override: None,
            publish: Some(false),
            publish_at: None,
            tags: None,
            summary: None,
            audience: None,
        };
        let json = serde_json::to_string(&post).unwrap();
        assert!(serde_json::from_str::<PostInputs>(&json).is_ok());
        let bad = json.replace("\"markdown\"", "\"bogus\"");
        assert!(serde_json::from_str::<PostInputs>(&bad).is_err());
    }

    #[cfg(feature = "server")]
    #[test]
    fn rendered_post_keeps_titleless_posts_titleless() {
        use crate::posts::server::rendered_post;
        use chrono::{TimeZone, Utc};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
        };
        use storage::{PostFormat, PostRecord};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let slug = "titleless-note".parse::<Slug>().unwrap();

        let summary = rendered_post(
            PostRecord {
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username: parse_username("author"),
                title: None,
                slug,
                body: parse_post_body("Titleless note"),
                format: PostFormat::Markdown,
                rendered_html: common::test_support::rendered_html("<p>Titleless note</p>"),
                created_at: common::time::UtcInstant::from(base_time),
                updated_at: common::time::UtcInstant::from(base_time),
                published_at: Some(common::time::UtcInstant::from(base_time)),
                deleted_at: None,
                summary: None,
                tags: vec![],
            },
            None,
        )
        .expect("published post should summarize");

        assert_eq!(summary.title, None);
        assert_eq!(summary.username, "author");
        assert_eq!(
            summary.permalink.as_deref(),
            Some("/~author/2026/04/16/titleless-note")
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn authored_post_marks_draft_state_from_published_at() {
        use crate::posts::server::authored_post;
        use chrono::{TimeZone, Utc};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
        };
        use storage::{PostFormat, PostRecord};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let author_username = parse_username("author");
        let slug = "hello-world".parse::<Slug>().unwrap();

        let draft = authored_post(
            PostRecord {
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username: author_username.clone(),
                title: Some(common::test_support::parse_post_title("Draft")),
                slug: slug.clone(),
                body: parse_post_body("body"),
                format: PostFormat::Markdown,
                rendered_html: common::test_support::rendered_html("<p>body</p>"),
                created_at: common::time::UtcInstant::from(base_time),
                updated_at: common::time::UtcInstant::from(base_time),
                published_at: None,
                deleted_at: None,
                summary: None,
                tags: vec![],
            },
            true,
        );
        assert!(draft.post.is_draft());
        assert!(draft.post.published_at.is_none());
        assert_eq!(draft.post.username, "author");

        let published = authored_post(
            PostRecord {
                post_id: PostId::from(2),
                user_id: UserId::from(2),
                author_username,
                title: Some(common::test_support::parse_post_title("Published")),
                slug,
                body: parse_post_body("body"),
                format: PostFormat::Markdown,
                rendered_html: common::test_support::rendered_html("<p>body</p>"),
                created_at: common::time::UtcInstant::from(base_time),
                updated_at: common::time::UtcInstant::from(base_time),
                published_at: Some(common::time::UtcInstant::from(base_time)),
                deleted_at: None,
                summary: None,
                tags: vec![],
            },
            false,
        );
        assert!(!published.post.is_draft());
        assert!(published.post.published_at.is_some());
    }
}

#[cfg(all(test, feature = "server"))]
mod server_tests {
    // Helper fns in this feature-gated test module aren't covered by clippy's
    // allow-{unwrap,expect}-in-tests, so expect the test-scaffolding panics.
    // lint-suppression:allow approved in #294; existing expectation documents intentional test-scaffolding or naming exception
    #![expect(clippy::unwrap_used)]
    use super::{
        PostInputs, create, get_post_history, get_revision_history_detail, list_drafts,
        list_history, publish, unpublish, update,
    };
    use crate::error::WebError;
    use crate::test_support::auth_parts;
    use common::pagination::PageSize;
    use common::revision_history::{RevisionHistoryAudience, RevisionHistoryTag};
    use common::slug::Slug;
    use common::tag::TagLabel;
    use common::test_support::{parse_post_body, parse_tag_label, parse_username};
    use common::time::UtcInstant;
    use common::{
        MutationOutcome,
        ids::{PostId, UserId},
    };
    use leptos::prelude::provide_context;
    use leptos::reactive::owner::Owner;
    use std::sync::Arc;
    use storage::{
        AudienceStorage, CreatedPost, CurrentPostRevisionSummary, FeedEventStorage,
        MediaContentLocks, MockAudienceStorage, MockFeedEventStorage, MockPostStorage, PostFormat,
        PostRecord, PostRevisionMetadata, PostRevisionPage, PostStorage, UpdatePostError,
        test_support::{fixture_media_content_locks, mock_write_scope},
    };
    fn owned_post(user_id: UserId) -> PostRecord {
        let now = UtcInstant::now();
        PostRecord {
            post_id: PostId::from(1),
            user_id,
            author_username: parse_username("alice"),
            title: Some(common::test_support::parse_post_title("t")),
            slug: "hello-world".parse::<Slug>().unwrap(),
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            rendered_html: common::test_support::rendered_html("<p>body</p>"),
            created_at: now,
            updated_at: now,
            published_at: None,
            deleted_at: None,
            summary: None,
            tags: vec![],
        }
    }

    /// Wires an authenticated owner (user 1) whose post store answers
    /// `publish_post` with `outcome`. Returns the owner, which the caller must keep
    /// alive across the `.await`.
    fn setup(outcome: fn() -> Result<PostRecord, UpdatePostError>) -> Owner {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "alice"));
        let mut posts = MockPostStorage::new();
        posts
            .expect_publish_post()
            .returning(move |_transaction, _id, _user| outcome());
        provide_context(Arc::new(posts) as Arc<dyn PostStorage>);
        provide_context(mock_write_scope());
        let mut events = MockFeedEventStorage::new();
        events
            .expect_enqueue_many()
            .returning(|_transaction, _paths| Ok(()));
        provide_context(Arc::new(events) as Arc<dyn FeedEventStorage>);
        owner
    }

    fn draft_row(post_id: i64) -> PostRecord {
        PostRecord {
            post_id: PostId::from(post_id),
            ..owned_post(UserId::from(1))
        }
    }

    #[test]
    fn revision_metadata_projector_preserves_every_field() {
        use common::test_support::parse_post_title;
        use storage::{PostLifecycle, PostRevisionMetadata};

        let at = common::test_support::parse_utc_instant("2026-08-27T12:34:56Z");
        let metadata = PostRevisionMetadata {
            revision_id: common::ids::RevisionId::from(8),
            post_id: PostId::from(7),
            title: Some(parse_post_title("Snapshot title")),
            slug: "snapshot".parse().expect("valid slug"),
            captured_at: at,
            snapshot_lifecycle: PostLifecycle::Published,
            current_deleted: true,
        };
        assert_eq!(
            super::revision_metadata(metadata),
            super::RevisionHistoryMetadata {
                revision_id: common::ids::RevisionId::from(8),
                post_id: PostId::from(7),
                title: Some(parse_post_title("Snapshot title")),
                slug: "snapshot".parse().expect("valid slug"),
                captured_at: at,
                snapshot_lifecycle: super::RevisionLifecycle::Published,
                current_deleted: true,
            }
        );
    }

    #[test]
    fn revision_lifecycle_and_current_projectors_preserve_every_field() {
        use storage::{CurrentPostRevisionSummary, PostLifecycle};

        let at = common::test_support::parse_utc_instant("2026-08-27T12:34:56Z");

        for (source, projected) in [
            (PostLifecycle::Draft, super::RevisionLifecycle::Draft),
            (
                PostLifecycle::Scheduled,
                super::RevisionLifecycle::Scheduled,
            ),
            (
                PostLifecycle::Published,
                super::RevisionLifecycle::Published,
            ),
            (PostLifecycle::Deleted, super::RevisionLifecycle::Deleted),
        ] {
            assert_eq!(super::revision_lifecycle(source), projected);
        }

        assert_eq!(
            super::current_post_history(CurrentPostRevisionSummary {
                post_id: PostId::from(7),
                title: None,
                slug: "current".parse().expect("valid slug"),
                format: PostFormat::Org,
                created_at: at,
                updated_at: at,
                published_at: Some(at),
                deleted_at: Some(at),
                lifecycle: PostLifecycle::Deleted,
            }),
            super::CurrentPostHistory {
                post_id: PostId::from(7),
                title: None,
                slug: "current".parse().expect("valid slug"),
                format: PostFormat::Org,
                created_at: at,
                updated_at: at,
                published_at: Some(at),
                deleted_at: Some(at),
                lifecycle: super::RevisionLifecycle::Deleted,
            }
        );
    }

    #[test]
    fn revision_detail_projector_preserves_every_field() {
        use common::media::parse_media_url;
        use common::test_support::{parse_post_summary, parse_post_title, parse_tag};
        use common::visibility::AudienceTarget;
        use storage::{PostRevisionDetail, PostRevisionRecord, PostRevisionTag};

        let at = common::test_support::parse_utc_instant("2026-08-27T12:34:56Z");

        let media = parse_media_url(
            "/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/photo.jpg",
        )
        .expect("valid media URL");
        let detail = super::revision_detail(PostRevisionDetail {
            revision: PostRevisionRecord {
                revision_id: common::ids::RevisionId::from(8),
                post_id: PostId::from(7),
                user_id: UserId::from(1),
                title: Some(parse_post_title("Snapshot title")),
                slug: "snapshot".parse().expect("valid slug"),
                body: parse_post_body("Snapshot body"),
                format: PostFormat::Markdown,
                rendered_html: common::test_support::rendered_html("<p>Snapshot body</p>"),
                summary: Some(parse_post_summary("Snapshot summary")),
                created_at: at,
                updated_at: at,
                published_at: Some(at),
                deleted_at: Some(at),
                captured_at: at,
                tags: vec![PostRevisionTag {
                    tag: parse_tag("rust"),
                    display: parse_tag_label("Rust"),
                }],
                audiences: vec![
                    AudienceTarget::Public,
                    AudienceTarget::Private,
                    AudienceTarget::Subscribers,
                    AudienceTarget::Named(common::ids::AudienceId::from(9)),
                ],
                media: vec![media],
            },
        });
        assert_eq!(detail.revision_id, common::ids::RevisionId::from(8));
        assert_eq!(detail.post_id, PostId::from(7));
        assert_eq!(detail.title, Some(parse_post_title("Snapshot title")));
        assert_eq!(detail.slug, "snapshot".parse::<Slug>().expect("valid slug"));
        assert_eq!(detail.body, parse_post_body("Snapshot body"));
        assert_eq!(detail.format, PostFormat::Markdown);
        assert_eq!(detail.rendered_html.as_ref(), "<p>Snapshot body</p>");
        assert_eq!(detail.summary, Some(parse_post_summary("Snapshot summary")));
        assert_eq!(
            (
                detail.created_at,
                detail.updated_at,
                detail.published_at,
                detail.deleted_at,
                detail.captured_at
            ),
            (at, at, Some(at), Some(at), at)
        );
        assert_eq!(
            detail.tags,
            vec![RevisionHistoryTag {
                tag: parse_tag("rust"),
                display: parse_tag_label("Rust")
            }]
        );
        assert_eq!(
            detail.audiences,
            vec![
                RevisionHistoryAudience {
                    kind: "public".to_owned(),
                    audience_id: None
                },
                RevisionHistoryAudience {
                    kind: "private".to_owned(),
                    audience_id: None
                },
                RevisionHistoryAudience {
                    kind: "subscribers".to_owned(),
                    audience_id: None
                },
                RevisionHistoryAudience {
                    kind: "named".to_owned(),
                    audience_id: Some(common::ids::AudienceId::from(9))
                }
            ]
        );
        assert_eq!(detail.media, vec!["/media/upload/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855/photo.jpg".to_owned()]);
    }

    #[test]
    fn revision_history_cursor_traces_its_immutable_id() {
        use common::trace_field::TraceField;

        let cursor = super::RevisionHistoryCursor {
            revision_id: common::ids::RevisionId::from(8),
        };
        assert_eq!(cursor.trace_value(), common::ids::RevisionId::from(8));
        let storage = super::storage_revision_cursor(&cursor);
        assert_eq!(storage.revision_id, common::ids::RevisionId::from(8));
        assert_eq!(super::revision_history_cursor(storage), cursor);
    }

    #[test]
    fn structured_lifecycle_preserves_transport_presence() {
        use common::org::{Presence, PublicationState};

        let clock: UtcInstant = "2026-08-26T12:00:00Z".parse().unwrap();
        assert!(matches!(
            super::structured_lifecycle(None, None, clock),
            Presence::Absent
        ));
        assert!(matches!(
            super::structured_lifecycle(Some(false), None, clock),
            Presence::Present(PublicationState::Draft)
        ));
        assert!(matches!(
            super::structured_lifecycle(Some(true), None, clock),
            Presence::Present(PublicationState::Published(at)) if at == clock
        ));
        let future = common::test_support::parse_utc_instant("2026-08-26T12:01:00Z");
        assert!(matches!(
            super::structured_lifecycle(Some(true), Some(future), clock),
            Presence::Present(PublicationState::Scheduled(at)) if at == future
        ));
    }

    /// The probing-row twin of `listing.rs`'s
    /// `every_paginated_fetcher_asks_storage_for_the_probing_row`, which cannot reach
    /// this path: `list_drafts` is a `#[server]` fn needing an owner and an
    /// authenticated context, not a plain fetcher.
    ///
    /// It shipped as `exact_limit()` — exactly the page, no probe — which pins
    /// `has_more` to `false` and `next_cursor` to `None` forever, so the drafts surface
    /// can never turn a page. That is the regression `fetch_posts_by_tag` already
    /// suffered once; asserting the limit here is what keeps it from recurring.
    // guard:no-backend — mock store
    #[tokio::test]
    async fn list_drafts_asks_storage_for_the_probing_row() {
        for (returned, expect_more) in [(5usize, false), (6usize, true)] {
            let page_size = PageSize::clamped(5);
            let owner = Owner::new();
            owner.set();
            provide_context(auth_parts(UserId::from(1), "alice"));
            let mut posts = MockPostStorage::new();
            posts
                .expect_list_drafts_by_user()
                .withf(move |_uid, _cursor, limit, _now| *limit == page_size.fetch_limit())
                .returning(move |_uid, _cursor, _limit, _now| {
                    // `try_from(...).unwrap_or` rather than an `as` cast: total, and the
                    // ids only have to be distinct.
                    Ok((0..returned)
                        .map(|i| draft_row(i64::try_from(i).unwrap_or(0) + 1))
                        .collect())
                });
            provide_context(Arc::new(posts) as Arc<dyn PostStorage>);

            let page = list_drafts(None, Some(page_size)).await;
            drop(owner);
            let page = page.expect("listing succeeds");

            assert_eq!(page.has_more, expect_more, "has_more for {returned} rows");
            assert_eq!(
                page.next_cursor.is_some(),
                expect_more,
                "a cursor exactly when another page exists"
            );
            // The probing row never reaches the caller.
            assert_eq!(page.posts.len(), returned.min(page_size.page_len()));
        }
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn publish_maps_not_found_publish_error_to_not_found() {
        let owner = setup(|| Err(UpdatePostError::NotFound));
        let result = publish(PostId::from(1)).await;
        drop(owner);
        assert!(matches!(result.unwrap_err(), WebError::NotFound { .. }));
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn publish_maps_internal_publish_error_to_storage() {
        let owner = setup(|| Err(UpdatePostError::Internal(sqlx::Error::PoolClosed)));
        let result = publish(PostId::from(1)).await;
        drop(owner);
        assert!(matches!(result.unwrap_err(), WebError::Storage { .. }));
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn publish_maps_a_still_unpublished_record_to_not_found() {
        // `publish_post` always stamps `published_at`; if a record comes back without
        // one the handler has nothing to report, so it must not claim success.
        let owner = setup(|| Ok(owned_post(UserId::from(1))));
        let result = publish(PostId::from(1)).await;
        drop(owner);
        assert!(matches!(result.unwrap_err(), WebError::NotFound { .. }));
    }

    /// Wires an authenticated owner (user 1) over a caller-configured post store
    /// plus a permissive feed-event store, so `create`/`update` run end-to-end
    /// without a database. The mock's expectations are verified when the owner —
    /// and with it the context-held `Arc` — is dropped.
    fn mutation_owner(posts: MockPostStorage) -> Owner {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "alice"));
        provide_context(Arc::new(posts) as Arc<dyn PostStorage>);
        provide_context(Arc::new(MockAudienceStorage::new()) as Arc<dyn AudienceStorage>);
        provide_context(mock_write_scope());
        provide_context(Arc::new(fixture_media_content_locks()) as Arc<MediaContentLocks>);
        let mut events = MockFeedEventStorage::new();
        events
            .expect_enqueue_many()
            .returning(|_transaction, _paths| Ok(()));
        provide_context(Arc::new(events) as Arc<dyn FeedEventStorage>);
        owner
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn unpublish_uses_the_guarded_returned_record_without_a_preread() {
        let returned = owned_post(UserId::from(1));
        let expected_permalink = returned.permalink();
        let mut posts = MockPostStorage::new();
        posts
            .expect_unpublish_post()
            .times(1)
            .withf(|_transaction, post_id, user_id| {
                *post_id == PostId::from(1) && *user_id == UserId::from(1)
            })
            .returning(move |_transaction, _post_id, _user_id| Ok(returned.clone()));
        let owner = mutation_owner(posts);

        let saved = unpublish(PostId::from(1)).await;
        drop(owner);
        assert!(matches!(
            saved,
            Ok(MutationOutcome::Confirmed(saved))
                if saved.post_id == PostId::from(1)
                    && saved.published_at.is_none()
                    && saved.permalink == expected_permalink
        ));
    }

    // guard:no-backend — mock store
    fn history_metadata(revision_id: i64) -> PostRevisionMetadata {
        PostRevisionMetadata {
            revision_id: common::ids::RevisionId::from(revision_id),
            post_id: PostId::from(7),
            title: None,
            slug: common::test_support::parse_slug("history"),
            captured_at: common::test_support::parse_utc_instant("2026-08-27T12:00:00Z"),
            snapshot_lifecycle: storage::PostLifecycle::Draft,
            current_deleted: false,
        }
    }

    fn history_owner(posts: MockPostStorage) -> Owner {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "alice"));
        provide_context(Arc::new(posts) as Arc<dyn PostStorage>);
        owner
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn list_history_projects_storage_rows_and_cursor() {
        let page_size = PageSize::clamped(5);
        let mut posts = MockPostStorage::new();
        posts
            .expect_list_owned_revision_history()
            .withf(move |user_id, cursor, limit| {
                *user_id == UserId::from(1) && cursor.is_none() && *limit == page_size
            })
            .returning(|_, _, _| {
                Ok(PostRevisionPage {
                    revisions: vec![history_metadata(8)],
                    next_cursor: Some(storage::PostRevisionCursor {
                        revision_id: common::ids::RevisionId::from(7),
                    }),
                })
            });
        let owner = history_owner(posts);
        let page = list_history(None, Some(page_size)).await;
        drop(owner);

        let page = page.expect("history list succeeds");
        assert_eq!(page.revisions.len(), 1);
        assert_eq!(
            page.revisions[0].revision_id,
            common::ids::RevisionId::from(8)
        );
        assert_eq!(
            page.next_cursor,
            Some(super::RevisionHistoryCursor {
                revision_id: common::ids::RevisionId::from(7),
            })
        );
        assert!(page.has_more);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn post_history_projects_current_state_and_revision_page() {
        let page_size = PageSize::clamped(5);
        let captured_at = common::test_support::parse_utc_instant("2026-08-27T12:00:00Z");
        let mut posts = MockPostStorage::new();
        posts
            .expect_get_current_revision_summary()
            .returning(move |user_id, post_id, _| {
                let summary = CurrentPostRevisionSummary {
                    post_id,
                    title: None,
                    slug: common::test_support::parse_slug("history"),
                    format: PostFormat::Markdown,
                    created_at: captured_at,
                    updated_at: captured_at,
                    published_at: None,
                    deleted_at: None,
                    lifecycle: storage::PostLifecycle::Draft,
                };
                Ok((user_id == UserId::from(1)).then_some(summary))
            });
        posts
            .expect_list_post_revision_history()
            .withf(move |user_id, post_id, cursor, limit| {
                *user_id == UserId::from(1)
                    && *post_id == PostId::from(7)
                    && cursor.is_none()
                    && *limit == page_size
            })
            .returning(|_, _, _, _| {
                Ok(Some(PostRevisionPage {
                    revisions: vec![history_metadata(8)],
                    next_cursor: Some(storage::PostRevisionCursor {
                        revision_id: common::ids::RevisionId::from(7),
                    }),
                }))
            });
        let owner = history_owner(posts);
        let history = get_post_history(PostId::from(7), None, Some(page_size)).await;
        drop(owner);

        let history = history.expect("post history succeeds");
        assert_eq!(history.current.post_id, PostId::from(7));
        assert_eq!(history.current.title, None);
        assert_eq!(history.current.lifecycle, super::RevisionLifecycle::Draft);
        assert_eq!(
            history.revisions.revisions[0].revision_id,
            common::ids::RevisionId::from(8)
        );
        assert_eq!(
            history.revisions.next_cursor,
            Some(super::RevisionHistoryCursor {
                revision_id: common::ids::RevisionId::from(7),
            })
        );
        assert!(history.revisions.has_more);
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn post_history_masks_missing_current_or_page_as_not_found() {
        let mut missing_current = MockPostStorage::new();
        missing_current
            .expect_get_current_revision_summary()
            .returning(|_, _, _| Ok(None));
        let owner = history_owner(missing_current);
        let current = get_post_history(PostId::from(7), None, None).await;
        drop(owner);
        assert!(matches!(current, Err(WebError::NotFound { .. })));

        let captured_at = common::test_support::parse_utc_instant("2026-08-27T12:00:00Z");
        let mut missing_page = MockPostStorage::new();
        missing_page
            .expect_get_current_revision_summary()
            .returning(move |_, post_id, _| {
                Ok(Some(CurrentPostRevisionSummary {
                    post_id,
                    title: None,
                    slug: "history".parse().expect("valid slug"),
                    format: PostFormat::Markdown,
                    created_at: captured_at,
                    updated_at: captured_at,
                    published_at: None,
                    deleted_at: None,
                    lifecycle: storage::PostLifecycle::Draft,
                }))
            });
        missing_page
            .expect_list_post_revision_history()
            .returning(|_, _, _, _| Ok(None));
        let owner = history_owner(missing_page);
        let page = get_post_history(PostId::from(7), None, None).await;
        drop(owner);
        assert!(matches!(page, Err(WebError::NotFound { .. })));
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn revision_detail_masks_missing_and_projects_storage_failures() {
        let mut missing = MockPostStorage::new();
        missing
            .expect_get_post_revision_detail()
            .returning(|_, _, _| Ok(None));
        let owner = history_owner(missing);
        let result =
            get_revision_history_detail(PostId::from(7), common::ids::RevisionId::from(8)).await;
        drop(owner);
        assert!(matches!(result, Err(WebError::NotFound { .. })));

        let mut failed = MockPostStorage::new();
        failed
            .expect_get_post_revision_detail()
            .returning(|_, _, _| Err(sqlx::Error::PoolClosed));
        let owner = history_owner(failed);
        let result =
            get_revision_history_detail(PostId::from(7), common::ids::RevisionId::from(8)).await;
        drop(owner);
        assert!(matches!(result, Err(WebError::Storage { .. })));
    }
    #[tokio::test]
    async fn unpublish_masks_storage_unauthorized_as_not_found() {
        let mut posts = MockPostStorage::new();
        posts
            .expect_unpublish_post()
            .returning(|_transaction, _post_id, _user_id| Err(UpdatePostError::Unauthorized));
        let owner = mutation_owner(posts);

        let result = unpublish(PostId::from(1)).await;
        drop(owner);
        assert!(matches!(result.unwrap_err(), WebError::NotFound { .. }));
    }

    /// The content half of a mutation call. `create` and `update` take the same
    /// `PostInputs`; `update` names the post it edits in a separate argument.
    fn post_inputs(tags: Option<Vec<TagLabel>>) -> PostInputs {
        PostInputs {
            body: parse_post_body("body"),
            format: PostFormat::Markdown,
            slug_override: None,
            publish: Some(false),
            publish_at: None,
            tags,
            summary: None,
            audience: None,
        }
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn create_writes_every_tag_in_one_batched_call() {
        // Tags travel in the creation input, so the storage transaction writes
        // the complete initial post state atomically.
        let mut posts = MockPostStorage::new();
        posts
            .expect_create_post()
            .withf(|_transaction, input, _now| input.tags.len() == 2)
            .returning(|_transaction, _input, _now| {
                Ok(CreatedPost {
                    record: owned_post(UserId::from(1)),
                    idempotency_key_expired: false,
                })
            });
        let owner = mutation_owner(posts);
        let result = create(post_inputs(Some(vec![
            parse_tag_label("rust"),
            parse_tag_label("web"),
        ])))
        .await;
        drop(owner);
        result.expect("create succeeds");
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn update_writes_every_tag_in_one_batched_call() {
        let mut posts = MockPostStorage::new();
        posts
            .expect_get_post_by_id()
            .returning(|_id, _viewer| Ok(Some(owned_post(UserId::from(1)))));
        posts
            .expect_update_post()
            .withf(|_transaction, _id, _user, input| input.tags.len() == 2)
            .returning(|_transaction, _id, _user, _input| Ok(owned_post(UserId::from(1))));
        let owner = mutation_owner(posts);
        let result = update(
            PostId::from(1),
            post_inputs(Some(vec![parse_tag_label("rust"), parse_tag_label("web")])),
        )
        .await;
        drop(owner);
        result.expect("update succeeds");
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn update_with_tags_unset_preserves_current_tags_atomically() {
        // `tags: None` preserves the current tags in the single update input.
        let mut posts = MockPostStorage::new();
        posts
            .expect_get_post_by_id()
            .returning(|_id, _viewer| Ok(Some(owned_post(UserId::from(1)))));
        posts
            .expect_update_post()
            .withf(|_transaction, _id, _user, input| input.tags.is_empty())
            .returning(|_transaction, _id, _user, _input| Ok(owned_post(UserId::from(1))));

        let owner = mutation_owner(posts);
        let result = update(PostId::from(1), post_inputs(None)).await;
        drop(owner);
        result.expect("update succeeds");
    }
    // guard:no-backend — mock store
    #[tokio::test]
    async fn update_org_keeps_structured_audience_and_summary() {
        use common::test_support::parse_post_summary;
        use common::visibility::{AudienceBase, AudienceSelection, AudienceTarget};

        let mut posts = MockPostStorage::new();
        posts
            .expect_get_post_by_id()
            .returning(|_id, _viewer| Ok(Some(owned_post(UserId::from(1)))));
        posts
            .expect_update_post()
            .withf(|_transaction, _id, _user, input| {
                input.summary.as_deref() == Some("structured summary")
                    && input.audiences == [AudienceTarget::Subscribers]
            })
            .returning(|_transaction, _id, _user, _input| Ok(owned_post(UserId::from(1))));
        let owner = mutation_owner(posts);
        let result = update(
            PostId::from(1),
            PostInputs {
                body: parse_post_body("Body"),
                format: PostFormat::Org,
                slug_override: None,
                publish: Some(false),
                publish_at: None,
                tags: None,
                summary: Some(parse_post_summary("structured summary")),
                audience: Some(AudienceSelection {
                    base: AudienceBase::Subscribers,
                    named: vec![],
                }),
            },
        )
        .await;
        drop(owner);
        result.expect("update succeeds");
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn update_projects_stale_org_sync_to_conflict() {
        let mut posts = MockPostStorage::new();
        posts
            .expect_get_post_by_id()
            .returning(|_id, _viewer| Ok(Some(owned_post(UserId::from(1)))));
        posts
            .expect_update_post()
            .returning(|_transaction, _id, _user, _input| Err(UpdatePostError::StaleContent));
        let owner = mutation_owner(posts);
        let result = update(
            PostId::from(1),
            PostInputs {
                body: parse_post_body(
                    "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_SYNCED \"sha256-stale\"\n\nbody",
                ),
                format: PostFormat::Org,
                slug_override: None,
                publish: Some(false),
                publish_at: None,
                tags: None,
                summary: None,
                audience: None,
            },
        )
        .await;
        drop(owner);
        assert!(matches!(result, Err(WebError::Conflict { .. })));
    }
}
