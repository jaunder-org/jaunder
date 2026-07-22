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
    ids::PostId,
    pagination::PageSize,
    post_body::PostBody,
    post_summary::PostSummary,
    post_title::PostTitle,
    render::{PostFormat, RenderedHtml},
    root_relative_url::RootRelativeUrl,
    slug::Slug,
    tag::TagLabel,
    time::UtcInstant,
    username::Username,
    visibility::AudienceSelection,
};

use crate::error::WebResult;
use crate::tags::TagSummary;

// The audience-picker DTO and its converters live in `common::visibility` (beside
// `AudienceBase`/`AudienceTarget`); the server fn bodies below use these two to
// translate the wire `AudienceSelection` to/from the domain `AudienceTarget`s. The
// calls are server-only (inside `boundary!`), so the import is gated to match.
#[cfg(feature = "server")]
use common::visibility::{audience_targets_or_public, targets_to_audience_selection};

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
        PostCreation, PostStorage, PostUpdate, PublishUpdate, SiteConfigStorage, UpdatePostInput,
    },
};

/// Wraps a server-composed `/…` path (draft-preview / edit routes) as a
/// [`RootRelativeUrl`]. The path is built from a known-valid literal template, so
/// the parse cannot fail; the `unreachable!` arm keeps a genuine panic branch
/// (never an uncovered `expect`).
#[cfg(feature = "server")]
fn root_relative_path(path: String) -> RootRelativeUrl {
    let Ok(url) = RootRelativeUrl::try_from(path) else {
        unreachable!("server composes a valid root-relative path");
    };
    url
}

/// Result returned by [`create_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreatePostResult {
    pub post_id: PostId,
    pub slug: Slug,
    pub created_at: UtcInstant,
    pub published_at: Option<UtcInstant>,
    pub preview_url: RootRelativeUrl,
    pub permalink: Option<RootRelativeUrl>,
    pub summary: Option<PostSummary>,
}

/// Result returned by [`update_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdatePostResult {
    pub post_id: PostId,
    pub slug: Slug,
    pub published_at: Option<UtcInstant>,
    pub preview_url: RootRelativeUrl,
    pub permalink: Option<RootRelativeUrl>,
    pub summary: Option<PostSummary>,
}

/// A draft row returned by [`list_drafts`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DraftSummary {
    pub post_id: PostId,
    pub title: Option<PostTitle>,
    pub summary_label: PostSummary,
    pub slug: Slug,
    pub created_at: UtcInstant,
    pub updated_at: UtcInstant,
    /// UTC publication instant for a *scheduled* post (`published_at`
    /// in the future); `None` for true drafts. Drives the "Scheduled for …"
    /// author marker.
    pub scheduled_at: Option<UtcInstant>,
    pub preview_url: RootRelativeUrl,
    pub edit_url: RootRelativeUrl,
    pub permalink: RootRelativeUrl,
}

/// Result returned by [`publish_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublishPostResult {
    pub post_id: PostId,
    pub slug: Slug,
    pub published_at: UtcInstant,
    pub permalink: RootRelativeUrl,
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
    pub format: PostFormat,
    #[serde(deserialize_with = "deserialize_rendered_html")]
    pub rendered_html: RenderedHtml,
    pub created_at: UtcInstant,
    pub published_at: Option<UtcInstant>,
    pub is_draft: bool,
    pub is_author: bool,
    /// Permalink URL for published posts; `None` for drafts.
    pub permalink: Option<RootRelativeUrl>,
    /// Tags applied to this post, ordered by canonical slug.
    pub tags: Vec<TagSummary>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<PostSummary>,
}

/// Bundled arguments for [`create_post`]. The eight fields are the RPC input
/// contract; bundling them into a typed struct nests the JSON wire under `args`
/// (#299).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePostArgs {
    pub body: PostBody,
    pub format: PostFormat,
    pub slug_override: Option<Slug>,
    pub publish: bool,
    pub publish_at: Option<UtcInstant>,
    pub tags: Option<Vec<TagLabel>>,
    pub summary: Option<PostSummary>,
    pub audience: Option<AudienceSelection>,
}

