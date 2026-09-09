use std::{collections::BTreeSet, sync::Mutex};

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::media::{MediaReference, MediaReferenceForm};
use common::tagged_url::BaseUrl;
use storage::test_support::{Backend, confirmed_for, noop_mailer};
use storage::{
    ForeignEvidenceSink, InstanceId, LocalMediaSink, MediaReferenceEvidence,
    MediaReferenceOwnershipResolver, PersistedMediaReference, ProvenLocalMediaRefs,
};
use tempfile::TempDir;
use tower::ServiceExt;

use super::registrar::ensure_server_fns_registered;
pub fn confirmed_mutation<T: serde::de::DeserializeOwned>(body: &str) -> T {
    let outcome: common::MutationOutcome<T> =
        serde_json::from_str(body).expect("parse mutation outcome");
    confirmed_for(outcome, "integration test backend")
}

/// Read a response body fully and decode it as UTF-8.
pub async fn body_string(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Runs a test-composition closure after preparing the media-root layout.
///
/// Test roots mint their exact storage handles before router composition; the
/// router receives no aggregate storage state.
pub fn prepare_app(
    storage: &TempDir,
    build: impl FnOnce(std::path::PathBuf) -> axum::Router,
) -> axum::Router {
    ensure_server_fns_registered();
    let storage_path = storage.path().to_path_buf();
    std::fs::create_dir_all(storage_path.join("media").join("upload")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("cached")).unwrap();
    std::fs::create_dir_all(storage_path.join("media").join("tmp")).unwrap();
    build(storage_path)
}

/// Router composition at a test root. The input expression supplies focused
/// storage minting methods; the expanded router crosses only exact dependencies.
macro_rules! make_app {
    ($env:expr, $storage:expr) => {
        make_app!(
            @build $storage,
            storage::InstanceId::new(),
            storage::test_support::noop_mailer(),
            false,
            std::sync::Arc::new(
                jaunder::media_ownership::LiveMediaReferenceOwnershipResolver::new(),
            );
            site_config = ($env).site_config(),
            users = ($env).users(),
            sessions = ($env).sessions(),
            invites = ($env).invites(),
            email_verifications = ($env).email_verifications(),
            password_resets = ($env).password_resets(),
            posts = ($env).posts(),
            write_scope = ($env).write_scope(),
            subscriptions = ($env).subscriptions(),
            audiences = ($env).audiences(),
            media = ($env).media(),
            user_config = ($env).user_config(),
            themes = ($env).themes(),
            feed_cache = ($env).feed_cache(),
            feed_events = ($env).feed_events(),
            publisher = ($env).publisher(),
        )
    };
    ($env:expr, $storage:expr; override_mailer = $mailer:expr) => {
        make_app!(
            @build $storage,
            storage::InstanceId::new(),
            $mailer,
            false,
            std::sync::Arc::new(
                jaunder::media_ownership::LiveMediaReferenceOwnershipResolver::new(),
            );
            site_config = ($env).site_config(),
            users = ($env).users(),
            sessions = ($env).sessions(),
            invites = ($env).invites(),
            email_verifications = ($env).email_verifications(),
            password_resets = ($env).password_resets(),
            posts = ($env).posts(),
            write_scope = ($env).write_scope(),
            subscriptions = ($env).subscriptions(),
            audiences = ($env).audiences(),
            media = ($env).media(),
            user_config = ($env).user_config(),
            themes = ($env).themes(),
            feed_cache = ($env).feed_cache(),
            feed_events = ($env).feed_events(),
            publisher = ($env).publisher(),
        )
    };
    ($env:expr, $storage:expr; secure_cookies = $secure_cookies:expr) => {
        make_app!(
            $env, $storage;
            instance_id = storage::InstanceId::new(),
            mailer = storage::test_support::noop_mailer(),
            secure_cookies = $secure_cookies,
            resolver = std::sync::Arc::new(
                jaunder::media_ownership::LiveMediaReferenceOwnershipResolver::new(),
            )
        )
    };
    (
        $env:expr, $storage:expr;
        instance_id = $instance_id:expr,
        mailer = $mailer:expr,
        secure_cookies = $secure_cookies:expr,
        resolver = $resolver:expr
    ) => {
        make_app!(
            @build $storage, $instance_id, $mailer, $secure_cookies, $resolver;
            site_config = ($env).site_config(),
            users = ($env).users(),
            sessions = ($env).sessions(),
            invites = ($env).invites(),
            email_verifications = ($env).email_verifications(),
            password_resets = ($env).password_resets(),
            posts = ($env).posts(),
            write_scope = ($env).write_scope(),
            subscriptions = ($env).subscriptions(),
            audiences = ($env).audiences(),
            media = ($env).media(),
            user_config = ($env).user_config(),
            themes = ($env).themes(),
            feed_cache = ($env).feed_cache(),
            feed_events = ($env).feed_events(),
            publisher = ($env).publisher(),
        )
    };
    ($env:expr, $storage:expr; override_sessions = $sessions:expr) => {
        make_app!(@build $storage, storage::InstanceId::new(), storage::test_support::noop_mailer(), false, std::sync::Arc::new(jaunder::media_ownership::LiveMediaReferenceOwnershipResolver::new());
            site_config = ($env).site_config(), users = ($env).users(), sessions = $sessions,
            invites = ($env).invites(), email_verifications = ($env).email_verifications(),
            password_resets = ($env).password_resets(), posts = ($env).posts(),
            write_scope = ($env).write_scope(), subscriptions = ($env).subscriptions(),
            audiences = ($env).audiences(), media = ($env).media(), user_config = ($env).user_config(),
            themes = ($env).themes(), feed_cache = ($env).feed_cache(),
            feed_events = ($env).feed_events(), publisher = ($env).publisher(),
        )
    };
    ($env:expr, $storage:expr; override_site_config = $site_config:expr) => {
        make_app!(@build $storage, storage::InstanceId::new(), storage::test_support::noop_mailer(), false, std::sync::Arc::new(jaunder::media_ownership::LiveMediaReferenceOwnershipResolver::new());
            site_config = $site_config, users = ($env).users(), sessions = ($env).sessions(),
            invites = ($env).invites(), email_verifications = ($env).email_verifications(),
            password_resets = ($env).password_resets(), posts = ($env).posts(),
            write_scope = ($env).write_scope(), subscriptions = ($env).subscriptions(),
            audiences = ($env).audiences(), media = ($env).media(), user_config = ($env).user_config(),
            themes = ($env).themes(), feed_cache = ($env).feed_cache(),
            feed_events = ($env).feed_events(), publisher = ($env).publisher(),
        )
    };
    ($env:expr, $storage:expr; override_feed_cache = $feed_cache:expr) => {
        make_app!(@build $storage, storage::InstanceId::new(), storage::test_support::noop_mailer(), false, std::sync::Arc::new(jaunder::media_ownership::LiveMediaReferenceOwnershipResolver::new());
            site_config = ($env).site_config(), users = ($env).users(), sessions = ($env).sessions(),
            invites = ($env).invites(), email_verifications = ($env).email_verifications(),
            password_resets = ($env).password_resets(), posts = ($env).posts(),
            write_scope = ($env).write_scope(), subscriptions = ($env).subscriptions(),
            audiences = ($env).audiences(), media = ($env).media(), user_config = ($env).user_config(),
            themes = ($env).themes(), feed_cache = $feed_cache,
            feed_events = ($env).feed_events(), publisher = ($env).publisher(),
        )
    };
    ($env:expr, $storage:expr; override_posts_and_publisher = $posts:expr, $publisher:expr) => {
        make_app!(@build $storage, storage::InstanceId::new(), storage::test_support::noop_mailer(), false, std::sync::Arc::new(jaunder::media_ownership::LiveMediaReferenceOwnershipResolver::new());
            site_config = ($env).site_config(), users = ($env).users(), sessions = ($env).sessions(),
            invites = ($env).invites(), email_verifications = ($env).email_verifications(),
            password_resets = ($env).password_resets(), posts = $posts,
            write_scope = ($env).write_scope(), subscriptions = ($env).subscriptions(),
            audiences = ($env).audiences(), media = ($env).media(), user_config = ($env).user_config(),
            themes = ($env).themes(), feed_cache = ($env).feed_cache(),
            feed_events = ($env).feed_events(), publisher = $publisher,
        )
    };
    (
        @build $storage:expr,
        $instance_id:expr, $mailer:expr, $secure_cookies:expr, $resolver:expr;
        site_config = $site_config:expr,
        users = $users:expr,
        sessions = $sessions:expr,
        invites = $invites:expr,
        email_verifications = $email_verifications:expr,
        password_resets = $password_resets:expr,
        posts = $posts:expr,
        write_scope = $write_scope:expr,
        subscriptions = $subscriptions:expr,
        audiences = $audiences:expr,
        media = $media:expr,
        user_config = $user_config:expr,
        themes = $themes:expr,
        feed_cache = $feed_cache:expr,
        feed_events = $feed_events:expr,
        publisher = $publisher:expr,
    ) => {
        $crate::helpers::prepare_app($storage, |storage_path| {
            let site_config: std::sync::Arc<dyn storage::SiteConfigStorage> = $site_config;
            let users: std::sync::Arc<dyn storage::UserStorage> = $users;
            let sessions: std::sync::Arc<dyn storage::SessionStorage> = $sessions;
            let invites: std::sync::Arc<dyn storage::InviteStorage> = $invites;
            let email_verifications: std::sync::Arc<
                dyn storage::EmailVerificationStorage,
            > = $email_verifications;
            let password_resets: std::sync::Arc<dyn storage::PasswordResetStorage> =
                $password_resets;
            let posts: std::sync::Arc<dyn storage::PostStorage> = $posts;
            let write_scope = $write_scope;
            let subscriptions: std::sync::Arc<dyn storage::SubscriptionStorage> =
                $subscriptions;
            let audiences: std::sync::Arc<dyn storage::AudienceStorage> = $audiences;
            let media: std::sync::Arc<dyn storage::MediaStorage> = $media;
            let user_config: std::sync::Arc<dyn storage::UserConfigStorage> = $user_config;
            let themes: std::sync::Arc<dyn storage::ThemeStorage> = $themes;
            let feed_cache: std::sync::Arc<dyn storage::FeedCacheStorage> = $feed_cache;
            let feed_events: std::sync::Arc<dyn storage::FeedEventStorage> = $feed_events;
            let publisher: std::sync::Arc<dyn storage::PublisherStorage> = $publisher;
            let instance_id = $instance_id;
            let resolver: std::sync::Arc<dyn storage::MediaReferenceOwnershipResolver> =
                $resolver;
            let mailer: std::sync::Arc<dyn common::mailer::MailSender> = $mailer;
            let storage_path = std::sync::Arc::new(storage_path);
            let content_locks = std::sync::Arc::new(storage::MediaContentLocks::new(
                std::sync::Arc::clone(&storage_path),
            ));
            let post_media_ownership = storage::PostMediaOwnership::new(
                resolver.clone(),
                instance_id.clone(),
                site_config.clone(),
            );
            let publisher_service = std::sync::Arc::new(jaunder::publisher::PublisherService::new(
                (*storage_path).clone(),
                publisher,
                write_scope.clone(),
            ));
            let media_manager = std::sync::Arc::new(storage::MediaManager::new(
                media.clone(),
                posts.clone(),
                site_config.clone(),
                write_scope.clone(),
                std::sync::Arc::clone(&content_locks),
                instance_id.clone(),
                resolver,
            ));
            let theme_asset_manager = std::sync::Arc::new(storage::ThemeAssetManager::new(
                themes.clone(),
                write_scope.clone(),
                std::sync::Arc::clone(&storage_path),
            ));
            let theme_operation_coordinator =
                std::sync::Arc::new(host::theme_operations::ThemeOperationCoordinator::new());
            let theme_manager = std::sync::Arc::new(storage::ThemeManager::new(
                themes.clone(),
                media.clone(),
                write_scope.clone(),
                std::sync::Arc::clone(&content_locks),
            ));
            let provide_contexts = {
                let users = users.clone();
                let sessions = sessions.clone();
                let invites = invites.clone();
                let email_verifications = email_verifications.clone();
                let password_resets = password_resets.clone();
                let posts = posts.clone();
                let write_scope = write_scope.clone();
                let subscriptions = subscriptions.clone();
                let audiences = audiences.clone();
                let media = media.clone();
                let user_config = user_config.clone();
                let site_config = site_config.clone();
                let themes = themes.clone();
                let feed_events = feed_events.clone();
                let publisher_service = publisher_service.clone();
                let content_locks = content_locks.clone();
                let media_manager = media_manager.clone();
                let post_media_ownership = post_media_ownership.clone();
                let theme_asset_manager = theme_asset_manager.clone();
                let theme_operation_coordinator = theme_operation_coordinator.clone();
                let theme_manager = theme_manager.clone();
                let mailer = mailer.clone();
                move || {
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::UserStorage>>(users.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::SessionStorage>>(sessions.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::InviteStorage>>(invites.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::EmailVerificationStorage>>(email_verifications.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::PasswordResetStorage>>(password_resets.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::PostStorage>>(posts.clone());
                    leptos::prelude::provide_context::<storage::WriteScope>(write_scope.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::SubscriptionStorage>>(subscriptions.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::AudienceStorage>>(audiences.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::MediaStorage>>(media.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::UserConfigStorage>>(user_config.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::SiteConfigStorage>>(site_config.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::ThemeStorage>>(themes.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn storage::FeedEventStorage>>(feed_events.clone());
                    leptos::prelude::provide_context(publisher_service.clone());
                    leptos::prelude::provide_context::<std::sync::Arc<dyn web::websub::WebsubPublisher>>(publisher_service.clone());
                    leptos::prelude::provide_context(post_media_ownership.clone());
                    jaunder::context::provide_media_content_locks_context(&content_locks);
                    jaunder::context::provide_mailer_context(&mailer);
                    jaunder::context::provide_media_manager_context(&media_manager);
                    jaunder::context::provide_theme_asset_manager_context(&theme_asset_manager);
                    jaunder::context::provide_theme_operation_coordinator_context(&theme_operation_coordinator);
                    jaunder::context::provide_theme_manager_context(&theme_manager);
                    leptos::prelude::provide_context(web::auth::CookieSettings {
                        secure: $secure_cookies,
                    });
                }
            };
            let public_projector = jaunder::projector::PublicProjector::new(
                posts.clone(),
                users.clone(),
                themes.clone(),
                jaunder::projector::Shell(jaunder::site::shell_html()),
            );
            let app = jaunder::application_routes(
                jaunder::client_telemetry_routes(sessions.clone(), write_scope.clone()),
                provide_contexts,
                public_projector,
            )
            .layer(axum::Extension(post_media_ownership))
            .layer(axum::Extension(media_manager))
            .layer(axum::Extension(content_locks))
            .layer(axum::Extension(storage_path))
            .layer(axum::Extension(posts))
            .layer(axum::Extension(audiences))
            .layer(axum::Extension(users))
            .layer(axum::Extension(user_config))
            .layer(axum::Extension(themes))
            .layer(axum::Extension(site_config))
            .layer(axum::Extension(media))
            .layer(axum::Extension(feed_cache))
            .layer(axum::Extension(publisher_service))
            .layer(axum::Extension(feed_events))
            .layer(axum::Extension(sessions))
            .layer(axum::Extension(write_scope));
            jaunder::create_router(app, &instance_id, $secure_cookies)
                .expect("canonical instance identity is an HTTP header")
        })
    };
}
pub(crate) use make_app as make_app_macro;

/// Deterministic resolver that proves only configured exact persisted URL forms
/// foreign. It records each input batch so endpoint tests can assert one global
/// resolution before deletion takes locks.
pub struct ForeignReferenceResolver {
    foreign_forms: Mutex<BTreeSet<MediaReferenceForm>>,
    calls: Mutex<Vec<Vec<PersistedMediaReference>>>,
}

impl ForeignReferenceResolver {
    pub fn new(foreign_forms: impl IntoIterator<Item = MediaReferenceForm>) -> Self {
        Self {
            foreign_forms: Mutex::new(foreign_forms.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn insert_foreign_form(&self, reference_form: MediaReferenceForm) {
        self.foreign_forms
            .lock()
            .expect("foreign forms lock")
            .insert(reference_form);
    }

    pub fn calls(&self) -> Vec<Vec<PersistedMediaReference>> {
        self.calls.lock().expect("resolver calls lock").clone()
    }
}

#[async_trait]
impl MediaReferenceOwnershipResolver for ForeignReferenceResolver {
    async fn resolve(
        &self,
        references: &[PersistedMediaReference],
        _instance_id: &InstanceId,
        _base_url: Option<&BaseUrl>,
        mut foreign: ForeignEvidenceSink,
    ) -> MediaReferenceEvidence {
        self.calls
            .lock()
            .expect("resolver calls lock")
            .push(references.to_vec());
        for reference in references {
            if self
                .foreign_forms
                .lock()
                .expect("foreign forms lock")
                .contains(reference.reference_form())
            {
                foreign.prove_foreign(reference.clone());
            }
        }
        foreign.finish()
    }

    async fn resolve_local(
        &self,
        _references: &[MediaReference],
        _instance_id: &InstanceId,
        _base_url: Option<&BaseUrl>,
        local: LocalMediaSink,
    ) -> ProvenLocalMediaRefs {
        local.finish()
    }
}

struct RequestCredentials<'a> {
    cookie: Option<&'a str>,
    authorization: Option<&'a str>,
}

/// A POST body paired with its content type — the two always travel together, so
/// they are one argument. `Form` is `application/x-www-form-urlencoded`, `Json` is
/// `application/json`.
enum PostBody {
    Form(String),
    Json(String),
}

impl PostBody {
    fn server_fn<I>(input: &I) -> Self
    where
        I: serde::Serialize,
    {
        Self::Form(serde_qs::to_string(input).expect("failed to encode server-function input"))
    }

    fn content_type(&self) -> &'static str {
        match self {
            PostBody::Form(_) => "application/x-www-form-urlencoded",
            PostBody::Json(_) => "application/json",
        }
    }

    fn into_string(self) -> String {
        match self {
            PostBody::Form(s) | PostBody::Json(s) => s,
        }
    }
}

pub struct TestHttpResponse {
    pub status: StatusCode,
    pub set_cookies: Vec<String>,
    pub body: String,
}

impl TestHttpResponse {
    fn into_without_cookies(self) -> (StatusCode, String) {
        (self.status, self.body)
    }

    fn into_first_cookie(self) -> (StatusCode, Option<String>, String) {
        (self.status, self.set_cookies.into_iter().next(), self.body)
    }
}

/// The single implementation behind every POST transport helper. Its caller
/// supplies the already-composed route behavior, so no storage aggregate crosses
/// this transport seam.
async fn post_inner(
    app: axum::Router,
    uri: &str,
    body: PostBody,
    credentials: RequestCredentials<'_>,
    user_agent: Option<&str>,
) -> TestHttpResponse {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, body.content_type());
    if let Some(cookie) = credentials.cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(authorization) = credentials.authorization {
        builder = builder.header(header::AUTHORIZATION, authorization);
    }
    if let Some(ua) = user_agent {
        builder = builder.header(header::USER_AGENT, ua);
    }
    let request = builder
        .body(Body::from(body.into_string()))
        .expect("failed to build request");

    let response = app.oneshot(request).await.expect("router oneshot failed");

    let status = response.status();
    let set_cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| {
            value
                .to_str()
                .expect("Set-Cookie header is not valid UTF-8")
                .to_owned()
        })
        .collect();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body = String::from_utf8(bytes.to_vec()).expect("response body is not valid UTF-8");

    TestHttpResponse {
        status,
        set_cookies,
        body,
    }
}

/// Canonical case: cookie auth with `Set-Cookie` dropped.
pub async fn post_form(
    app: axum::Router,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_inner(
        app,
        uri,
        PostBody::Form(body.into()),
        RequestCredentials {
            cookie,
            authorization: None,
        },
        None,
    )
    .await
    .into_without_cookies()
}

/// Shared typed/fixture server-function dispatcher. `F` selects the generated
/// endpoint path; `I` supplies the serializable wire shape.
async fn post_server_fn_inner<F, I>(
    app: axum::Router,
    input: &I,
    cookie: Option<&str>,
    user_agent: Option<&str>,
) -> (StatusCode, Option<String>, String)
where
    F: server_fn::ServerFn,
    I: serde::Serialize,
{
    post_inner(
        app,
        F::PATH,
        PostBody::server_fn(input),
        RequestCredentials {
            cookie,
            authorization: None,
        },
        user_agent,
    )
    .await
    .into_first_cookie()
}

/// POST one typed server-function input using that function's derived path and
/// default URL-encoded input codec.
pub async fn post_server_fn<F>(
    app: axum::Router,
    input: &F,
    cookie: Option<&str>,
) -> (StatusCode, String)
where
    F: serde::Serialize + server_fn::ServerFn,
{
    let (status, _set_cookie, body) = post_server_fn_inner::<F, F>(app, input, cookie, None).await;
    (status, body)
}

/// Posts a typed server function through an already-composed router with
/// deterministic ownership behavior supplied at the test root.
pub async fn post_server_fn_with_media_ownership_resolver<F>(
    app: axum::Router,
    input: &F,
    cookie: Option<&str>,
) -> (StatusCode, String)
where
    F: serde::Serialize + server_fn::ServerFn,
{
    let mut builder = Request::builder()
        .method("POST")
        .uri(F::PATH)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let request = builder
        .body(Body::from(
            serde_qs::to_string(input).expect("server function input encodes"),
        ))
        .expect("server function request builds");
    let response = app.oneshot(request).await.expect("router request succeeds");
    let status = response.status();
    let body = body_string(response).await;
    (status, body)
}

/// Posts a reset request through an already-composed production router. Tests
/// that replace detached-worker dependencies build that router at their root.
pub async fn post_password_reset_request_with_dependencies(
    app: axum::Router,
    input: &web::password_reset::Request,
) -> (StatusCode, String) {
    post_password_reset_form_with_dependencies(
        app,
        serde_qs::to_string(input).expect("server function input encodes"),
    )
    .await
}

/// Posts a raw password-reset request form through an already-composed router.
/// Decode-rejection tests use this when an invalid value cannot inhabit the
/// operation's typed request.
pub async fn post_password_reset_form_with_dependencies(
    app: axum::Router,
    body: impl Into<String>,
) -> (StatusCode, String) {
    let request = Request::builder()
        .method("POST")
        .uri(<web::password_reset::Request as server_fn::ServerFn>::PATH)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.into()))
        .expect("server function request builds");
    let response = app.oneshot(request).await.expect("router request succeeds");
    let status = response.status();
    let body = body_string(response).await;
    (status, body)
}

#[derive(serde::Serialize)]
struct RequestFixture<'a, R> {
    request: &'a R,
}

/// POST a serializable request-aggregate fixture to server function `F`.
///
/// Decode-rejection tests use this when an invalid value cannot inhabit the
/// operation's typed request. Valid requests use [`post_server_fn`].
pub async fn post_server_fn_request_fixture<F, R>(
    app: axum::Router,
    request: &R,
    cookie: Option<&str>,
) -> (StatusCode, String)
where
    F: server_fn::ServerFn,
    R: serde::Serialize,
{
    let input = RequestFixture { request };
    let (status, _set_cookie, body) = post_server_fn_inner::<F, _>(app, &input, cookie, None).await;
    (status, body)
}

/// Like [`post_server_fn`], but exposes the secure-cookie toggle and returns
/// `Set-Cookie`.
/// Like [`post_server_fn`], but returns `Set-Cookie`; cookie behavior belongs to
/// the router supplied by the test root.
pub async fn post_server_fn_with_secure_flag<F>(
    app: axum::Router,
    input: &F,
    cookie: Option<&str>,
    _secure_cookies: bool,
) -> (StatusCode, Option<String>, String)
where
    F: serde::Serialize + server_fn::ServerFn,
{
    post_server_fn_inner::<F, F>(app, input, cookie, None).await
}

/// Fixture counterpart to [`post_server_fn_with_secure_flag`].
pub async fn post_server_fn_request_fixture_with_secure_flag<F, R>(
    app: axum::Router,
    request: &R,
    cookie: Option<&str>,
    _secure_cookies: bool,
) -> (StatusCode, Option<String>, String)
where
    F: server_fn::ServerFn,
    R: serde::Serialize,
{
    let input = RequestFixture { request };
    post_server_fn_inner::<F, _>(app, &input, cookie, None).await
}

/// Like [`post_server_fn_with_secure_flag`], but also sets `User-Agent`.
pub async fn post_server_fn_with_ua<F>(
    app: axum::Router,
    input: &F,
    cookie: Option<&str>,
    user_agent: &str,
    _secure_cookies: bool,
) -> (StatusCode, Option<String>, String)
where
    F: serde::Serialize + server_fn::ServerFn,
{
    post_server_fn_inner::<F, F>(app, input, cookie, Some(user_agent)).await
}

/// Exposes the `secure_cookies` toggle and returns the `Set-Cookie` value —
/// what the auth/session tests need over the canonical [`post_form`].
pub async fn post_form_with_secure_flag(
    app: axum::Router,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
    _secure_cookies: bool,
) -> (StatusCode, Option<String>, String) {
    post_inner(
        app,
        uri,
        PostBody::Form(body.into()),
        RequestCredentials {
            cookie,
            authorization: None,
        },
        None,
    )
    .await
    .into_first_cookie()
}

/// Authenticates with an `Authorization: Bearer <token>` header instead of a
/// cookie. Returns the first `Set-Cookie` value like the existing auth helpers.
pub async fn post_form_with_bearer(
    app: axum::Router,
    uri: &str,
    body: impl Into<String>,
    bearer: &str,
) -> (StatusCode, Option<String>, String) {
    let authorization = format!("Bearer {bearer}");
    post_inner(
        app,
        uri,
        PostBody::Form(body.into()),
        RequestCredentials {
            cookie: None,
            authorization: Some(&authorization),
        },
        None,
    )
    .await
    .into_first_cookie()
}

/// Sends form data with cookie and Authorization headers controlled independently.
pub async fn post_form_with_credentials(
    app: axum::Router,
    uri: &str,
    body: impl Into<String>,
    cookie: Option<&str>,
    authorization: Option<&str>,
    _secure_cookies: bool,
) -> TestHttpResponse {
    post_inner(
        app,
        uri,
        PostBody::Form(body.into()),
        RequestCredentials {
            cookie,
            authorization,
        },
        None,
    )
    .await
}

/// POST a JSON body (`Content-Type: application/json`) with secure cookies and
/// optional cookie auth; returns `(status, body)` — drops `Set-Cookie`, like the
/// canonical [`post_form`].
pub async fn post_json(
    app: axum::Router,
    uri: &str,
    body: serde_json::Value,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    post_json_with_credentials(app, uri, body, cookie, None, true)
        .await
        .into_without_cookies()
}

/// Sends JSON with cookie and Authorization headers controlled independently.
pub async fn post_json_with_credentials(
    app: axum::Router,
    uri: &str,
    body: serde_json::Value,
    cookie: Option<&str>,
    authorization: Option<&str>,
    _secure_cookies: bool,
) -> TestHttpResponse {
    post_inner(
        app,
        uri,
        PostBody::Json(body.to_string()),
        RequestCredentials {
            cookie,
            authorization,
        },
        None,
    )
    .await
}

/// One uploaded multipart file used by [`post_multipart`].
pub struct MultipartFile<'a> {
    pub filename: &'a str,
    pub content_type: &'a str,
    pub bytes: &'a [u8],
}

/// POST a single-file `multipart/form-data` body to an already-composed router.
/// Returns `(status, body)`. Mirrors the exact CRLF framing of the multipart
/// request in `misc/media_handlers.rs`.
pub async fn post_multipart(
    app: axum::Router,
    uri: &str,
    file: MultipartFile<'_>,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let boundary = "----testboundary1234";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
            file.filename, file.content_type,
        )
        .as_bytes(),
    );
    body.extend_from_slice(file.bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let mut builder = Request::builder().method("POST").uri(uri).header(
        header::CONTENT_TYPE,
        format!("multipart/form-data; boundary={boundary}"),
    );
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let request = builder
        .body(Body::from(body))
        .expect("failed to build request");
    let response = app.oneshot(request).await.expect("router oneshot failed");
    let status = response.status();
    (status, body_string(response).await)
}

/// GET a static asset and return `(status, Content-Type)`. Pins the Sqlite backend
/// — static-asset serving never touches storage, so it need not run on both.
pub async fn get_asset(uri: &str) -> (StatusCode, Option<String>) {
    let env = Backend::Sqlite.setup().await;
    let storage = TempDir::new().expect("test storage directory");

    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();

    let app = make_app!(
        &env,
        &storage;
        instance_id = env.base.instance_id().clone(),
        mailer = noop_mailer(),
        secure_cookies = false,
        resolver = std::sync::Arc::new(
            jaunder::media_ownership::LiveMediaReferenceOwnershipResolver::new(),
        )
    );
    let response = app.oneshot(request).await.unwrap();

    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string());

    (status, content_type)
}
