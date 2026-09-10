//! The timeline vertical's `#[server]` endpoints: cursor-paginated public
//! listings return
//! [`PublicPresentation<Page<RenderedPost, TimelineCursor>>`](PublicPresentation)
//! so a client-side destination commits both its data and server-resolved theme.
//! The authenticated home feed remains a private `Page` response.
//!
//! The wire types they exchange are defined in `common::seed`; the host-only query
//! helpers these bodies call live in the [`super::server`] leaf. `timeline/mod.rs` is
//! wiring only and re-exports these under the stable `crate::timeline::…` paths that
//! call sites and the server-fn registrar depend on.
//!
//! Every endpoint carries the cohesive [`TimelinePageRequest`]. Its nested cursor
//! binds the continuation to its publication-time direction; JSON input is required
//! because the default form-urlencoded codec cannot carry nested structs.

use crate::error::WebResult;
#[cfg(feature = "server")]
use crate::error::{InternalError, InternalResult};
use common::seed::{Page, PublicPresentation, RenderedPost, TimelinePageRequest};
use common::{tag::Tag, username::Username};
use leptos::server_fn::codec::Json;

// Server-only imports for the `#[server]` fn bodies (gated on `feature = "server"`).
#[cfg(feature = "server")]
use {
    super::server,
    crate::{auth, viewer},
    common::time::UtcInstant,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{self, PostStorage, ThemeStorage, UserStorage},
};

#[cfg(feature = "server")]
async fn site_presentation(
    route: common::theme::PublicThemeRoute,
    page: Page<RenderedPost, common::seed::TimelineCursor>,
) -> InternalResult<PublicPresentation<Page<RenderedPost, common::seed::TimelineCursor>>> {
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let theme =
        storage::resolve_public_theme(storage::PublicThemeOwner::Site, &route, themes.as_ref())
            .await?;
    Ok(PublicPresentation { theme, page })
}

#[cfg(feature = "server")]
async fn author_presentation(
    username: &Username,
    route: common::theme::PublicThemeRoute,
    page: Page<RenderedPost, common::seed::TimelineCursor>,
) -> InternalResult<PublicPresentation<Page<RenderedPost, common::seed::TimelineCursor>>> {
    let users = expect_context::<Arc<dyn UserStorage>>();
    let themes = expect_context::<Arc<dyn ThemeStorage>>();
    let owner = users
        .get_user_by_username(username)
        .await?
        .map_or(storage::PublicThemeOwner::Site, |author| {
            storage::PublicThemeOwner::Author(author.user_id)
        });
    let theme = storage::resolve_public_theme(owner, &route, themes.as_ref()).await?;
    Ok(PublicPresentation { theme, page })
}

/// Lists published, non-deleted posts for a user using cursor pagination.
#[macros::server(input = Json)]
pub async fn list_by_user(
    username: Username,
    request: TimelinePageRequest,
) -> WebResult<PublicPresentation<Page<RenderedPost, common::seed::TimelineCursor>>> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer::viewer_identity().await?;
    let page = server::fetch_user_posts(
        posts.as_ref(),
        &viewer,
        &username,
        storage::timeline_keyset_cursor(request.cursor),
        request.order,
        request.limit,
    )
    .await?;
    author_presentation(
        &username,
        common::theme::PublicThemeRoute::author(&username),
        page,
    )
    .await
}

#[macros::server(input = Json)]
/// Lists published, non-deleted posts across all users using cursor pagination.
pub async fn list_local_timeline(
    request: TimelinePageRequest,
) -> WebResult<PublicPresentation<Page<RenderedPost, common::seed::TimelineCursor>>> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer::viewer_identity().await?;
    let page = server::fetch_local_timeline(
        posts.as_ref(),
        &viewer,
        storage::timeline_keyset_cursor(request.cursor),
        request.order,
        request.limit,
    )
    .await?;
    site_presentation(common::theme::PublicThemeRoute::site(), page).await
}

/// Lists published, non-deleted posts by the authenticated user using cursor pagination.
#[macros::server(input = Json)]
pub async fn list_home_feed(
    request: TimelinePageRequest,
) -> WebResult<Page<RenderedPost, common::seed::TimelineCursor>> {
    let auth = auth::require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let cursor = storage::timeline_keyset_cursor(request.cursor);
    if cursor
        .as_ref()
        .is_some_and(|cursor| cursor.order != request.order)
    {
        return Err(InternalError::validation("timeline cursor order mismatch"));
    }
    let viewer = viewer::viewer_identity().await?;
    let page_size = request.limit.unwrap_or_default();
    let rows = posts
        .list_published_by_user(
            &auth.username,
            storage::PublishedPageRequest {
                cursor: cursor.as_ref(),
                order: request.order,
                limit: page_size.fetch_limit(),
            },
            &viewer,
            UtcInstant::now(),
        )
        .await?;
    server::page_from_rows(rows, page_size, Some(auth.user_id), request.order)
}

/// Lists published, non-deleted posts site-wide carrying `tag`.
#[macros::server(input = Json)]
pub async fn list_by_tag(
    tag: Tag,
    request: TimelinePageRequest,
) -> WebResult<PublicPresentation<Page<RenderedPost, common::seed::TimelineCursor>>> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer::viewer_identity().await?;
    let page = server::fetch_posts_by_tag(
        posts.as_ref(),
        &viewer,
        &tag,
        storage::timeline_keyset_cursor(request.cursor),
        request.order,
        request.limit,
    )
    .await?;
    site_presentation(common::theme::PublicThemeRoute::site_tag(&tag), page).await
}

/// Lists published, non-deleted posts by `username` carrying `tag`.
#[macros::server(input = Json)]
pub async fn list_by_user_and_tag(
    username: Username,
    tag: Tag,
    request: TimelinePageRequest,
) -> WebResult<PublicPresentation<Page<RenderedPost, common::seed::TimelineCursor>>> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let viewer = viewer::viewer_identity().await?;
    let page = server::fetch_user_posts_by_tag(
        posts.as_ref(),
        users.as_ref(),
        &viewer,
        &username,
        &tag,
        request,
    )
    .await?;
    author_presentation(
        &username,
        common::theme::PublicThemeRoute::author_tag(&username, &tag),
        page,
    )
    .await
}
