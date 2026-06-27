use leptos::prelude::*;
use leptos::server_fn::codec::Json;
use serde::{Deserialize, Serialize};

#[cfg(feature = "ssr")]
mod server;

/// Timeline/listing endpoints, split out from the single-post lifecycle below.
/// Re-exported so `crate::posts::list_*` / `TimelinePage` keep resolving.
mod listing;
pub use listing::*;

#[cfg(feature = "ssr")]
use server::{
    apply_post_tag_diff, find_draft_by_permalink_for_user, not_found_error, parse_post_cursor,
    perform_creation_error, perform_update_error, post_response, private_post_not_found_error,
};

use crate::error::WebResult;
use crate::tags::TagSummary;

// SSR-only imports for #[server] bodies
#[cfg(feature = "ssr")]
use {
    crate::auth::require_auth,
    crate::error::InternalError,
    crate::feed_events::enqueue_feed_events,
    crate::viewer::viewer_identity,
    chrono::{DateTime, NaiveDate, Utc},
    common::{slug::Slug, tag::Tag, username::Username},
    std::{collections::BTreeSet, sync::Arc},
    storage::{
        perform_post_creation, perform_post_update, FeedEventStorage, PerformUpdateError,
        PostCreation, PostFormat, PostStorage, PostUpdate, PublishUpdate, SiteConfigStorage,
        UpdatePostError, UpdatePostInput,
    },
};

/// Result returned by [`create_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreatePostResult {
    pub post_id: i64,
    pub slug: String,
    pub created_at: String,
    pub published_at: Option<String>,
    pub preview_url: String,
    pub permalink: Option<String>,
    pub summary: Option<String>,
}

/// Result returned by [`update_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdatePostResult {
    pub post_id: i64,
    pub slug: String,
    pub published_at: Option<String>,
    pub preview_url: String,
    pub permalink: Option<String>,
    pub summary: Option<String>,
}

/// The audience-picker selection as it crosses the server-fn boundary.
///
/// `base` is the mutually-exclusive built-in (`"public"`, `"private"`, or
/// `"subscribers"`); `named` is the set of selected named-audience ids. The
/// two compose by UNION except for `"private"`, which is author-only and
/// cannot combine with anything — a `"private"` base discards `named`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AudienceSelection {
    pub base: String,
    pub named: Vec<i64>,
}

/// Translates an [`AudienceSelection`] into the `Vec<AudienceTarget>` the
/// storage layer persists.
///
/// - `"public"` / `"subscribers"` → the built-in target, in union with one
///   `Named(id)` per selected named audience.
/// - `"private"` (or any unrecognized base) → an empty vec (author-only); the
///   named set is ignored, since `Private` cannot combine with other targets.
#[must_use]
pub fn audience_selection_to_targets(
    selection: &AudienceSelection,
) -> Vec<common::visibility::AudienceTarget> {
    use common::visibility::AudienceTarget;
    let base = match selection.base.as_str() {
        "public" => Some(AudienceTarget::Public),
        "subscribers" => Some(AudienceTarget::Subscribers),
        // "private" and anything unrecognized fall through to author-only.
        _ => None,
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
/// The built-in base is `"public"`/`"subscribers"` when that target is present,
/// otherwise `"private"` (covering both an explicit `Private` and an empty
/// targeting). Every `Named(id)` becomes an entry in `named`.
#[must_use]
pub fn targets_to_audience_selection(
    targets: &[common::visibility::AudienceTarget],
) -> AudienceSelection {
    use common::visibility::AudienceTarget;
    let mut base = "private";
    let mut named = Vec::new();
    for target in targets {
        match target {
            AudienceTarget::Public => base = "public",
            AudienceTarget::Subscribers => base = "subscribers",
            AudienceTarget::Named(id) => named.push(*id),
            AudienceTarget::Private => {}
        }
    }
    AudienceSelection {
        base: base.to_string(),
        named,
    }
}

/// A draft row returned by [`list_drafts`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DraftSummary {
    pub post_id: i64,
    pub title: Option<String>,
    pub summary_label: String,
    pub slug: String,
    pub created_at: String,
    pub updated_at: String,
    /// RFC3339 UTC publication instant for a *scheduled* post (`published_at`
    /// in the future); `None` for true drafts. Drives the "Scheduled for …"
    /// author marker.
    pub scheduled_at: Option<String>,
    pub preview_url: String,
    pub edit_url: String,
    pub permalink: String,
}

/// Result returned by [`publish_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishPostResult {
    pub post_id: i64,
    pub slug: String,
    pub published_at: String,
    pub permalink: String,
}

