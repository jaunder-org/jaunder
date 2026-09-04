use axum::{
    Router,
    extract::{Extension, Path},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::get,
};
use common::pagination::PageSize;
use common::permalink_route::PermalinkRoute;
use common::seed::{PageSeed, PublicPresentation};
use common::tag::Tag;
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::ViewerIdentity;
use serde::{Deserialize, Deserializer};

use crate::soft_path::SoftPath;
use std::{future::Future, sync::Arc};
use storage::{PostStorage, SiteConfigStorage, UserConfigStorage, UserStorage};
use web::error::{self, SwallowedSource};
use web::timeline;

use super::Shell;
use super::document;

/// Register the public projector routes. Generic over the router state because
/// the handlers extract only request `Extension`s (the storage traits + the
/// shell), never `State`, so they compose onto the bare `Router<()>` in
/// `create_router` and in tests alike.
///
/// The route table covers every cacheable public surface. Private, malformed, and
/// semantically missing public content still falls through to the SPA shell so
/// the client may resolve session-specific state.
pub fn register<S>(router: Router<S>, shell: Shell) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .route("/", get(site_timeline))
        .route("/~{username}", get(profile))
        .route("/~{username}/{year}/{month}/{day}/{slug}", get(permalink))
        .route("/tags/{tag}", get(site_tag))
        .route("/~{username}/tags/{tag}", get(user_tag))
        .layer(Extension(shell))
}

/// A decoded permalink capture set, softly parsed as one all-or-nothing route value.
///
/// The public projector keeps its shell fallback for semantic misses (#697): only decoding or
/// tuple-shape failures are extractor errors. This private adapter applies ADR-0063 §4 at the
/// route boundary, so no raw permalink components enter handler logic.
struct PermalinkPath(Option<PermalinkRoute>);

impl<'de> Deserialize<'de> for PermalinkPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (username, year, month, day, slug) =
            <(String, String, String, String, String)>::deserialize(deserializer)?;
        Ok(Self(PermalinkRoute::parse(
            &username, &year, &month, &day, &slug,
        )))
    }
}

