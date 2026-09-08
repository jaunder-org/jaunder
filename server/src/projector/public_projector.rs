use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use common::{
    pagination::PageSize,
    permalink_route::PermalinkRoute,
    seed::{PageSeed, PublicPresentation},
    theme::PublicThemeRoute,
    time::UtcInstant,
    visibility::ViewerIdentity,
};
use std::sync::Arc;
use storage::{PostStorage, ThemeStorage};
use web::{error::InternalError, posts, timeline};

use super::{Shell, document};

/// The public routes currently served by the projector.
///
/// Route handlers decode their soft path segments before selecting an operation,
/// preserving their SPA-shell behavior for malformed paths.
pub(crate) enum PublicProjection {
    Permalink(PermalinkRoute),
    SiteTimeline,
}

/// Projects anonymous public routes into their final HTTP responses.
pub(crate) struct PublicProjector {
    posts: Arc<dyn PostStorage>,
    themes: Arc<dyn ThemeStorage>,
    shell: Shell,
}

impl PublicProjector {
    #[must_use]
    pub(crate) fn new(
        posts: Arc<dyn PostStorage>,
        themes: Arc<dyn ThemeStorage>,
        shell: Shell,
    ) -> Self {
        Self {
            posts,
            themes,
            shell,
        }
    }

    pub(crate) async fn project(
        &self,
        operation: PublicProjection,
        headers: &HeaderMap,
    ) -> Response {
        let outcome = self.execute(operation).await;
        self.response_for(&outcome, headers)
    }

    async fn execute(&self, operation: PublicProjection) -> ProjectionResult {
        match operation {
            PublicProjection::Permalink(route) => self.permalink(route).await,
            PublicProjection::SiteTimeline => self.site_timeline().await,
        }
    }

    fn response_for(&self, outcome: &ProjectionResult, headers: &HeaderMap) -> Response {
        match outcome {
            Ok(presentation) => document::cacheable_presentation(headers, presentation),
            Err(ProjectionFailure::Shell) => document::shell_response(&self.shell),
            Err(ProjectionFailure::Boundary(error)) => {
                error.emit_boundary_failure();
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }

    async fn permalink(&self, route: PermalinkRoute) -> ProjectionResult {
        let record = match storage::fetch_post_record(
            self.posts.as_ref(),
            &ViewerIdentity::Anonymous,
            &route.username,
            route.date,
            &route.slug,
            UtcInstant::now(),
        )
        .await
        {
            Ok(Some(record)) => record,
            Ok(None) => return Err(ProjectionFailure::Shell),
            Err(error) => return Err(Self::boundary(error, "server.projector.permalink")),
        };
        let theme = match storage::resolve_public_theme(
            storage::PublicThemeOwner::Author(record.user_id),
            &PublicThemeRoute::permalink(&route),
            self.themes.as_ref(),
        )
        .await
        {
            Ok(theme) => theme,
            Err(error) => return Err(Self::boundary(error, "server.projector.permalink")),
        };
        Ok(PublicPresentation {
            theme,
            page: PageSeed::Permalink(posts::authored_post(record, false)),
        })
    }

    async fn site_timeline(&self) -> ProjectionResult {
        let page = match timeline::fetch_local_timeline(
            self.posts.as_ref(),
            &ViewerIdentity::Anonymous,
            None,
            Some(PageSize::default()),
        )
        .await
        {
            Ok(page) => page,
            Err(error) => return Err(Self::boundary(error, "server.projector.timeline")),
        };
        let theme = match storage::resolve_public_theme(
            storage::PublicThemeOwner::Site,
            &PublicThemeRoute::site(),
            self.themes.as_ref(),
        )
        .await
        {
            Ok(theme) => theme,
            Err(error) => return Err(Self::boundary(error, "server.projector.timeline_theme")),
        };
        Ok(PublicPresentation {
            theme,
            page: PageSeed::SiteTimeline(page),
        })
    }

    fn boundary(error: impl Into<InternalError>, context: &'static str) -> ProjectionFailure {
        ProjectionFailure::Boundary(error.into().with_context("boundary", context))
    }
}

/// The internal route result before its single final HTTP mapping.
type ProjectionResult = Result<PublicPresentation<PageSeed>, ProjectionFailure>;

enum ProjectionFailure {
    Shell,
    Boundary(InternalError),
}
