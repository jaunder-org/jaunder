use axum::{
    Router,
    extract::{Extension, Path},
    http::HeaderMap,
    response::Response,
    routing::get,
};
use common::pagination::PageSize;
use common::seed::PageSeed;
use common::slug::Slug;
use common::tag::Tag;
use common::time::PermalinkDate;
use common::username::Username;
use common::visibility::ViewerIdentity;

use crate::soft_path::SoftPath;
use std::sync::Arc;
use storage::{PostStorage, UserStorage, fetch_post_record};
use web::timeline::{
    fetch_local_timeline, fetch_posts_by_tag, fetch_user_posts, fetch_user_posts_by_tag,
};

use super::Shell;
use super::document::{
    cacheable, permalink_response, shell_response, tag_response, timeline_response,
};

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

/// The permalink route's five path segments: soft-parsed `username`/`slug` around the numeric
/// `year`/`month`/`day`. A `type` alias to keep the `Path<…>` under clippy's type-complexity
/// threshold. `SoftPath` gives a malformed `username`/`slug` the SPA shell (client-rendered
/// 404) rather than axum's pre-handler 400 — the projector-vs-atompub boundary split (ADR-0063
/// §4): atompub handlers are strictly typed (400-on-malformed API); the public projector
/// serves the shell.
type PermalinkPath = (
    SoftPath<Username>,
    SoftPath<i32>,
    SoftPath<u32>,
    SoftPath<u32>,
    SoftPath<Slug>,
);

async fn permalink(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path((username, year, month, day, slug)): Path<PermalinkPath>,
) -> Response {
    // The three `SoftPath` date segments are already `Option`s (soft-deserialized);
    // present + a real date, else `None` → the shell (soft-404) below.
    let date = Option::<i32>::from(year)
        .zip(Option::<u32>::from(month))
        .zip(Option::<u32>::from(day))
        .and_then(|((y, m), d)| PermalinkDate::from_ymd(y, m, d));
    let (Some(username), Some(date), Some(slug)) = (username.into(), date, slug.into()) else {
        // An unparseable segment — or an impossible date (e.g. month 13) — is never
        // public content: let the client route it (it may be a server URL the SPA
        // reloads for), a uniform soft-404 (#583).
        return shell_response(&shell);
    };
    let result = fetch_post_record(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        &username,
        date,
        &slug,
        chrono::Utc::now(),
    )
    .await;
    permalink_response(result, &headers, &shell)
}

async fn site_timeline(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    headers: HeaderMap,
) -> Response {
    let result = fetch_local_timeline(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        None,
        Some(PageSize::default()),
    )
    .await;
    timeline_response(result, &headers, PageSeed::SiteTimeline)
}

async fn profile(
    Extension(posts): Extension<Arc<dyn PostStorage>>,
    Extension(shell): Extension<Shell>,
    headers: HeaderMap,
    Path(username): Path<SoftPath<Username>>,
) -> Response {
    // An unparseable username or a storage error is never public content — serve the
    // shell and let the client route it. A valid username (even an unknown one, which
    // yields an empty profile) is cached like any other public page.
    let seed = match username.into() {
        Some(username) => fetch_user_posts(
            posts.as_ref(),
            &ViewerIdentity::Anonymous,
            &username,
            None,
            Some(PageSize::default()),
        )
        .await
        .ok()
        .map(|page| PageSeed::Profile { username, page }),
        None => None,
    };
    match seed {
        Some(seed) => cacheable(&headers, &seed),
        None => shell_response(&shell),
    }
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
        return shell_response(&shell);
    };
    let result = fetch_posts_by_tag(
        posts.as_ref(),
        &ViewerIdentity::Anonymous,
        &tag,
        None,
        Some(PageSize::default()),
    )
    .await;
    tag_response(result, &headers, &shell, |page| PageSeed::SiteTag {
        tag,
        page,
    })
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
    let (Some(username), Some(tag)) = (username.into(), tag.into()) else {
        return shell_response(&shell);
    };
    // An unknown user or a storage error is likewise never public content.
    let result = fetch_user_posts_by_tag(
        posts.as_ref(),
        users.as_ref(),
        &ViewerIdentity::Anonymous,
        &username,
        &tag,
        None,
        Some(PageSize::default()),
    )
    .await;
    tag_response(result, &headers, &shell, |page| PageSeed::UserTag {
        username,
        tag,
        page,
    })
}