async fn permalink(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(user_config): Extension<Arc<dyn UserConfigStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path(PermalinkPath(route)): Path<PermalinkPath>,
) -> Response {
    let Some(route) = route else {
        // A semantically invalid decoded permalink is never public content: let the client
        // resolve it, preserving the projector's uniform shell soft-404.
        return document::shell_response(&shell);
    };
    let record = match storage::fetch_post_record(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        &route.username,
        route.date,
        &route.slug,
        UtcInstant::now(),
    )
    .await
    {
        Ok(Some(record)) => record,
        Ok(None) => return document::shell_response(&shell),
        Err(error) => {
            error
                .with_context("boundary", "server.projector.permalink")
                .emit_boundary_failure();
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let theme = match storage::resolve_public_theme(
        storage::PublicThemeOwner::Author(record.user_id),
        site_config.as_ref(),
        user_config.as_ref(),
    )
    .await
    {
        Ok(theme) => theme,
        Err(error) => {
            error::InternalError::from(error)
                .with_context("boundary", "server.projector.permalink")
                .emit_boundary_failure();
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    document::permalink_response(Ok(Some(record)), &headers, &shell, theme)
}

async fn site_timeline(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(user_config): Extension<Arc<dyn UserConfigStorage>>,
    headers: HeaderMap,
) -> Response {
    let page = match timeline::fetch_local_timeline(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        None,
        Some(PageSize::default()),
    )
    .await
    {
        Ok(page) => page,
        Err(error) => {
            error
                .with_context("boundary", "server.projector.timeline")
                .emit_boundary_failure();
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let theme = match storage::resolve_public_theme(
        storage::PublicThemeOwner::Site,
        site_config.as_ref(),
        user_config.as_ref(),
    )
    .await
    {
        Ok(theme) => theme,
        Err(error) => {
            web::error::InternalError::from(error)
                .with_context("boundary", "server.projector.timeline_theme")
                .emit_boundary_failure();
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    document::cacheable_presentation(
        &headers,
        &PublicPresentation {
            theme,
            page: PageSeed::SiteTimeline(page),
        },
    )
}

struct ThemeStores<'a> {
    users: &'a dyn UserStorage,
    site_config: &'a dyn SiteConfigStorage,
    user_config: &'a dyn UserConfigStorage,
}

/// Project a username-keyed public page with the route owner's effective theme.
///
/// The route fetch remains authoritative for unknown-user semantics: profiles
/// project an empty page, while user-tag routes soft-fall back to the shell.
async fn username_page_response<F, Fut>(
    username: SoftPath<Username>,
    headers: &HeaderMap,
    shell: &Shell,
    context: &'static str,
    stores: ThemeStores<'_>,
    fetch_seed: F,
) -> Response
where
    F: FnOnce(Username) -> Fut,
    Fut: Future<Output = web::error::InternalResult<PageSeed>>,
{
    let Some(username): Option<Username> = username.into() else {
        return document::shell_response(shell);
    };
    let lookup_username = username.clone();
    let seed = match fetch_seed(username).await {
        Ok(seed) => seed,
        Err(error) => {
            error::report_swallowed(
                error.kind(),
                error.class(),
                context,
                SwallowedSource::Error(&error),
            );
            return document::shell_response(shell);
        }
    };
    let owner = match stores.users.get_user_by_username(&lookup_username).await {
        Ok(Some(author)) => storage::PublicThemeOwner::Author(author.user_id),
        Ok(None) => storage::PublicThemeOwner::Site,
        Err(error) => {
            error::InternalError::from(error)
                .with_context("boundary", context)
                .emit_boundary_failure();
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let theme =
        match storage::resolve_public_theme(owner, stores.site_config, stores.user_config).await {
            Ok(theme) => theme,
            Err(error) => {
                error::InternalError::from(error)
                    .with_context("boundary", context)
                    .emit_boundary_failure();
                return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    document::cacheable_presentation(headers, &PublicPresentation { theme, page: seed })
}

async fn profile(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(users): Extension<Arc<dyn UserStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(user_config): Extension<Arc<dyn UserConfigStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path(username): Path<SoftPath<Username>>,
) -> Response {
    username_page_response(
        username,
        &headers,
        &shell,
        "server.projector.profile",
        ThemeStores {
            users: users.as_ref(),
            site_config: site_config.as_ref(),
            user_config: user_config.as_ref(),
        },
        |username| async move {
            timeline::fetch_user_posts(
                posts.as_ref(),
                &ViewerIdentity::Anonymous,
                &username,
                None,
                Some(PageSize::default()),
            )
            .await
            .map(|page| PageSeed::Profile { username, page })
        },
    )
    .await
}

async fn site_tag(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(user_config): Extension<Arc<dyn UserConfigStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    // malformed segment is parsed *inside* the handler and falls back to the SPA
    // shell (client-rendered 404) below — a typed extractor would reject it with a
    // 400 *before* the handler runs. This is the deliberate projector-vs-atompub
    // boundary split (ADR-0063 §4): atompub handlers are typed (400-on-malformed
    // API); the public projector serves the shell. Mirrors the `permalink` handler.
    Path(tag): Path<SoftPath<Tag>>,
) -> Response {
    // `Tag::from_str` lowercases, so the projected heading and the client render
    // coincide. An unparseable tag is never public content — let the client route it.
    let Some(tag) = tag.into() else {
        return document::shell_response(&shell);
    };
    let result = timeline::fetch_posts_by_tag(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        &tag,
        None,
        Some(PageSize::default()),
    )
    .await;
    match result {
        Ok(page) => {
            let theme = match storage::resolve_public_theme(
                storage::PublicThemeOwner::Site,
                site_config.as_ref(),
                user_config.as_ref(),
            )
            .await
            {
                Ok(theme) => theme,
                Err(error) => {
                    error::InternalError::from(error)
                        .with_context("boundary", "server.projector.site_tag")
                        .emit_boundary_failure();
                    return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };
            document::cacheable_presentation(
                &headers,
                &PublicPresentation {
                    theme,
                    page: PageSeed::SiteTag { tag, page },
                },
            )
        }
        Err(error) => {
            error::report_swallowed(
                error.kind(),
                error.class(),
                "server.projector.site_tag",
                SwallowedSource::Error(&error),
            );
            document::shell_response(&shell)
        }
    }
}

async fn user_tag(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(users): Extension<Arc<dyn UserStorage>>,
    Extension(site_config): Extension<Arc<dyn SiteConfigStorage>>,
    Extension(user_config): Extension<Arc<dyn UserConfigStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path((username, tag)): Path<(SoftPath<Username>, SoftPath<Tag>)>,
) -> Response {
    let Some(tag) = tag.into() else {
        return document::shell_response(&shell);
    };
    let fetch_users = Arc::clone(&users);
    username_page_response(
        username,
        &headers,
        &shell,
        "server.projector.user_tag",
        ThemeStores {
            users: users.as_ref(),
            site_config: site_config.as_ref(),
            user_config: user_config.as_ref(),
        },
        |username| async move {
            timeline::fetch_user_posts_by_tag(
                posts.as_ref(),
                fetch_users.as_ref(),
                &ViewerIdentity::Anonymous,
                &username,
                &tag,
                None,
                Some(PageSize::default()),
            )
            .await
            .map(|page| PageSeed::UserTag {
                username,
                tag,
                page,
            })
        },
    )
    .await
}
