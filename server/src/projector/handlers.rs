use axum::{
    Router,
    extract::{Extension, OriginalUri, Path},
    http::HeaderMap,
    response::Response,
    routing::get,
};
use common::tag::Tag;
use common::username::Username;
use common::{permalink_route::PermalinkRoute, slug::Slug, time::PermalinkDate};
use serde::{Deserialize, Deserializer};

use crate::soft_path::SoftPath;

use super::{PublicProjector, public_projector::PublicProjection};

/// Register the public projector routes. Generic over the router state because
/// the handlers extract only the owned projector and request inputs, never
/// `State`, so they compose onto the bare `Router<()>` in `create_router` and in
/// tests alike.
///
/// The route table covers every cacheable public surface. Private, malformed, and
/// semantically missing public content still falls through to the SPA shell so
/// the client may resolve session-specific state.
pub fn register<S>(router: Router<S>, projector: PublicProjector) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router
        .route("/", get(site_timeline))
        .route("/~{username}", get(profile))
        .route("/~{username}/{year}/{month}/{day}/{slug}", get(permalink))
        .route("/{year}/{month}/{day}/{slug}", get(permalink_alias))
        .route("/tags/{tag}", get(site_tag))
        .route("/~{username}/tags/{tag}", get(user_tag))
        .layer(Extension(projector))
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

/// A decoded User-omitting permalink path, parsed as one soft route value.
struct PermalinkAliasPath(Option<(PermalinkDate, Slug)>);

impl<'de> Deserialize<'de> for PermalinkAliasPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (year, month, day, slug) =
            <(String, String, String, String)>::deserialize(deserializer)?;
        let year = year.parse().ok();
        let month = month.parse().ok();
        let day = day.parse().ok();
        let route = year
            .zip(month)
            .zip(day)
            .and_then(|((year, month), day)| PermalinkDate::from_ymd(year, month, day))
            .zip(slug.parse().ok());
        Ok(Self(route))
    }
}

async fn permalink_alias(
    Extension(projector): Extension<PublicProjector>,
    OriginalUri(uri): OriginalUri,
    Path(PermalinkAliasPath(route)): Path<PermalinkAliasPath>,
) -> Response {
    let Some((date, slug)) = route else {
        return projector.shell_response();
    };
    projector
        .resolve_permalink_alias(date, slug, uri.query())
        .await
}

async fn permalink(
    Extension(projector): Extension<PublicProjector>,
    headers: HeaderMap,
    Path(PermalinkPath(route)): Path<PermalinkPath>,
) -> Response {
    let Some(route) = route else {
        // A semantically invalid decoded permalink is never public content: let the client
        // resolve it, preserving the projector's uniform shell soft-404.
        return projector.shell_response();
    };
    projector
        .project(PublicProjection::Permalink(route), &headers)
        .await
}

async fn site_timeline(
    Extension(projector): Extension<PublicProjector>,
    headers: HeaderMap,
) -> Response {
    projector
        .project(PublicProjection::SiteTimeline, &headers)
        .await
}

async fn profile(
    Extension(projector): Extension<PublicProjector>,
    headers: HeaderMap,
    Path(username): Path<SoftPath<Username>>,
) -> Response {
    let Some(username) = username.into() else {
        return projector.shell_response();
    };
    projector
        .project(PublicProjection::Profile(username), &headers)
        .await
}

async fn site_tag(
    Extension(projector): Extension<PublicProjector>,
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
        return projector.shell_response();
    };
    projector
        .project(PublicProjection::SiteTag(tag), &headers)
        .await
}

async fn user_tag(
    Extension(projector): Extension<PublicProjector>,
    headers: HeaderMap,
    Path((username, tag)): Path<(SoftPath<Username>, SoftPath<Tag>)>,
) -> Response {
    let (Some(username), Some(tag)) = (username.into(), tag.into()) else {
        return projector.shell_response();
    };
    projector
        .project(PublicProjection::UserTag { username, tag }, &headers)
        .await
}
