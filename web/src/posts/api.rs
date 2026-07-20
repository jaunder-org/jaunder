//! Posts wire types and `#[server]` endpoints (ADR-0070, amended #530).
//!
//! The single-post lifecycle DTOs and their `#[server]` fns live here; the
//! timeline/listing surface is in the [`listing`] submodule and re-exported.
//! `posts/mod.rs` is wiring only and re-exports these under the stable
//! `crate::posts::…` paths that external call sites and the server-fn registrar
//! depend on.

use leptos::prelude::*;
use leptos::server_fn::codec::Json;
use serde::{Deserialize, Serialize};

/// Timeline/listing endpoints, split out from the single-post lifecycle below.
/// Re-exported so `crate::posts::list_*` / `TimelinePage` keep resolving.
mod listing;
pub use listing::*;

use common::{
    ids::{AudienceId, PostId},
    pagination::PageSize,
    post_body::PostBody,
    post_title::PostTitle,
    render::RenderedHtml,
    slug::Slug,
    tag::TagLabel,
    time::UtcInstant,
    username::Username,
    visibility::AudienceBase,
};

use crate::error::WebResult;
use crate::tags::TagSummary;

// SSR-only imports for #[server] bodies
#[cfg(feature = "server")]
use {
    super::server::{not_found_error, post_response, private_post_not_found_error},
    crate::auth::require_auth,
    crate::error::InternalError,
    crate::feed_events::enqueue_feed_events,
    crate::viewer::viewer_identity,
    chrono::Utc,
    common::tag::Tag,
    std::{collections::BTreeSet, sync::Arc},
    storage::{
        apply_post_tag_diff, fetch_post_record, find_draft_by_permalink_for_user,
        parse_post_cursor, perform_post_creation, perform_post_update, FeedEventStorage,
        PostCreation, PostFormat, PostStorage, PostUpdate, PublishUpdate, SiteConfigStorage,
        UpdatePostInput,
    },
};

/// Result returned by [`create_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreatePostResult {
    pub post_id: PostId,
    pub slug: Slug,
    pub created_at: UtcInstant,
    pub published_at: Option<UtcInstant>,
    pub preview_url: String,
    pub permalink: Option<String>,
    pub summary: Option<String>,
}

/// Result returned by [`update_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdatePostResult {
    pub post_id: PostId,
    pub slug: Slug,
    pub published_at: Option<UtcInstant>,
    pub preview_url: String,
    pub permalink: Option<String>,
    pub summary: Option<String>,
}

/// The audience-picker selection as it crosses the server-fn boundary.
///
/// `base` is the mutually-exclusive built-in ([`AudienceBase::Public`],
/// [`AudienceBase::Private`], or [`AudienceBase::Subscribers`]); `named` is the
/// set of selected named-audience ids. The two compose by UNION except for
/// [`AudienceBase::Private`], which is author-only and cannot combine with
/// anything — a `Private` base discards `named`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AudienceSelection {
    pub base: AudienceBase,
    pub named: Vec<AudienceId>,
}

/// Translates an [`AudienceSelection`] into the `Vec<AudienceTarget>` the
/// storage layer persists.
///
/// - [`AudienceBase::Public`] / [`AudienceBase::Subscribers`] → the built-in
///   target, in union with one `Named(id)` per selected named audience.
/// - [`AudienceBase::Private`] → an empty vec (author-only); the named set is
///   ignored, since `Private` cannot combine with other targets.
#[must_use]
pub fn audience_selection_to_targets(
    selection: &AudienceSelection,
) -> Vec<common::visibility::AudienceTarget> {
    use common::visibility::AudienceTarget;
    let base = match selection.base {
        AudienceBase::Public => Some(AudienceTarget::Public),
        AudienceBase::Subscribers => Some(AudienceTarget::Subscribers),
        // Private is author-only: no built-in target, and named is dropped below.
        AudienceBase::Private => None,
    };
    let Some(base) = base else {
        // Private/author-only: no rows, named selection ignored.
        return Vec::new();
    };
    std::iter::once(base)
        .chain(selection.named.iter().copied().map(AudienceTarget::Named))
        .collect()
}

