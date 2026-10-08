use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use common::theme::{PublicThemeSelection, Theme};
use common::{post_title::PostTitle, registration::RegistrationPolicy, site::SiteIdentity};
use jiff::tz::Offset;
use storage::test_support::{SeedRawPost, SeedUser, TestEnv, confirmed};
use storage::{
    MockSiteConfigStorage, MockThemeStorage, PostStorage, RenderedHtml, SiteConfigStorage,
    SystemThemeRevision, ThemeAssetManager, ThemeOwner, ThemeStorage, UserStorage,
};

/// A recognizable stand-in for the real `index.html`, so tests can tell a
/// shell-fallback response apart from a projected one.
pub(super) const TEST_SHELL: &str = "<!DOCTYPE html><!--test-shell--><html><body></body></html>";

/// Installs the closed release inventory at an explicit projector test root.
///
/// Projector tests intentionally start from pristine storage; unlike production
/// startup they must opt into system artifact installation before public
/// presentation resolution can be exercised.
pub(super) async fn install_projector_system_inventory(env: &TestEnv) {
    let inventory = host::system_theme::compile_system_artifact_inventory()
        .expect("closed system artifact inventory compiles");
    let manager = ThemeAssetManager::new(
        env.themes(),
        env.write_scope().clone(),
        Arc::new(env.base.path().to_path_buf()),
    );
    confirmed(
        manager
            .install_system(&inventory, 0)
            .await
            .expect("install projector system artifact inventory"),
    );
}

fn application_stylesheet_url() -> common::root_relative_url::RootRelativeUrl {
    host::system_theme::compile_system_artifact_inventory()
        .expect("closed system artifact inventory compiles")
        .application()
        .content_digest()
        .content_url()
}

/// A router carrying only the public projector routes and their storage
/// dependencies.
/// The projector is feature-independent (mounted into the live router only under
/// `csr`, but `register` itself always compiles), so registering it onto a bare
/// router exercises it directly under the default test build — no `csr` feature,
/// no full `create_router`.
pub(super) fn projector_app(
    posts: Arc<dyn PostStorage>,
    users: Arc<dyn UserStorage>,
    themes: Arc<dyn ThemeStorage>,
) -> Router {
    projector_app_with_dependencies(posts, users, themes)
}

/// A projector router with independently replaceable storage dependencies.
///
/// Real-backend failure tests close a shared pool, which cannot reach a later
/// dependency after an earlier database read succeeds. Trait-level replacements
/// keep the preceding route reads real while faulting the intended boundary.
pub(super) fn projector_app_with_dependencies(
    posts: Arc<dyn PostStorage>,
    users: Arc<dyn UserStorage>,
    themes: Arc<dyn ThemeStorage>,
) -> Router {
    projector_app_with_site_config(posts, users, themes, default_site_config())
}

/// A projector router with an independently replaceable Site Config dependency.
pub(super) fn projector_app_with_site_config(
    posts: Arc<dyn PostStorage>,
    users: Arc<dyn UserStorage>,
    themes: Arc<dyn ThemeStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
) -> Router {
    let projector = jaunder::projector::PublicProjector::new(
        posts,
        users,
        themes,
        site_config,
        application_stylesheet_url(),
        jaunder::projector::Shell(TEST_SHELL.into()),
    );
    jaunder::projector::register(Router::new(), projector)
}

fn default_site_config() -> Arc<dyn SiteConfigStorage> {
    let mut site_config = MockSiteConfigStorage::new();
    site_config.expect_get_identity().returning(|| {
        Ok(SiteIdentity {
            title: "Jaunder".parse().unwrap(),
            tagline: None,
            base_url: None,
        })
    });
    site_config
        .expect_get_registration_policy()
        .returning(|| Ok(RegistrationPolicy::Open));
    Arc::new(site_config)
}

/// A site selection store whose read fails after the route's content query succeeds.
pub(super) fn failing_site_theme_selection(message: &'static str) -> Arc<dyn ThemeStorage> {
    let mut themes = MockThemeStorage::new();
    themes
        .expect_selection()
        .times(1)
        .return_once(move |_| Err(sqlx::Error::Io(std::io::Error::other(message))));
    Arc::new(themes)
}

fn mocked_system_revision(theme: Theme) -> SystemThemeRevision {
    let digest = match theme {
        Theme::Terminal => 'a',
        Theme::Studio => 'b',
        Theme::Reader => 'c',
    }
    .to_string()
    .repeat(64);
    SystemThemeRevision {
        theme,
        digest: digest.parse().expect("test revision digest"),
        source_digest: digest.parse().expect("test source digest"),
        stylesheet_digest: digest.parse().expect("test stylesheet digest"),
        manifest: br#"{"defaults":{}}"#.to_vec(),
        assets: vec![],
    }
}

/// An author selection store that resolves the site selection before failing.
pub(super) fn failing_author_theme_selection(message: &'static str) -> Arc<dyn ThemeStorage> {
    let mut themes = MockThemeStorage::new();
    themes
        .expect_system_theme_revision()
        .returning(|theme| Ok(Some(mocked_system_revision(theme))));
    themes
        .expect_selection()
        .times(2)
        .returning(move |owner| match owner {
            ThemeOwner::Site => Ok(Some(PublicThemeSelection::BuiltIn(Theme::Studio))),
            ThemeOwner::Author(_) => Err(sqlx::Error::Io(std::io::Error::other(message))),
        });
    Arc::new(themes)
}

/// Seed a published, `rust`-tagged post; returns the seeded user's username and the
/// post's autogenerated title (the tag pages assert the post is present by its title).
pub(super) async fn seed_tagged_post(
    users: Arc<dyn UserStorage>,
    posts: Arc<dyn PostStorage>,
    write_scope: storage::WriteScope,
) -> (String, PostTitle) {
    let user = SeedUser::new().seed(users, write_scope.clone()).await;
    let post = SeedRawPost::new(user.user_id)
        .tags(["rust"])
        .seed(posts, write_scope)
        .await;
    (user.username.to_string(), post.title)
}

/// Seed a published post and return the permalink components (username, y, m, d, slug)
/// plus the post's autogenerated title and rendered HTML for the projected-content
/// assertions.
pub(super) async fn seed_published_post(
    users: Arc<dyn UserStorage>,
    posts: Arc<dyn PostStorage>,
    write_scope: storage::WriteScope,
) -> (String, i32, u32, u32, String, PostTitle, RenderedHtml) {
    let user = SeedUser::new().seed(users, write_scope.clone()).await;
    let post = SeedRawPost::new(user.user_id)
        .seed(posts, write_scope)
        .await;
    let published_at = post.published_at.expect("seeded post is published");
    let published_date = Offset::UTC.to_datetime(published_at.value()).date();
    (
        user.username.to_string(),
        i32::from(published_date.year()),
        u32::try_from(published_date.month()).expect("Jiff civil month fits u32"),
        u32::try_from(published_date.day()).expect("Jiff civil day fits u32"),
        post.slug.to_string(),
        post.title,
        post.rendered_html,
    )
}

pub(super) fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

/// Assert an indistinguishable no-store public shell miss.
pub(super) async fn assert_shell_miss(response: Response) {
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read shell body");
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("test-shell"), "served shell: {body}");
    assert!(!body.contains("jaunder-seed"), "shell has no projection");
}

/// Assert the public projector's sanitized, non-cacheable storage-failure response.
pub(super) async fn assert_sanitized_internal_server_error(response: Response) {
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        response.headers().get(header::CACHE_CONTROL).is_none(),
        "500 is not cached"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    assert!(body.is_empty(), "500 body remains sanitized");
}
