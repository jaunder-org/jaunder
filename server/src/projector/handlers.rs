use axum::{
    Router,
    extract::{Extension, OriginalUri, Path, Query},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
};
use common::seed::TimelineOrder;
use common::tag::Tag;
use common::username::Username;
use common::{permalink_route::PermalinkRoute, slug::Slug, time::PermalinkDate};
use percent_encoding::percent_decode_str;
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
        .route(
            "/{year}/{month}/{day}/{slug}",
            get(permalink_alias).head(permalink_alias_head),
        )
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

/// URL parser shared by every projected web timeline. Unknown tokens deliberately
/// select the canonical Newest representation rather than failing a public page.
#[derive(Deserialize)]
struct TimelineQuery {
    order: Option<String>,
}

impl TimelineQuery {
    fn order(self) -> TimelineOrder {
        self.order
            .and_then(|order| order.parse().ok())
            .unwrap_or_default()
    }
}

impl<'de> Deserialize<'de> for PermalinkPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (username, year, month, day, slug) =
            <(String, String, String, String, String)>::deserialize(deserializer)?;
        Ok(Self(PermalinkRoute::parse(
            &username, &year, &month, &day, &slug,
        )))
    }
}

/// Parse the router-matched four raw alias segments so invalid percent-decoded UTF-8
/// reaches the projector's indistinguishable shell miss rather than axum's extractor
/// rejection.
fn parse_permalink_alias_path(path: &str) -> Option<(PermalinkDate, Slug)> {
    let mut segments = path.strip_prefix('/')?.split('/');
    let year = percent_decode_str(segments.next()?).decode_utf8().ok()?;
    let month = percent_decode_str(segments.next()?).decode_utf8().ok()?;
    let day = percent_decode_str(segments.next()?).decode_utf8().ok()?;
    let slug = percent_decode_str(segments.next()?).decode_utf8().ok()?;

    if year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || !year.bytes().all(|byte| byte.is_ascii_digit())
        || !month.bytes().all(|byte| byte.is_ascii_digit())
        || !day.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let date = PermalinkDate::from_ymd(year.parse().ok()?, month.parse().ok()?, day.parse().ok()?)?;
    Some((date, slug.parse().ok()?))
}

async fn permalink_alias_head() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

async fn permalink_alias(
    Extension(projector): Extension<PublicProjector>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let Some((date, slug)) = parse_permalink_alias_path(uri.path()) else {
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
    Query(query): Query<TimelineQuery>,
) -> Response {
    projector
        .project(PublicProjection::SiteTimeline(query.order()), &headers)
        .await
}

async fn profile(
    Extension(projector): Extension<PublicProjector>,
    headers: HeaderMap,
    Path(username): Path<SoftPath<Username>>,
    Query(query): Query<TimelineQuery>,
) -> Response {
    let Some(username) = username.into() else {
        return projector.shell_response();
    };
    projector
        .project(PublicProjection::Profile(username, query.order()), &headers)
        .await
}

async fn site_tag(
    Extension(projector): Extension<PublicProjector>,
    headers: HeaderMap,
    Path(tag): Path<SoftPath<Tag>>,
    Query(query): Query<TimelineQuery>,
) -> Response {
    let Some(tag) = tag.into() else {
        return projector.shell_response();
    };
    projector
        .project(PublicProjection::SiteTag(tag, query.order()), &headers)
        .await
}

async fn user_tag(
    Extension(projector): Extension<PublicProjector>,
    headers: HeaderMap,
    Path((username, tag)): Path<(SoftPath<Username>, SoftPath<Tag>)>,
    Query(query): Query<TimelineQuery>,
) -> Response {
    let (Some(username), Some(tag)) = (username.into(), tag.into()) else {
        return projector.shell_response();
    };
    projector
        .project(
            PublicProjection::UserTag {
                username,
                tag,
                order: query.order(),
            },
            &headers,
        )
        .await
}