/// Resolves an optional picker selection to the targets to persist. An absent
/// selection defaults to `[Public]` — the historical behavior and the safe
/// default for non-editor callers that omit the field on the wire.
#[must_use]
pub fn audience_targets_or_public(
    selection: Option<&AudienceSelection>,
) -> Vec<common::visibility::AudienceTarget> {
    selection.map_or_else(
        || vec![common::visibility::AudienceTarget::Public],
        audience_selection_to_targets,
    )
}

/// Translates a post's persisted `Vec<AudienceTarget>` into the picker's
/// [`AudienceSelection`] (the inverse of [`audience_selection_to_targets`],
/// for pre-selecting the editor).
///
/// The built-in base is [`AudienceBase::Public`]/[`AudienceBase::Subscribers`]
/// when that target is present, otherwise [`AudienceBase::Private`] (covering
/// both an explicit `Private` and an empty targeting). Every `Named(id)` becomes
/// an entry in `named`.
#[must_use]
pub fn targets_to_audience_selection(
    targets: &[common::visibility::AudienceTarget],
) -> AudienceSelection {
    use common::visibility::AudienceTarget;
    let mut base = AudienceBase::Private;
    let mut named = Vec::new();
    for target in targets {
        match target {
            AudienceTarget::Public => base = AudienceBase::Public,
            AudienceTarget::Subscribers => base = AudienceBase::Subscribers,
            AudienceTarget::Named(id) => named.push(*id),
            AudienceTarget::Private => {}
        }
    }
    AudienceSelection { base, named }
}

/// A draft row returned by [`list_drafts`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DraftSummary {
    pub post_id: PostId,
    pub title: Option<PostTitle>,
    pub summary_label: String,
    pub slug: Slug,
    pub created_at: UtcInstant,
    pub updated_at: UtcInstant,
    /// UTC publication instant for a *scheduled* post (`published_at`
    /// in the future); `None` for true drafts. Drives the "Scheduled for …"
    /// author marker.
    pub scheduled_at: Option<UtcInstant>,
    pub preview_url: String,
    pub edit_url: String,
    pub permalink: String,
}

/// Result returned by [`publish_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishPostResult {
    pub post_id: PostId,
    pub slug: Slug,
    pub published_at: UtcInstant,
    pub permalink: String,
}

/// Trusted-rebuild `deserialize_with` for a wire `RenderedHtml` field: the value
/// is prior `render()` output serialized by our own server, so reconstruct it via
/// [`RenderedHtml::from_trusted`] (the type has no blanket `Deserialize` by design).
pub(crate) fn deserialize_rendered_html<'de, D>(deserializer: D) -> Result<RenderedHtml, D::Error>
where
    D: serde::Deserializer<'de>,
{
    String::deserialize(deserializer).map(RenderedHtml::from_trusted)
}

/// Details of a post returned by [`get_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostResponse {
    pub post_id: PostId,
    pub username: Username,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub body: PostBody,
    pub format: String,
    #[serde(deserialize_with = "deserialize_rendered_html")]
    pub rendered_html: RenderedHtml,
    pub created_at: UtcInstant,
    pub published_at: Option<UtcInstant>,
    pub is_draft: bool,
    pub is_author: bool,
    /// Permalink URL for published posts; `None` for drafts.
    pub permalink: Option<String>,
    /// Tags applied to this post, ordered by canonical slug.
    pub tags: Vec<TagSummary>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<String>,
}