/// Details of a post returned by [`get_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostResponse {
    pub post_id: i64,
    pub username: String,
    pub title: Option<String>,
    pub slug: String,
    pub body: String,
    pub format: String,
    pub rendered_html: String,
    pub created_at: String,
    pub published_at: Option<String>,
    pub is_draft: bool,
    pub is_author: bool,
    /// Permalink URL for published posts; `None` for drafts.
    pub permalink: Option<String>,
    /// Tags applied to this post, ordered by canonical slug.
    pub tags: Vec<TagSummary>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<String>,
}

/// Parses the wire `publish_at` (an optional RFC3339 UTC instant from the
/// compose/editor datetime control) into a `DateTime<Utc>`. An absent or
/// blank value is `None`; a present-but-unparseable value is a validation
/// error.
#[cfg(feature = "ssr")]
fn parse_publish_at(raw: Option<&str>) -> crate::error::InternalResult<Option<DateTime<Utc>>> {
    raw.and_then(common::text::non_empty)
        .map(|s| {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| InternalError::validation(format!("invalid publish_at: {e}")))
        })
        .transpose()
}

/// Creates a post for the authenticated user.
///
/// `publish_at` is an optional RFC3339 UTC instant supplied by the compose
/// form's datetime control. It is carried as a `String` (not `DateTime<Utc>`)
/// because `chrono` is an `ssr`-only dependency here and the server-fn
/// signature must also compile for the wasm client. The wire is UTC; the
/// browser converts the author's local `datetime-local` value before sending.
#[allow(clippy::too_many_arguments)]
#[server(endpoint = "/create_post", input = Json)]
pub async fn create_post(
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
    publish_at: Option<String>,
    tags: Option<Vec<String>>,
    summary: Option<String>,
    audience: Option<AudienceSelection>,
) -> WebResult<CreatePostResult> {
    boundary!("create_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let validated_tags = common::tag::parse_and_validate_tags(tags.unwrap_or_default())
            .map_err(|e| InternalError::validation(e.to_string()))?;

        let format = format
            .parse::<PostFormat>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        // Publish + a supplied time = scheduled (future) or backdated (past);
        // publish + no time = live now; not publishing = draft (NULL).
        let published_at = if publish {
            Some(match parse_publish_at(publish_at.as_deref())? {
                Some(at) => at,
                None => Utc::now(),
            })
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
                slug_override: slug_override.as_deref(),
                published_at,
                max_attempts: 100,
                summary: normalized_summary,
                audiences,
            },
        )
        .await
        .map_err(perform_creation_error)?;

        let created_at = record.created_at.to_rfc3339();
        let published_at_str = record.published_at.map(|timestamp| timestamp.to_rfc3339());
        // Only published posts have a public permalink. For drafts, the permalink is None.
        let permalink = record.published_at.is_some().then(|| record.permalink());
        let preview_url = format!("/draft/{}/preview", record.post_id);

        let created = CreatePostResult {
            post_id: record.post_id,
            slug: record.slug.to_string(),
            created_at,
            published_at: published_at_str,
            preview_url,
            permalink,
            summary: record.summary,
        };

        for display in &validated_tags {
            posts
                .tag_post(created.post_id, display)
                .await
                .map_err(|e| InternalError::server_message(e.to_string()))?;
        }

        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        let tag_post_tags = posts
            .get_tags_for_post(created.post_id)
            .await
            .map_err(InternalError::storage)?;
        let tag_slugs: BTreeSet<Tag> = tag_post_tags.iter().map(|t| t.tag_slug.clone()).collect();
        enqueue_feed_events(feed_events.as_ref(), &auth.username, &tag_slugs)
            .await
            .map_err(InternalError::storage)?;

        common::metrics::post(common::metrics::PostEvent::Created);
        Ok(created)
    })
}