/// Bundled arguments for [`update_post`]. Like [`CreatePostArgs`] with a leading
/// `post_id`; bundling nests the JSON wire under `args` (#299).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePostArgs {
    pub post_id: PostId,
    pub body: PostBody,
    pub format: PostFormat,
    pub slug_override: Option<Slug>,
    pub publish: bool,
    pub publish_at: Option<UtcInstant>,
    pub tags: Option<Vec<TagLabel>>,
    pub summary: Option<PostSummary>,
    pub audience: Option<AudienceSelection>,
}

/// Creates a post for the authenticated user.
///
/// `publish_at` is an optional UTC instant supplied by the compose form's
/// datetime control, carried as a [`UtcInstant`] (serde-transparent over an
/// RFC 3339 wire string; expressible in the `#[server]` signature on both the
/// server and the wasm client). The browser converts the author's local
/// `datetime-local` value to UTC before sending.
#[server(endpoint = "/create_post", input = Json)]
pub async fn create_post(args: CreatePostArgs) -> WebResult<CreatePostResult> {
    let CreatePostArgs {
        body,
        format,
        slug_override,
        publish,
        publish_at,
        tags,
        summary,
        audience,
    } = args;
    boundary!("create_post", {
        let auth = require_auth().await?;
        let posts = expect_context::<Arc<dyn PostStorage>>();

        // The wire delivers `Vec<TagLabel>` directly: each tag is validated at
        // arg-decode (ADR-0065) and a `TagLabel` is never empty, so the body only
        // dedups and enforces the per-post cap.
        let validated_tags = common::tag::parse_and_validate_tags(tags.unwrap_or_default())?;

        // Publish + a supplied time = scheduled (future) or backdated (past);
        // publish + no time = live now; not publishing = draft (NULL).
        let published_at = if publish {
            Some(publish_at.map_or_else(Utc::now, UtcInstant::value))
        } else {
            None
        };
        // `PostSummary`'s `FromStr` already trims and rejects empty at arg-decode
        // (ADR-0065), so the value is passed through typed — no `non_empty_owned`
        // normalization needed.
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
                summary,
                audiences,
                idempotency_key: None,
            },
        )
        .await?;

        let created_at = UtcInstant::from(record.created_at);
        let published_at = record.published_at.map(UtcInstant::from);
        // Only published posts have a public permalink. For drafts, the permalink is None.
        let permalink = record.published_at.is_some().then(|| record.permalink());
        let preview_url = root_relative_path(format!("/draft/{}/preview", record.post_id));

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
///
/// `publish_at` is an optional UTC instant from the editor's datetime control.
/// See `create_post` for why it crosses the boundary as a [`UtcInstant`].
#[server(endpoint = "/update_post", input = Json)]
pub async fn update_post(args: UpdatePostArgs) -> WebResult<UpdatePostResult> {
    let UpdatePostArgs {
        post_id,
        body,
        format,
        slug_override,
        publish,
        publish_at,
        tags,
        summary,
        audience,
    } = args;
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

        // See `create_post`: the typed `PostSummary` arg is already validated at
        // decode, so no `non_empty_owned` normalization is applied here.
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
                summary,
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
            preview_url: root_relative_path(format!("/draft/{post_id}/preview")),
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
                    preview_url: root_relative_path(format!("/draft/{}/preview", draft.post_id)),
                    edit_url: root_relative_path(format!("/posts/{}/edit", draft.post_id)),
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
    use common::slug::Slug;
    use common::test_support::parse_username;
    use storage::candidate_slug;

    // A wire DTO's `rendered_html` survives a serde round-trip: `Serialize` writes
    // the raw string, and the `deserialize_with` trusted-rebuild reconstructs a
    // `RenderedHtml` (the type has no blanket `Deserialize`). Covers the sole wire
    // reconstruction door.
    #[test]
    fn timeline_summary_round_trips_rendered_html_via_trusted_rebuild() {
        use super::TimelinePostSummary;
        use common::ids::PostId;
        use common::render::RenderedHtml;
        use common::test_support::{parse_root_relative_url, parse_utc_instant};

        let original = TimelinePostSummary {
            post_id: PostId::from(1),
            username: parse_username("alice"),
            title: Some("T".into()),
            summary: None,
            slug: "hello".parse::<Slug>().unwrap(),
            rendered_html: RenderedHtml::from_trusted("<p>hi</p>"),
            created_at: parse_utc_instant("2026-01-01T00:00:00Z"),
            published_at: parse_utc_instant("2026-01-01T00:00:00Z"),
            permalink: Some(parse_root_relative_url("/~alice/2026/01/01/hello")),
            is_author: false,
            tags: vec![],
        };
        let json = serde_json::to_string(&original).unwrap();
        let round_tripped: TimelinePostSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped.rendered_html.as_ref(), "<p>hi</p>");
        assert_eq!(round_tripped, original);
    }

    // The typed `RootRelativeUrl` permalink field pins the wire grammar: a
    // root-relative value round-trips, and an absolute URL is rejected at
    // JSON decode by the newtype's validating serde bridge (no in-body parse).
    #[test]
    fn publish_result_permalink_wire_is_root_relative() {
        use super::PublishPostResult;
        use common::ids::PostId;
        use common::test_support::{parse_root_relative_url, parse_utc_instant};

        let original = PublishPostResult {
            post_id: PostId::from(1),
            slug: "hello".parse::<Slug>().unwrap(),
            published_at: parse_utc_instant("2026-01-01T00:00:00Z"),
            permalink: parse_root_relative_url("/~alice/2026/01/01/hello"),
        };
        let json = serde_json::to_string(&original).unwrap();
        // A root-relative permalink round-trips over the wire.
        assert_eq!(
            serde_json::from_str::<PublishPostResult>(&json).unwrap(),
            original
        );
        // Swapping the field to an absolute URL is rejected at decode.
        let absolute = json.replace("/~alice/2026/01/01/hello", "https://evil.example/x");
        assert!(serde_json::from_str::<PublishPostResult>(&absolute).is_err());
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

    // #498: the create/update RPC input contracts carry `format` as a typed
    // `PostFormat`, so an out-of-domain token is rejected at JSON wire-decode (the
    // `input = Json` codec) — no in-body parse. Build a valid value, serialize, then
    // corrupt only the format token so the test never hardcodes the full wire shape.
    #[test]
    fn create_post_args_rejects_unknown_format_token() {
        use super::CreatePostArgs;
        use common::render::PostFormat;
        let args = CreatePostArgs {
            body: "hi".into(),
            format: PostFormat::Markdown,
            slug_override: None,
            publish: false,
            publish_at: None,
            tags: None,
            summary: None,
            audience: None,
        };
        let json = serde_json::to_string(&args).unwrap();
        assert!(serde_json::from_str::<CreatePostArgs>(&json).is_ok());
        let bad = json.replace("\"markdown\"", "\"bogus\"");
        assert!(serde_json::from_str::<CreatePostArgs>(&bad).is_err());
    }

    #[test]
    fn update_post_args_rejects_unknown_format_token() {
        use super::UpdatePostArgs;
        use common::ids::PostId;
        use common::render::PostFormat;
        let args = UpdatePostArgs {
            post_id: PostId::from(1),
            body: "hi".into(),
            format: PostFormat::Markdown,
            slug_override: None,
            publish: false,
            publish_at: None,
            tags: None,
            summary: None,
            audience: None,
        };
        let json = serde_json::to_string(&args).unwrap();
        assert!(serde_json::from_str::<UpdatePostArgs>(&json).is_ok());
        let bad = json.replace("\"markdown\"", "\"bogus\"");
        assert!(serde_json::from_str::<UpdatePostArgs>(&bad).is_err());
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
        assert_eq!(
            summary.permalink.as_deref(),
            Some("/~author/2026/04/16/titleless-note")
        );
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