/// Creates a post for the authenticated user.
///
/// `publish_at` is an optional UTC instant supplied by the compose form's
/// datetime control, carried as a [`UtcInstant`] (serde-transparent over an
/// RFC 3339 wire string; expressible in the `#[server]` signature on both the
/// server and the wasm client). The browser converts the author's local
/// `datetime-local` value to UTC before sending.
// `#[expect]` can't be used here: the `#[server]` macro emits too_many_arguments from
// its own expansion, so a fn-level expectation is always reported "unfulfilled". A plain
// `#[allow]` is the only mechanism that suppresses a macro-emitted lint. The args are the
// RPC input contract — bundling them into a struct would change the JSON wire shape. (#94)
#[allow(clippy::too_many_arguments)]
#[server(endpoint = "/create_post", input = Json)]
pub async fn create_post(
    body: PostBody,
    format: String,
    slug_override: Option<Slug>,
    publish: bool,
    publish_at: Option<UtcInstant>,
    tags: Option<Vec<TagLabel>>,
    summary: Option<String>,
    audience: Option<AudienceSelection>,
) -> WebResult<CreatePostResult> {
    boundary!("create_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        // The wire delivers `Vec<TagLabel>` directly: each tag is validated at
        // arg-decode (ADR-0065) and a `TagLabel` is never empty, so the body only
        // dedups and enforces the per-post cap.
        let validated_tags = common::tag::parse_and_validate_tags(tags.unwrap_or_default())?;

        let format = format.parse::<PostFormat>()?;
        // Publish + a supplied time = scheduled (future) or backdated (past);
        // publish + no time = live now; not publishing = draft (NULL).
        let published_at = if publish {
            Some(publish_at.map_or_else(Utc::now, UtcInstant::value))
        } else {
            None
        };
        let normalized_summary = summary.and_then(common::text::non_empty_owned);
        let audiences = audience_targets_or_public(audience.as_ref());

        let record = perform_post_creation(
            posts.as_ref(),
            PostCreation {
                user_id: auth.user_id,
                body,
                title: None,
                format,
                slug_override: slug_override.as_ref(),
                published_at,
                max_attempts: 100,
                summary: normalized_summary,
                audiences,
                idempotency_key: None,
            },
        )
        .await?;

        let created_at = UtcInstant::from(record.created_at);
        let published_at = record.published_at.map(UtcInstant::from);
        // Only published posts have a public permalink. For drafts, the permalink is None.
        let permalink = record.published_at.is_some().then(|| record.permalink());
        let preview_url = format!("/draft/{}/preview", record.post_id);

        let created = CreatePostResult {
            post_id: record.post_id,
            slug: record.slug,
            created_at,
            published_at,
            preview_url,
            permalink,
            summary: record.summary,
        };

        for label in &validated_tags {
            posts.tag_post(created.post_id, label).await?;
        }

        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        let tag_post_tags = posts.get_tags_for_post(created.post_id).await?;
        let tag_slugs: BTreeSet<Tag> = tag_post_tags.iter().map(|t| t.tag_slug.clone()).collect();
        enqueue_feed_events(feed_events.as_ref(), &auth.username, &tag_slugs)
            .await
            .map_err(InternalError::storage)?;

        host::metrics::post(host::metrics::PostEvent::Created);
        Ok(created)
    })
}

/// Retrieves a post by its permalink.
#[server(endpoint = "/get_post")]
pub async fn get_post(
    username: Username,
    year: i32,
    month: u32,
    day: u32,
    slug: Slug,
) -> WebResult<PostResponse> {
    boundary!("get_post", {
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let viewer = viewer_identity().await;
        if let Some(post) =
            fetch_post_record(posts.as_ref(), &viewer, &username, year, month, day, &slug).await?
        {
            let is_author = require_auth()
                .await
                .is_ok_and(|auth| auth.user_id == post.user_id);
            return Ok(post_response(post, is_author));
        }

        // The visibility-filtered lookup above found nothing public at this
        // permalink. The only remaining legitimate resolution is the author
        // viewing their own unpublished draft, so require auth and confirm the
        // requester owns the namespace; everyone else gets an indistinguishable
        // 404 (never a 403 that would leak the draft's existence).
        let auth = require_auth()
            .await
            .map_err(|e| private_post_not_found_error(&e))?;
        if auth.username != username {
            return Err(not_found_error());
        }

        let draft =
            find_draft_by_permalink_for_user(posts.as_ref(), auth.user_id, year, month, day, &slug)
                .await?
                .ok_or_else(not_found_error)?;

        Ok(post_response(draft, true))
    })
}