/// Retrieves a post by its permalink.
#[server(endpoint = "/get_post")]
pub async fn get_post(
    username: String,
    year: i32,
    month: u32,
    day: u32,
    slug: String,
) -> WebResult<PostResponse> {
    boundary!("get_post", {
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let username_parsed = username
            .parse::<Username>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let slug_parsed = slug
            .parse::<Slug>()
            .map_err(|e| InternalError::validation(e.to_string()))?;

        NaiveDate::from_ymd_opt(year, month, day)
            .ok_or_else(|| InternalError::validation("Invalid permalink"))?;

        let viewer = viewer_identity().await;
        if let Some(post) = posts
            .get_post_by_permalink(
                &username_parsed,
                year,
                month,
                day,
                &slug_parsed,
                &viewer,
                chrono::Utc::now(),
            )
            .await
            .map_err(InternalError::storage)?
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
        let auth = require_auth().await.map_err(private_post_not_found_error)?;
        if auth.username != username_parsed {
            return Err(not_found_error());
        }

        let draft = find_draft_by_permalink_for_user(
            posts.as_ref(),
            auth.user_id,
            year,
            month,
            day,
            &slug_parsed,
        )
        .await?
        .ok_or_else(not_found_error)?;

        Ok(post_response(draft, true))
    })
}

/// Retrieves a draft preview for the authenticated author.
#[server(endpoint = "/get_post_preview")]
pub async fn get_post_preview(post_id: i64) -> WebResult<PostResponse> {
    boundary!("get_post_preview", {
        let auth = require_auth().await.map_err(private_post_not_found_error)?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let post = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(not_found_error)?;

        if post.deleted_at.is_some() || post.user_id != auth.user_id {
            return Err(not_found_error());
        }

        Ok(post_response(post, true))
    })
}

