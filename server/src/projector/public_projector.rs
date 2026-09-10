use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use common::{
    pagination::PageSize,
    permalink_route::PermalinkRoute,
    seed::{PageSeed, PublicPresentation},
    slug::Slug,
    tag::Tag,
    theme::{PublicThemeRoute, PublishedThemePresentation},
    time::{PermalinkDate, UtcInstant},
    username::Username,
    visibility::ViewerIdentity,
};
use std::sync::Arc;
use storage::{PostPermalinkAliasMatch, PostStorage, PublicThemeOwner, ThemeStorage, UserStorage};
use web::{
    error::{self, InternalError, SwallowedSource},
    posts, timeline,
};

use super::{Shell, document};

/// The public routes served by the projector.
///
/// Route handlers decode their soft path segments before selecting an operation,
/// preserving their SPA-shell behavior for malformed paths.
pub(crate) enum PublicProjection {
    Permalink(PermalinkRoute),
    SiteTimeline,
    Profile(Username),
    SiteTag(Tag),
    UserTag { username: Username, tag: Tag },
}

/// Projects anonymous public routes into their final HTTP responses.
#[derive(Clone)]
pub struct PublicProjector {
    posts: Arc<dyn PostStorage>,
    users: Arc<dyn UserStorage>,
    themes: Arc<dyn ThemeStorage>,
    shell: Shell,
}

impl PublicProjector {
    /// Creates a projector from its exact storage and shell dependencies.
    #[must_use]
    pub fn new(
        posts: Arc<dyn PostStorage>,
        users: Arc<dyn UserStorage>,
        themes: Arc<dyn ThemeStorage>,
        shell: Shell,
    ) -> Self {
        Self {
            posts,
            users,
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
            PublicProjection::Profile(username) => self.profile(username).await,
            PublicProjection::SiteTag(tag) => self.site_tag(tag).await,
            PublicProjection::UserTag { username, tag } => self.user_tag(username, tag).await,
        }
    }

    fn response_for(&self, outcome: &ProjectionResult, headers: &HeaderMap) -> Response {
        match outcome {
            Ok(presentation) => document::cacheable_presentation(headers, presentation),
            Err(ProjectionFailure::ShellFallback) => document::shell_response(&self.shell),
            Err(ProjectionFailure::SwallowedFailure { error, context }) => {
                error::report_swallowed(
                    error.kind(),
                    error.class(),
                    context,
                    SwallowedSource::Error(error),
                );
                document::shell_response(&self.shell)
            }
            Err(ProjectionFailure::BoundaryFailure(error)) => {
                error.emit_boundary_failure();
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }

    pub(crate) fn shell_response(&self) -> Response {
        document::shell_response(&self.shell)
    }

    /// Resolves the inbound User-omitting alias without turning it into a second
    /// public projection or cacheable document.
    pub(crate) async fn resolve_permalink_alias(
        &self,
        date: PermalinkDate,
        slug: Slug,
        query: Option<&str>,
    ) -> Response {
        let username = match self
            .posts
            .resolve_post_permalink_alias(date, &slug, UtcInstant::now())
            .await
        {
            Ok(PostPermalinkAliasMatch::Unique(username)) => username,
            Ok(PostPermalinkAliasMatch::Missing | PostPermalinkAliasMatch::Ambiguous) => {
                return self.shell_response();
            }
            Err(error) => {
                return Self::boundary_response(error.into(), "server.projector.permalink_alias");
            }
        };
        let route = PermalinkRoute {
            username,
            date,
            slug,
        };
        match document::permalink_alias_redirect(&route, query) {
            Ok(response) => response,
            Err(error) => Self::boundary_response(
                InternalError::server(error),
                "server.projector.permalink_alias",
            ),
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
            Ok(None) => return Err(ProjectionFailure::ShellFallback),
            Err(error) => return Err(Self::boundary(error, "server.projector.permalink")),
        };
        let theme = match storage::resolve_public_theme(
            PublicThemeOwner::Author(record.user_id),
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
            PublicThemeOwner::Site,
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

    async fn profile(&self, username: Username) -> ProjectionResult {
        let page = match timeline::fetch_user_posts(
            self.posts.as_ref(),
            &ViewerIdentity::Anonymous,
            &username,
            None,
            Some(PageSize::default()),
        )
        .await
        {
            Ok(page) => page,
            Err(error) => return Err(Self::swallowed(error, "server.projector.profile")),
        };
        let theme = self
            .author_theme(
                &username,
                PublicThemeRoute::author(&username),
                "server.projector.profile",
            )
            .await?;
        Ok(PublicPresentation {
            theme,
            page: PageSeed::Profile { username, page },
        })
    }

    async fn site_tag(&self, tag: Tag) -> ProjectionResult {
        let page = match timeline::fetch_posts_by_tag(
            self.posts.as_ref(),
            &ViewerIdentity::Anonymous,
            &tag,
            None,
            Some(PageSize::default()),
        )
        .await
        {
            Ok(page) => page,
            Err(error) => return Err(Self::swallowed(error, "server.projector.site_tag")),
        };
        let theme = match storage::resolve_public_theme(
            PublicThemeOwner::Site,
            &PublicThemeRoute::site_tag(&tag),
            self.themes.as_ref(),
        )
        .await
        {
            Ok(theme) => theme,
            Err(error) => return Err(Self::boundary(error, "server.projector.site_tag")),
        };
        Ok(PublicPresentation {
            theme,
            page: PageSeed::SiteTag { tag, page },
        })
    }

    async fn user_tag(&self, username: Username, tag: Tag) -> ProjectionResult {
        let page = match timeline::fetch_user_posts_by_tag(
            self.posts.as_ref(),
            self.users.as_ref(),
            &ViewerIdentity::Anonymous,
            &username,
            &tag,
            None,
            Some(PageSize::default()),
        )
        .await
        {
            Ok(page) => page,
            Err(error) => return Err(Self::swallowed(error, "server.projector.user_tag")),
        };
        let theme = self
            .author_theme(
                &username,
                PublicThemeRoute::author_tag(&username, &tag),
                "server.projector.user_tag",
            )
            .await?;
        Ok(PublicPresentation {
            theme,
            page: PageSeed::UserTag {
                username,
                tag,
                page,
            },
        })
    }

    async fn author_theme(
        &self,
        username: &Username,
        route: PublicThemeRoute,
        context: &'static str,
    ) -> Result<PublishedThemePresentation, ProjectionFailure> {
        let owner = match self.users.get_user_by_username(username).await {
            Ok(Some(author)) => PublicThemeOwner::Author(author.user_id),
            Ok(None) => PublicThemeOwner::Site,
            Err(error) => return Err(Self::boundary(error, context)),
        };
        storage::resolve_public_theme(owner, &route, self.themes.as_ref())
            .await
            .map_err(|error| Self::boundary(error, context))
    }

    fn swallowed(error: InternalError, context: &'static str) -> ProjectionFailure {
        ProjectionFailure::SwallowedFailure { error, context }
    }

    fn boundary(error: impl Into<InternalError>, context: &'static str) -> ProjectionFailure {
        ProjectionFailure::BoundaryFailure(error.into().with_context("boundary", context))
    }

    fn boundary_response(error: InternalError, context: &'static str) -> Response {
        error
            .with_context("boundary", context)
            .emit_boundary_failure();
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}

/// The internal route result before its single final HTTP mapping.
type ProjectionResult = Result<PublicPresentation<PageSeed>, ProjectionFailure>;

enum ProjectionFailure {
    ShellFallback,
    SwallowedFailure {
        error: InternalError,
        context: &'static str,
    },
    BoundaryFailure(InternalError),
}