/// Retrieves a draft preview for the authenticated author.
#[server(endpoint = "/get_post_preview")]
pub async fn get_post_preview(post_id: PostId) -> WebResult<PostResponse> {
    boundary!("get_post_preview", {
        let auth = require_auth()
            .await
            .map_err(|e| private_post_not_found_error(&e))?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let post = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await?
            .ok_or_else(not_found_error)?;

        if post.deleted_at.is_some() || post.user_id != auth.user_id {
            return Err(not_found_error());
        }

        Ok(post_response(post, true))
    })
}

/// Updates an existing post for the authenticated author.
// See `create_post`: `#[expect]` is always "unfulfilled" against the `#[server]` macro's
// own emission, so a justified `#[allow]` is the only working suppression here. (#94)
#[allow(clippy::too_many_arguments)]
#[server(endpoint = "/update_post", input = Json)]
pub async fn update_post(
    post_id: PostId,
    body: PostBody,
    format: String,
    slug_override: Option<Slug>,
    publish: bool,
    // Optional UTC instant from the editor's datetime control. See
    // `create_post` for why this crosses the boundary as a `UtcInstant`.
    publish_at: Option<UtcInstant>,
    tags: Option<Vec<TagLabel>>,
    summary: Option<String>,
    audience: Option<AudienceSelection>,
) -> WebResult<UpdatePostResult> {
    boundary!("update_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        // Load old tags before mutation to union with new tags
        let old = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await?;
        let old_tag_slugs: BTreeSet<Tag> = old
            .as_ref()
            .map(|p| p.tags.iter().map(|t| t.tag_slug.clone()).collect())
            .unwrap_or_default();

        // Validate tags up-front so a malformed input rejects before any
        // post mutation lands. The wire delivers `Vec<TagLabel>` (validated at
        // arg-decode per ADR-0065); the body only dedups and caps. `None` leaves
        // the existing tags untouched.
        let new_tags = tags.map(common::tag::parse_and_validate_tags).transpose()?;

        let format = format.parse::<PostFormat>()?;
        let normalized_summary = summary.and_then(common::text::non_empty_owned);
        let audiences = audience_targets_or_public(audience.as_ref());

        // A supplied time schedules/backdates; `None` lets storage keep an
        // existing timestamp or stamp `now` for a not-yet-published post.
        let publish_at = publish_at.map(UtcInstant::value);

        let record = perform_post_update(
            posts.as_ref(),
            PostUpdate {
                post_id,
                editor_user_id: auth.user_id,
                body,
                title: None,
                format,
                slug_override: slug_override.as_ref(),
                publish: if publish {
                    PublishUpdate::Publish { at: publish_at }
                } else {
                    PublishUpdate::Unpublish
                },
                summary: normalized_summary,
                audiences,
            },
        )
        .await?;

        if let Some(new_tags) = new_tags {
            apply_post_tag_diff(posts.as_ref(), post_id, &new_tags).await?;
        }

        // Fetch current tags after mutation and union with old tags
        let current_tags = posts.get_tags_for_post(post_id).await?;
        let mut all_tag_slugs: BTreeSet<Tag> = old_tag_slugs;
        for tag in current_tags {
            all_tag_slugs.insert(tag.tag_slug);
        }

        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        enqueue_feed_events(feed_events.as_ref(), &auth.username, &all_tag_slugs)
            .await
            .map_err(InternalError::storage)?;

        let published_at = record.published_at.map(UtcInstant::from);
        // Only published posts have a public permalink. For drafts, the permalink is None.
        let permalink = record.published_at.is_some().then(|| record.permalink());

        host::metrics::post(host::metrics::PostEvent::Updated);
        Ok(UpdatePostResult {
            post_id,
            slug: record.slug,
            published_at,
            preview_url: format!("/draft/{post_id}/preview"),
            permalink,
            summary: record.summary,
        })
    })
}