/// Updates an existing post for the authenticated author.
#[allow(clippy::too_many_arguments)]
#[server(endpoint = "/update_post", input = Json)]
pub async fn update_post(
    post_id: i64,
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
    // Optional RFC3339 UTC instant from the editor's datetime control. See
    // `create_post` for why this crosses the boundary as a `String`.
    publish_at: Option<String>,
    tags: Option<Vec<String>>,
    summary: Option<String>,
    audience: Option<AudienceSelection>,
) -> WebResult<UpdatePostResult> {
    boundary!("update_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        // Load old tags before mutation to union with new tags
        let old = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await
            .map_err(InternalError::storage)?;
        let old_tag_slugs: BTreeSet<Tag> = old
            .as_ref()
            .map(|p| p.tags.iter().map(|t| t.tag_slug.clone()).collect())
            .unwrap_or_default();

        // Validate tags up-front so a malformed input rejects before any
        // post mutation lands.
        let new_tags = tags
            .map(|t| {
                common::tag::parse_and_validate_tags(t)
                    .map_err(|e| InternalError::validation(e.to_string()))
            })
            .transpose()?;

        let format = format
            .parse::<PostFormat>()
            .map_err(|e| InternalError::validation(e.to_string()))?;
        let normalized_summary = summary.and_then(common::text::non_empty_owned);
        let audiences = audience_targets_or_public(audience.as_ref());

        // A supplied time schedules/backdates; `None` lets storage keep an
        // existing timestamp or stamp `now` for a not-yet-published post.
        let publish_at = parse_publish_at(publish_at.as_deref())?;

        let record = perform_post_update(
            posts.as_ref(),
            PostUpdate {
                post_id,
                editor_user_id: auth.user_id,
                body,
                title: None,
                format,
                slug_override: slug_override.as_deref(),
                publish: if publish {
                    PublishUpdate::Publish { at: publish_at }
                } else {
                    PublishUpdate::Unpublish
                },
                summary: normalized_summary,
                audiences,
            },
        )
        .await
        .map_err(|e| match e {
            PerformUpdateError::NotFound | PerformUpdateError::Unauthorized => {
                InternalError::not_found("Post")
            }
            other => perform_update_error(other),
        })?;

        if let Some(new_tags) = new_tags {
            apply_post_tag_diff(posts.as_ref(), post_id, &new_tags).await?;
        }

        // Fetch current tags after mutation and union with old tags
        let current_tags = posts
            .get_tags_for_post(post_id)
            .await
            .map_err(InternalError::storage)?;
        let mut all_tag_slugs: BTreeSet<Tag> = old_tag_slugs;
        for tag in current_tags {
            all_tag_slugs.insert(tag.tag_slug);
        }

        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        enqueue_feed_events(feed_events.as_ref(), &auth.username, &all_tag_slugs)
            .await
            .map_err(InternalError::storage)?;

        let published_at_str = record.published_at.map(|t| t.to_rfc3339());
        // Only published posts have a public permalink. For drafts, the permalink is None.
        let permalink = record.published_at.is_some().then(|| record.permalink());

        common::metrics::post(common::metrics::PostEvent::Updated);
        Ok(UpdatePostResult {
            post_id,
            slug: record.slug.to_string(),
            published_at: published_at_str,
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
        let default = site_config
            .get_default_audience()
            .await
            .map_err(InternalError::storage)?;
        Ok(targets_to_audience_selection(std::slice::from_ref(
            &default,
        )))
    })
}

/// Returns the audience-picker selection for an existing post (its current
/// targeting). Owner-only. Used to pre-select the editor on the edit page.
#[server(endpoint = "/post_audience_selection")]
pub async fn post_audience_selection(post_id: i64) -> WebResult<AudienceSelection> {
    boundary!("post_audience_selection", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let auth = require_auth().await.map_err(private_post_not_found_error)?;

        let post = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(not_found_error)?;
        if post.deleted_at.is_some() || post.user_id != auth.user_id {
            return Err(not_found_error());
        }

        let targets = posts
            .get_post_audiences(post_id)
            .await
            .map_err(InternalError::storage)?;
        Ok(targets_to_audience_selection(&targets))
    })
}

