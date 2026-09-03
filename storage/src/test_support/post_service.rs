//! Service-layer post creation and update fixtures. These helpers exercise production
//! rendering/extraction paths; raw storage-layer post inputs belong in [`super::posts`].

use super::{confirmed_for, fixture_media_content_locks};
use crate::{AppState, PostBookkeepingExpectation, PostFormat};

use common::ids::{PostId, UserId};
use common::post_body::PostBody;
use common::time::UtcInstant;
use common::visibility::AudienceTarget;
use std::sync::Arc;

/// Creates a post through [`perform_post_creation`](crate::perform_post_creation) —
/// the same entry point `web::posts::create` uses — so a test exercises the product's
/// own path (render, extract, write) rather than a synthetic [`CreatePostInput`](crate::CreatePostInput).
///
/// # Panics
///
/// If the post cannot be created.
pub async fn create_post_via_service(
    state: &Arc<AppState>,
    user_id: UserId,
    body: PostBody,
) -> PostId {
    create_via_service(state, user_id, body, Some(UtcInstant::now())).await
}

/// The unpublished twin of [`create_post_via_service`] — the draft a publication test
/// needs, created the same way.
///
/// # Panics
///
/// If the post cannot be created.
pub async fn create_draft_via_service(
    state: &Arc<AppState>,
    user_id: UserId,
    body: PostBody,
) -> PostId {
    create_via_service(state, user_id, body, None).await
}

/// Shared body of the two service-layer creators: everything but `published_at` is
/// fixed (public, Markdown, title derived from the body), as the two differ in exactly
/// that one field.
async fn create_via_service(
    state: &Arc<AppState>,
    user_id: UserId,
    body: PostBody,
    published_at: Option<UtcInstant>,
) -> PostId {
    let outcome = crate::perform_post_creation(
        &state.write_scope,
        &fixture_media_content_locks(),
        Arc::clone(&state.posts),
        Arc::clone(&state.feed_events),
        crate::PostCreation {
            user_id,
            body,
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            published_at,
            max_attempts: 100,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            tags: Vec::new(),
            idempotency_key: None,
            expectations: PostBookkeepingExpectation::default(),
        },
    )
    .await
    .expect("post creation via the service path should succeed");
    confirmed_for(outcome, "post creation fixture").post_id
}
/// Edits a post's body through [`perform_post_update`](crate::perform_post_update) —
/// the service-layer twin of [`create_post_via_service`], so an edit's re-render and
/// re-extraction run exactly as the product runs them. Publication state is left
/// as-is.
///
/// # Panics
///
/// If the update fails.
pub async fn update_post_body_via_service(
    state: &Arc<AppState>,
    post_id: PostId,
    editor_user_id: UserId,
    body: PostBody,
) {
    let outcome = crate::perform_post_update(
        &state.write_scope,
        &fixture_media_content_locks(),
        Arc::clone(&state.posts),
        Arc::clone(&state.feed_events),
        crate::PostUpdate {
            post_id,
            editor_user_id,
            body,
            title: None,
            format: PostFormat::Markdown,
            slug_override: None,
            publish: crate::PublishUpdate::Publish { at: None },
            summary: None,
            request_clock: UtcInstant::now(),
            expectations: PostBookkeepingExpectation::default(),
            audiences: vec![AudienceTarget::Public],
            tags: Vec::new(),
        },
    )
    .await
    .expect("post update via the service path should succeed");
    confirmed_for(outcome, "post update fixture");
}