/// Returns the audience-picker selection for a new post: the site-wide
/// default audience. Used to initialize the editor on the create page.
#[server(endpoint = "/default_audience_selection")]
pub async fn default_audience_selection() -> WebResult<AudienceSelection> {
    boundary!("default_audience_selection", {
        let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
        require_auth().await?;
        let default = site_config.get_default_audience().await?;
        Ok(targets_to_audience_selection(std::slice::from_ref(
            &default,
        )))
    })
}

/// Returns the audience-picker selection for an existing post (its current
/// targeting). Owner-only. Used to pre-select the editor on the edit page.
#[server(endpoint = "/post_audience_selection")]
pub async fn post_audience_selection(post_id: PostId) -> WebResult<AudienceSelection> {
    boundary!("post_audience_selection", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let auth = require_auth()
            .await
            .map_err(|e| private_post_not_found_error(&e))?;

        let post = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await?
            .ok_or_else(not_found_error)?;
        if post.deleted_at.is_some() || post.user_id != auth.user_id {
            return Err(not_found_error());
        }

        let targets = posts.get_post_audiences(post_id).await?;
        Ok(targets_to_audience_selection(&targets))
    })
}

/// Lists drafts for the authenticated user.
#[server(endpoint = "/list_drafts")]
pub async fn list_drafts(
    cursor_created_at: Option<UtcInstant>,
    cursor_post_id: Option<PostId>,
    limit: Option<PageSize>,
) -> WebResult<Vec<DraftSummary>> {
    boundary!("list_drafts", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let parsed_cursor =
            parse_post_cursor(cursor_created_at.map(UtcInstant::value), cursor_post_id)?;
        let page_size = limit.unwrap_or_default();
        let drafts = posts
            .list_drafts_by_user(
                auth.user_id,
                parsed_cursor.as_ref(),
                page_size.value(),
                chrono::Utc::now(),
            )
            .await?;

        Ok(drafts
            .into_iter()
            .map(|draft| {
                let permalink = draft.permalink();
                // `list_drafts_by_user` only returns drafts (`published_at`
                // NULL) and scheduled posts (`published_at` in the future), so
                // a `Some(published_at)` here is necessarily a scheduled time.
                let scheduled_at = draft.published_at.map(UtcInstant::from);
                DraftSummary {
                    post_id: draft.post_id,
                    title: draft.title.clone(),
                    summary_label: draft.fallback_summary_label(),
                    slug: draft.slug.clone(),
                    created_at: UtcInstant::from(draft.created_at),
                    updated_at: UtcInstant::from(draft.updated_at),
                    scheduled_at,
                    preview_url: format!("/draft/{}/preview", draft.post_id),
                    edit_url: format!("/posts/{}/edit", draft.post_id),
                    permalink,
                }
            })
            .collect())
    })
}

/// Publishes an existing draft owned by the authenticated user.
#[server(endpoint = "/publish_post")]
pub async fn publish_post(post_id: PostId) -> WebResult<PublishPostResult> {
    boundary!("publish_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let existing = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        // Preserve the post's existing audience targeting across publication
        // (chosen in the editor); publishing must not silently re-target it.
        let audiences = posts.get_post_audiences(post_id).await?;

        let updated = posts
            .update_post(
                post_id,
                auth.user_id,
                &UpdatePostInput {
                    title: existing.title,
                    slug: existing.slug,
                    body: existing.body,
                    format: existing.format,
                    rendered_html: existing.rendered_html,
                    summary: existing.summary,
                    unpublish: false,
                    explicit_published_at: None,
                    audiences,
                },
            )
            .await?;

        let published_at = updated
            .published_at
            .ok_or_else(|| InternalError::not_found("Post"))?;

        let tag_slugs: BTreeSet<Tag> = updated.tags.iter().map(|t| t.tag_slug.clone()).collect();
        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        enqueue_feed_events(feed_events.as_ref(), &updated.author_username, &tag_slugs)
            .await
            .map_err(InternalError::storage)?;

        host::metrics::post(host::metrics::PostEvent::Published);
        Ok(PublishPostResult {
            post_id: updated.post_id,
            slug: updated.slug.clone(),
            published_at: UtcInstant::from(published_at),
            permalink: updated.permalink(),
        })
    })
}