/// Lists drafts for the authenticated user.
#[server(endpoint = "/list_drafts")]
pub async fn list_drafts(
    cursor_created_at: Option<String>,
    cursor_post_id: Option<i64>,
    limit: Option<u32>,
) -> WebResult<Vec<DraftSummary>> {
    boundary!("list_drafts", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let parsed_cursor = parse_post_cursor(cursor_created_at, cursor_post_id)?;
        let page_size = limit.unwrap_or(50).clamp(1, 50);
        let drafts = posts
            .list_drafts_by_user(
                auth.user_id,
                parsed_cursor.as_ref(),
                page_size,
                chrono::Utc::now(),
            )
            .await
            .map_err(InternalError::storage)?;

        Ok(drafts
            .into_iter()
            .map(|draft| {
                let permalink = draft.permalink();
                // `list_drafts_by_user` only returns drafts (`published_at`
                // NULL) and scheduled posts (`published_at` in the future), so
                // a `Some(published_at)` here is necessarily a scheduled time.
                let scheduled_at = draft.published_at.map(|t| t.to_rfc3339());
                DraftSummary {
                    post_id: draft.post_id,
                    title: draft.title.clone(),
                    summary_label: draft.fallback_summary_label(),
                    slug: draft.slug.to_string(),
                    created_at: draft.created_at.to_rfc3339(),
                    updated_at: draft.updated_at.to_rfc3339(),
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
pub async fn publish_post(post_id: i64) -> WebResult<PublishPostResult> {
    boundary!("publish_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let existing = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        // Preserve the post's existing audience targeting across publication
        // (chosen in the editor); publishing must not silently re-target it.
        let audiences = posts
            .get_post_audiences(post_id)
            .await
            .map_err(InternalError::storage)?;

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
            .await
            .map_err(|e| match e {
                UpdatePostError::NotFound | UpdatePostError::Unauthorized => {
                    InternalError::not_found("Post")
                }
                UpdatePostError::Internal(error) => InternalError::storage(error),
            })?;

        let published_at = updated
            .published_at
            .ok_or_else(|| InternalError::not_found("Post"))?;

        let tag_slugs: BTreeSet<Tag> = updated.tags.iter().map(|t| t.tag_slug.clone()).collect();
        let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
        enqueue_feed_events(feed_events.as_ref(), &updated.author_username, &tag_slugs)
            .await
            .map_err(InternalError::storage)?;

        common::metrics::post(common::metrics::PostEvent::Published);
        Ok(PublishPostResult {
            post_id: updated.post_id,
            slug: updated.slug.to_string(),
            published_at: published_at.to_rfc3339(),
            permalink: updated.permalink(),
        })
    })
}

/// Soft-deletes a post owned by the authenticated user.
#[server(endpoint = "/delete_post")]
pub async fn delete_post(post_id: i64) -> WebResult<()> {
    boundary!("delete_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let existing = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        posts
            .soft_delete_post(post_id)
            .await
            .map_err(InternalError::storage)?;

        // Only enqueue feed events for published posts
        if existing.published_at.is_some() {
            let tag_slugs: BTreeSet<Tag> =
                existing.tags.iter().map(|t| t.tag_slug.clone()).collect();
            let feed_events = expect_context::<Arc<dyn FeedEventStorage>>();
            enqueue_feed_events(feed_events.as_ref(), &existing.author_username, &tag_slugs)
                .await
                .map_err(InternalError::storage)?;
        }

        common::metrics::post(common::metrics::PostEvent::Deleted);
        Ok(())
    })
}

/// Reverts a published post owned by the authenticated user back to draft status.
#[server(endpoint = "/unpublish_post")]
pub async fn unpublish_post(post_id: i64) -> WebResult<()> {
    boundary!("unpublish_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        let existing = posts
            .get_post_by_id(post_id, &viewer_identity().await)
            .await
            .map_err(InternalError::storage)?
            .ok_or_else(|| InternalError::not_found("Post"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(InternalError::not_found("Post"));
        }

        posts
            .unpublish_post(post_id)
            .await
            .map_err(InternalError::storage)?;

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
    use common::visibility::AudienceTarget;
    use storage::candidate_slug;

    fn selection(base: &str, named: &[i64]) -> AudienceSelection {
        AudienceSelection {
            base: base.to_string(),
            named: named.to_vec(),
        }
    }

    #[test]
    fn public_selection_maps_to_public_target() {
        assert_eq!(
            audience_selection_to_targets(&selection("public", &[])),
            vec![AudienceTarget::Public]
        );
    }

    #[test]
    fn subscribers_selection_maps_to_subscribers_target() {
        assert_eq!(
            audience_selection_to_targets(&selection("subscribers", &[])),
            vec![AudienceTarget::Subscribers]
        );
    }

    #[test]
    fn public_plus_named_unions() {
        assert_eq!(
            audience_selection_to_targets(&selection("public", &[5, 9])),
            vec![
                AudienceTarget::Public,
                AudienceTarget::Named(5),
                AudienceTarget::Named(9),
            ]
        );
    }

    #[test]
    fn private_selection_is_empty_and_ignores_named() {
        // Private cannot combine with anything; named ids are dropped.
        assert!(audience_selection_to_targets(&selection("private", &[5])).is_empty());
        // An unrecognized base also falls through to author-only.
        assert!(audience_selection_to_targets(&selection("nonsense", &[])).is_empty());
    }

    #[test]
    fn absent_selection_defaults_to_public() {
        assert_eq!(
            audience_targets_or_public(None),
            vec![AudienceTarget::Public]
        );
        // A present selection is translated normally.
        assert_eq!(
            audience_targets_or_public(Some(&selection("subscribers", &[]))),
            vec![AudienceTarget::Subscribers]
        );
    }

    #[test]
    fn targets_round_trip_through_selection() {
        // Edit round-trip: persisted targets -> selection -> targets.
        let targets = vec![AudienceTarget::Subscribers, AudienceTarget::Named(3)];
        let sel = targets_to_audience_selection(&targets);
        assert_eq!(sel, selection("subscribers", &[3]));
        assert_eq!(audience_selection_to_targets(&sel), targets);

        // Public round-trips through the picker.
        let sel = targets_to_audience_selection(&[AudienceTarget::Public]);
        assert_eq!(sel, selection("public", &[]));
        assert_eq!(
            audience_selection_to_targets(&sel),
            vec![AudienceTarget::Public]
        );

        // An explicit Private element yields a private selection.
        assert_eq!(
            targets_to_audience_selection(&[AudienceTarget::Private]),
            selection("private", &[])
        );

        // No rows (private) round-trips to a private selection and back to empty.
        let empty: Vec<AudienceTarget> = Vec::new();
        let sel = targets_to_audience_selection(&empty);
        assert_eq!(sel, selection("private", &[]));
        assert!(audience_selection_to_targets(&sel).is_empty());
    }

    #[test]
    fn candidate_slug_returns_seed_for_first_attempt() {
        assert_eq!(candidate_slug("hello-world", 0), "hello-world");
    }

    #[test]
    fn candidate_slug_appends_numeric_suffix_after_conflict() {
        assert_eq!(candidate_slug("hello-world", 1), "hello-world-2");
        assert_eq!(candidate_slug("hello-world", 2), "hello-world-3");
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn parse_post_cursor_accepts_empty_cursor() {
        use crate::posts::server::parse_post_cursor;

        let cursor = parse_post_cursor(None, None).unwrap();
        assert!(cursor.is_none());
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn timeline_post_summary_keeps_titleless_posts_titleless() {
        use crate::posts::server::timeline_post_summary;
        use chrono::{TimeZone, Utc};
        use common::{slug::Slug, username::Username};
        use storage::{PostFormat, PostRecord};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let slug = "titleless-note".parse::<Slug>().unwrap();

        let summary = timeline_post_summary(
            PostRecord {
                post_id: 1,
                user_id: 2,
                author_username: "author".parse::<Username>().unwrap(),
                title: None,
                slug,
                body: "Titleless note".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Titleless note</p>".to_string(),
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

    #[cfg(feature = "ssr")]
    #[test]
    fn post_response_marks_draft_state_from_published_at() {
        use crate::posts::server::post_response;
        use chrono::{TimeZone, Utc};
        use common::{slug::Slug, username::Username};
        use storage::{PostFormat, PostRecord};

        let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
        let author_username = "author".parse::<Username>().unwrap();
        let slug = "hello-world".parse::<Slug>().unwrap();

        let draft = post_response(
            PostRecord {
                post_id: 1,
                user_id: 2,
                author_username: author_username.clone(),
                title: Some("Draft".to_string()),
                slug: slug.clone(),
                body: "body".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
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
                post_id: 2,
                user_id: 2,
                author_username,
                title: Some("Published".to_string()),
                slug,
                body: "body".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>body</p>".to_string(),
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
