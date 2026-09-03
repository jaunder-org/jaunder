//! The timeline vertical's `#[server]` endpoints: cursor-paginated public
//! listings return [`PublicPresentation<Page<RenderedPost>>`](PublicPresentation)
//! so a client-side destination commits both its data and server-resolved theme.
//! The authenticated home feed remains a private `Page` response.
//!
//! The wire types they exchange are defined in `common::seed`; the host-only query
//! helpers these bodies call live in the [`super::server`] leaf. `timeline/mod.rs` is
//! wiring only and re-exports these under the stable `crate::timeline::…` paths that
//! call sites and the server-fn registrar depend on.
//!
//! Every endpoint here takes the keyset cursor as one bundled [`PageCursor`]
//! rather than a pair of `Option`s, so a half cursor is rejected at arg-decode
//! and no body has one to validate. A nested struct cannot travel through the
//! default form-urlencoded codec, which is why each carries `input = Json` —
//! required, not stylistic.

#[cfg(feature = "server")]
use crate::error::InternalResult;
use crate::error::WebResult;
use common::seed::{Page, PageCursor, PublicPresentation, RenderedPost};
use common::{pagination::PageSize, tag::Tag, username::Username};
use leptos::server_fn::codec::Json;

// Server-only imports for the `#[server]` fn bodies (gated on `feature = "server"`).
#[cfg(feature = "server")]
use {
    super::server,
    crate::{auth, viewer},
    common::time::UtcInstant,
    leptos::prelude::*,
    std::sync::Arc,
    storage::{self, PostStorage, SiteConfigStorage, UserConfigStorage, UserStorage},
};

#[cfg(feature = "server")]
async fn site_presentation(
    page: Page<RenderedPost>,
) -> InternalResult<PublicPresentation<Page<RenderedPost>>> {
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let user_config = expect_context::<Arc<dyn UserConfigStorage>>();
    let theme = storage::resolve_public_theme(
        storage::PublicThemeOwner::Site,
        site_config.as_ref(),
        user_config.as_ref(),
    )
    .await?;
    Ok(PublicPresentation { theme, page })
}

#[cfg(feature = "server")]
async fn author_presentation(
    username: &Username,
    page: Page<RenderedPost>,
) -> InternalResult<PublicPresentation<Page<RenderedPost>>> {
    let users = expect_context::<Arc<dyn UserStorage>>();
    let site_config = expect_context::<Arc<dyn SiteConfigStorage>>();
    let user_config = expect_context::<Arc<dyn UserConfigStorage>>();
    let owner = users
        .get_user_by_username(username)
        .await?
        .map_or(storage::PublicThemeOwner::Site, |author| {
            storage::PublicThemeOwner::Author(author.user_id)
        });
    let theme =
        storage::resolve_public_theme(owner, site_config.as_ref(), user_config.as_ref()).await?;
    Ok(PublicPresentation { theme, page })
}

/// Lists published, non-deleted posts for a user using cursor pagination.
#[macros::server(input = Json)]
pub async fn list_by_user(
    username: Username,
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<PublicPresentation<Page<RenderedPost>>> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer::viewer_identity().await?;
    let page = server::fetch_user_posts(
        posts.as_ref(),
        &viewer,
        &username,
        storage::keyset_cursor(cursor),
        limit,
    )
    .await?;
    author_presentation(&username, page).await
}

#[macros::server(input = Json)]
/// Lists published, non-deleted posts across all users using cursor pagination.
pub async fn list_local_timeline(
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<PublicPresentation<Page<RenderedPost>>> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer::viewer_identity().await?;
    let page = server::fetch_local_timeline(
        posts.as_ref(),
        &viewer,
        storage::keyset_cursor(cursor),
        limit,
    )
    .await?;
    site_presentation(page).await
}

/// Lists published, non-deleted posts by the authenticated user using cursor pagination.
#[macros::server(input = Json)]
pub async fn list_home_feed(
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<Page<RenderedPost>> {
    let auth = auth::require_auth().await?;
    let posts = expect_context::<Arc<dyn PostStorage>>();

    let cursor = storage::keyset_cursor(cursor);
    let viewer = viewer::viewer_identity().await?;
    let page_size = limit.unwrap_or_default();

    let rows = posts
        .list_published_by_user(
            &auth.username,
            cursor.as_ref(),
            page_size.fetch_limit(),
            &viewer,
            UtcInstant::now(),
        )
        .await?;

    // Via the shared `page_from_rows`, so the has-more rule is spelled once (#696).
    Ok(server::page_from_rows(rows, page_size, Some(auth.user_id)))
}

/// Lists published, non-deleted posts site-wide carrying `tag`.
#[macros::server(input = Json)]
pub async fn list_by_tag(
    tag: Tag,
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<PublicPresentation<Page<RenderedPost>>> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let viewer = viewer::viewer_identity().await?;
    let page = server::fetch_posts_by_tag(
        posts.as_ref(),
        &viewer,
        &tag,
        storage::keyset_cursor(cursor),
        limit,
    )
    .await?;
    site_presentation(page).await
}

/// Lists published, non-deleted posts by `username` carrying `tag`.
#[macros::server(input = Json)]
pub async fn list_by_user_and_tag(
    username: Username,
    tag: Tag,
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<PublicPresentation<Page<RenderedPost>>> {
    let posts = expect_context::<Arc<dyn PostStorage>>();
    let users = expect_context::<Arc<dyn UserStorage>>();
    let viewer = viewer::viewer_identity().await?;
    let page = server::fetch_user_posts_by_tag(
        posts.as_ref(),
        users.as_ref(),
        &viewer,
        &username,
        &tag,
        storage::keyset_cursor(cursor),
        limit,
    )
    .await?;
    author_presentation(&username, page).await
}