/// Soft-deletes a post owned by the authenticated user.
#[server(endpoint = "/delete_post")]
pub async fn delete_post(post_id: PostId) -> WebResult<()> {
    boundary!("delete_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let existing = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        posts.soft_delete_post(post_id).await?;

        // Only enqueue feed events for published posts
        if existing.published_at.is_some() {
            let tag_slugs: BTreeSet<Tag> =
                existing.tags.iter().map(|t| t.tag_slug.clone()).collect();
            let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
            enqueue_feed_events(feed_events.as_ref(), &existing.author_username, &tag_slugs)
                .await
                .map_err(InternalError::storage)?;
        }

        host::metrics::post(host::metrics::PostEvent::Deleted);
        Ok(())
    })
}

/// Reverts a published post owned by the authenticated user back to draft status.
#[server(endpoint = "/unpublish_post")]
pub async fn unpublish_post(post_id: PostId) -> WebResult<()> {
    boundary!("unpublish_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let existing = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        posts.unpublish_post(post_id).await?;

        let tag_slugs: BTreeSet<Tag> = existing.tags.iter().map(|t| t.tag_slug.clone()).collect();
        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        enqueue_feed_events(feed_events.as_ref(), &existing.author_username, &tag_slugs)
            .await
            .map_err(InternalError::storage)?;

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::{
        audience_selection_to_targets, audience_targets_or_public, targets_to_audience_selection,
        AudienceSelection,
    };
    use common::ids::AudienceId;
    use common::slug::Slug;
    use common::test_support::parse_username;
    use common::visibility::{AudienceBase, AudienceTarget};
    use storage::candidate_slug;

    fn selection(base: AudienceBase, named: &[AudienceId]) -> AudienceSelection {
        AudienceSelection {
            base,
            named: named.to_vec(),
        }
    }

    // A wire DTO's `rendered_html` survives a serde round-trip: `Serialize` writes
    // the raw string, and the `deserialize_with` trusted-rebuild reconstructs a
    // `RenderedHtml` (the type has no blanket `Deserialize`). Covers the sole wire
    // reconstruction door.
    #[test]
    fn timeline_summary_round_trips_rendered_html_via_trusted_rebuild() {
        use super::TimelinePostSummary;
        use common::ids::PostId;
        use common::render::RenderedHtml;
        use common::test_support::parse_utc_instant;

        let original = TimelinePostSummary {
            post_id: PostId::from(1),
            username: parse_username("alice"),
            title: Some("T".into()),
            summary: None,
            slug: "hello".parse::<Slug>().unwrap(),
            rendered_html: RenderedHtml::from_trusted("<p>hi</p>"),
            created_at: parse_utc_instant("2026-01-01T00:00:00Z"),
            published_at: parse_utc_instant("2026-01-01T00:00:00Z"),
            permalink: "/~alice/2026/01/01/hello".into(),
            is_author: false,
            tags: vec![],
        };
        let json = serde_json::to_string(&original).unwrap();
        let round_tripped: TimelinePostSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped.rendered_html.as_ref(), "<p>hi</p>");
        assert_eq!(round_tripped, original);
    }

    #[test]
    fn public_selection_maps_to_public_target() {
        assert_eq!(
            audience_selection_to_targets(&selection(AudienceBase::Public, &[])),
            vec![AudienceTarget::Public]
        );
    }

    #[test]
    fn subscribers_selection_maps_to_subscribers_target() {
        assert_eq!(
            audience_selection_to_targets(&selection(AudienceBase::Subscribers, &[])),
            vec![AudienceTarget::Subscribers]
        );
    }

    #[test]
    fn public_plus_named_unions() {
        assert_eq!(
            audience_selection_to_targets(&selection(
                AudienceBase::Public,
                &[AudienceId::from(5), AudienceId::from(9)]
            )),
            vec![
                AudienceTarget::Public,
                AudienceTarget::Named(AudienceId::from(5)),
                AudienceTarget::Named(AudienceId::from(9)),
            ]
        );
    }

    #[test]
    fn private_selection_is_empty_and_ignores_named() {
        // Private cannot combine with anything; named ids are dropped.
        assert!(audience_selection_to_targets(&selection(
            AudienceBase::Private,
            &[AudienceId::from(5)]
        ))
        .is_empty());
    }

    #[test]
    fn absent_selection_defaults_to_public() {
        assert_eq!(
            audience_targets_or_public(None),
            vec![AudienceTarget::Public]
        );
        // A present selection is translated normally.
        assert_eq!(
            audience_targets_or_public(Some(&selection(AudienceBase::Subscribers, &[]))),
            vec![AudienceTarget::Subscribers]
        );
    }

    #[test]
    fn targets_round_trip_through_selection() {
        // Edit round-trip: persisted targets -> selection -> targets.
        let targets = vec![
            AudienceTarget::Subscribers,
            AudienceTarget::Named(AudienceId::from(3)),
        ];
        let sel = targets_to_audience_selection(&targets);
        assert_eq!(
            sel,
            selection(AudienceBase::Subscribers, &[AudienceId::from(3)])
        );
        assert_eq!(audience_selection_to_targets(&sel), targets);

        // Public round-trips through the picker.
        let sel = targets_to_audience_selection(&[AudienceTarget::Public]);
        assert_eq!(sel, selection(AudienceBase::Public, &[]));
        assert_eq!(
            audience_selection_to_targets(&sel),
            vec![AudienceTarget::Public]
        );

        // An explicit Private element yields a private selection.
        assert_eq!(
            targets_to_audience_selection(&[AudienceTarget::Private]),
            selection(AudienceBase::Private, &[])
        );

        // No rows (private) round-trips to a private selection and back to empty.
        let empty: Vec<AudienceTarget> = Vec::new();
        let sel = targets_to_audience_selection(&empty);
        assert_eq!(sel, selection(AudienceBase::Private, &[]));
        assert!(audience_selection_to_targets(&sel).is_empty());
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

    #[cfg(feature = "server")]
    #[test]
    fn timeline_post_summary_keeps_titleless_posts_titleless() {
        use crate::posts::server::timeline_post_summary;
        use chrono::{TimeZone, Utc};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
        };
        use storage::{PostFormat, PostRecord, RenderedHtml};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let slug = "titleless-note".parse::<Slug>().unwrap();

        let summary = timeline_post_summary(
            PostRecord {
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username: parse_username("author"),
                title: None,
                slug,
                body: "Titleless note".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>Titleless note</p>"),
                created_at: base_time,
                updated_at: base_time,
                published_at: Some(base_time),
                deleted_at: None,
                summary: None,
                tags: vec![],
            },
            None,
        )
        .expect("published post should summarize");

        assert_eq!(summary.title, None);
        assert_eq!(summary.username, "author");
        assert_eq!(summary.permalink, "/~author/2026/04/16/titleless-note");
    }

    #[cfg(feature = "server")]
    #[test]
    fn post_response_marks_draft_state_from_published_at() {
        use crate::posts::server::post_response;
        use chrono::{TimeZone, Utc};
        use common::{
            ids::{PostId, UserId},
            slug::Slug,
        };
        use storage::{PostFormat, PostRecord, RenderedHtml};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let author_username = parse_username("author");
        let slug = "hello-world".parse::<Slug>().unwrap();

        let draft = post_response(
            PostRecord {
                post_id: PostId::from(1),
                user_id: UserId::from(2),
                author_username: author_username.clone(),
                title: Some("Draft".into()),
                slug: slug.clone(),
                body: "body".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                created_at: base_time,
                updated_at: base_time,
                published_at: None,
                deleted_at: None,
                summary: None,
                tags: vec![],
            },
            true,
        );
        assert!(draft.is_draft);
        assert!(draft.published_at.is_none());
        assert_eq!(draft.username, "author");

        let published = post_response(
            PostRecord {
                post_id: PostId::from(2),
                user_id: UserId::from(2),
                author_username,
                title: Some("Published".into()),
                slug,
                body: "body".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
                created_at: base_time,
                updated_at: base_time,
                published_at: Some(base_time),
                deleted_at: None,
                summary: None,
                tags: vec![],
            },
            false,
        );
        assert!(!published.is_draft);
        assert!(published.published_at.is_some());
    }
}

