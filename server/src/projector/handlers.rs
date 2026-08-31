use axum::{
    Router,
    extract::{Extension, Path},
    http::HeaderMap,
    response::Response,
    routing::get,
};
use common::pagination::PageSize;
use common::permalink_route::PermalinkRoute;
use common::seed::PageSeed;
use common::tag::Tag;
use common::time::UtcInstant;
use common::username::Username;
use common::visibility::ViewerIdentity;
use serde::{Deserialize, Deserializer};

use crate::soft_path::SoftPath;
use std::{future::Future, sync::Arc};
use storage::{PostStorage, UserStorage};
use web::error::{self, SwallowedSource};
use web::timeline;

use super::Shell;
use super::document;

/// Register the public projector routes. Generic over the router state because
/// the handlers extract only request `Extension`s (the storage traits + the
/// shell), never `State`, so they compose onto the bare `Router<()>` in
/// `create_router` and in tests alike.
///
/// Only the permalink route lands here for now; the profile / timeline / tag
/// routes arrive with their verticals. Until then those URLs keep hitting the
/// SPA fallback unchanged.
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
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path(PermalinkPath(route)): Path<PermalinkPath>,
) -> Response {
    let Some(route) = route else {
        // A semantically invalid decoded permalink is never public content: let the client
        // resolve it, preserving the projector's uniform shell soft-404.
        return document::shell_response(&shell);
    };
    let result = storage::fetch_post_record(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        &route.username,
        route.date,
        &route.slug,
        UtcInstant::now(),
    )
    .await;
    document::permalink_response(result, &headers, &shell)
}

async fn site_timeline(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    headers: HeaderMap,
) -> Response {
    let result = timeline::fetch_local_timeline(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        None,
        Some(PageSize::default()),
    )
    .await;
    document::timeline_response(result, &headers, PageSeed::SiteTimeline)
}

/// Project a username-keyed public page, or serve the SPA shell when the
/// username is malformed or the route-specific fetch says no anonymous-public
/// content exists. Public projector routes intentionally soft-404 to the shell
/// instead of returning a hard 404: the CSR client must get the same chance to
/// resolve a draft, authenticated owner view, or client-side 404 that it had
/// before the projector existed.
///
/// The fetch closure intentionally owns unknown-user semantics: profile's
/// `fetch_user_posts` returns an empty page that stays cacheable, while
/// user-tag's `fetch_user_posts_by_tag` returns an error that falls back here to
/// the shell.
async fn username_page_response<F, Fut>(
    username: SoftPath<Username>,
    headers: &HeaderMap,
    shell: &Shell,
    context: &'static str,
    fetch_seed: F,
) -> Response
where
    F: FnOnce(Username) -> Fut,
    Fut: Future<Output = web::error::InternalResult<PageSeed>>,
{
    let Some(username) = username.into() else {
        return document::shell_response(shell);
    };
    match fetch_seed(username).await {
        Ok(seed) => document::cacheable(headers, &seed),
        Err(error) => {
            error::report_swallowed(
                error.kind(),
                error.class(),
                context,
                SwallowedSource::Error(&error),
            );
            document::shell_response(shell)
        }
    }
}

async fn profile(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path(username): Path<SoftPath<Username>>,
) -> Response {
    // `username_page_response` documents why the valid-unknown-user result stays
    // route-specific instead of normalized here.
    username_page_response(
        username,
        &headers,
        &shell,
        "server.projector.profile",
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
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    // The tag segment stays `String` (not a typed `Path<Tag>` extractor) so a
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
    document::tag_response(
        result,
        &headers,
        &shell,
        "server.projector.site_tag",
        |page| PageSeed::SiteTag { tag, page },
    )
}

async fn user_tag(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(users): Extension<Arc<dyn UserStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    // The username/tag segments stay `String` (not typed extractors) so a malformed
    // segment is parsed *inside* the handler and falls back to the SPA shell below,
    // rather than a typed extractor's 400 before the handler runs — the deliberate
    // projector-vs-atompub boundary split (ADR-0063 §4). Mirrors `permalink`.
    Path((username, tag)): Path<(SoftPath<Username>, SoftPath<Tag>)>,
) -> Response {
    // `Tag::from_str` lowercases, so the projected heading and the client render
    // coincide. An unparseable username/tag is never public content — serve the
    // shell and let the client route it.
    let Some(tag) = tag.into() else {
        return document::shell_response(&shell);
    };
    username_page_response(
        username,
        &headers,
        &shell,
        "server.projector.user_tag",
        |username| async move {
            timeline::fetch_user_posts_by_tag(
                posts.as_ref(),
                users.as_ref(),
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