#[cfg(all(test, feature = "server"))]
mod server_tests {
    // Helper fns in this feature-gated test module aren't covered by clippy's
    // allow-{unwrap,expect}-in-tests, so allow the test-scaffolding panics.
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::publish_post;
    use crate::error::WebError;
    use crate::test_support::auth_parts;
    use chrono::Utc;
    use common::ids::{ChannelId, PostId, UserId};
    use common::slug::Slug;
    use common::test_support::parse_username;
    use leptos::prelude::provide_context;
    use leptos::reactive::owner::Owner;
    use std::sync::Arc;
    use storage::{
        MockPostStorage, MockSubscriptionStorage, PostFormat, PostRecord, PostStorage,
        RenderedHtml, SubscriptionStorage, UpdatePostError,
    };

    fn owned_post(user_id: UserId) -> PostRecord {
        let now = Utc::now();
        PostRecord {
            post_id: PostId::from(1),
            user_id,
            author_username: parse_username("alice"),
            title: Some("t".into()),
            slug: "hello-world".parse::<Slug>().unwrap(),
            body: "body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
            created_at: now,
            updated_at: now,
            published_at: None,
            deleted_at: None,
            summary: None,
            tags: vec![],
        }
    }

    /// Wires an authenticated owner (user 1) whose post store returns an owned,
    /// non-deleted post but fails `update_post` with `error`. Returns the owner,
    /// which the caller must keep alive across the `.await`.
    fn setup(error: fn() -> UpdatePostError) -> Owner {
        let owner = Owner::new();
        owner.set();
        provide_context(auth_parts(UserId::from(1), "alice"));
        let mut posts = MockPostStorage::new();
        posts
            .expect_get_post_by_id()
            .returning(|_id, _viewer| Ok(Some(owned_post(UserId::from(1)))));
        posts
            .expect_get_post_audiences()
            .returning(|_id| Ok(Vec::new()));
        posts
            .expect_update_post()
            .returning(move |_id, _editor, _input| Err(error()));
        provide_context(Arc::new(posts) as Arc<dyn PostStorage>);
        // `viewer_identity()` (used to fetch the post) may consult the local
        // channel id; allow it zero-or-more times.
        let mut subs = MockSubscriptionStorage::new();
        subs.expect_local_channel_id()
            .times(0..)
            .returning(|| Ok(ChannelId::from(1)));
        provide_context(Arc::new(subs) as Arc<dyn SubscriptionStorage>);
        owner
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn publish_post_maps_not_found_update_error_to_not_found() {
        let owner = setup(|| UpdatePostError::NotFound);
        let result = publish_post(PostId::from(1)).await;
        drop(owner);
        assert!(matches!(result.unwrap_err(), WebError::NotFound { .. }));
    }

    // guard:no-backend — mock store
    #[tokio::test]
    async fn publish_post_maps_internal_update_error_to_storage() {
        let owner = setup(|| UpdatePostError::Internal(sqlx::Error::PoolClosed));
        let result = publish_post(PostId::from(1)).await;
        drop(owner);
        assert!(matches!(result.unwrap_err(), WebError::Storage { .. }));
    }
}
