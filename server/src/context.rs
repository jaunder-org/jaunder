//! Helpers for placing server dependencies in Leptos request context.
//!
//! The server composition roots own storage construction. They provide only
//! the exact contexts their route setup requires.

use std::sync::Arc;

use axum::Router;

use common::mailer::MailSender;
use host::theme_operations::ThemeOperationCoordinator;
use leptos::prelude::provide_context;
use storage::{
    AudienceStorage, EmailVerificationStorage, FeedEventStorage, InviteStorage, MediaContentLocks,
    MediaManager, MediaStorage, PasswordResetStorage, PostMediaOwnership, PostStorage,
    SessionStorage, SiteConfigStorage, SubscriptionStorage, ThemeAssetManager, ThemeManager,
    ThemeStorage, UserConfigStorage, UserStorage, WriteScope,
};

/// Places the shared media filesystem coordinator in the current Leptos
/// request context.
pub fn provide_media_content_locks_context(content_locks: &Arc<MediaContentLocks>) {
    provide_context(Arc::clone(content_locks));
}

/// Places the shared media operation manager in the current Leptos request context.
pub fn provide_media_manager_context(manager: &Arc<MediaManager>) {
    provide_context(Arc::clone(manager));
}

/// Places immutable Theme content lifecycle operations in the request context.
pub fn provide_theme_asset_manager_context(manager: &Arc<ThemeAssetManager>) {
    provide_context(Arc::clone(manager));
}

/// Places theme binding/removal orchestration in the request context.
pub fn provide_theme_manager_context(manager: &Arc<ThemeManager>) {
    provide_context(Arc::clone(manager));
}

/// Places principal-scoped Theme Package admission in the request context.
pub fn provide_theme_operation_coordinator_context(coordinator: &Arc<ThemeOperationCoordinator>) {
    provide_context(Arc::clone(coordinator));
}

/// Place the mailer in the current Leptos context. Server functions that
/// send mail fetch it with `expect_context::<Arc<dyn MailSender>>()`.
pub fn provide_mailer_context(mailer: &Arc<dyn MailSender>) {
    provide_context::<Arc<dyn MailSender>>(mailer.clone());
}

/// Captures account-related storage handles for one server-function request.
pub fn account_context_provider(
    users: Arc<dyn UserStorage>,
    sessions: Arc<dyn SessionStorage>,
    invites: Arc<dyn InviteStorage>,
    email_verifications: Arc<dyn EmailVerificationStorage>,
    password_resets: Arc<dyn PasswordResetStorage>,
) -> impl Fn() + Clone + Send + Sync + 'static {
    move || {
        provide_context::<Arc<dyn UserStorage>>(users.clone());
        provide_context::<Arc<dyn SessionStorage>>(sessions.clone());
        provide_context::<Arc<dyn InviteStorage>>(invites.clone());
        provide_context::<Arc<dyn EmailVerificationStorage>>(email_verifications.clone());
        provide_context::<Arc<dyn PasswordResetStorage>>(password_resets.clone());
    }
}

/// Captures post-publication storage handles for one server-function request.
pub fn publication_context_provider(
    posts: Arc<dyn PostStorage>,
    write_scope: WriteScope,
    subscriptions: Arc<dyn SubscriptionStorage>,
    audiences: Arc<dyn AudienceStorage>,
    feed_events: Arc<dyn FeedEventStorage>,
) -> impl Fn() + Clone + Send + Sync + 'static {
    move || {
        provide_context::<Arc<dyn PostStorage>>(posts.clone());
        provide_context(write_scope.clone());
        provide_context::<Arc<dyn SubscriptionStorage>>(subscriptions.clone());
        provide_context::<Arc<dyn AudienceStorage>>(audiences.clone());
        provide_context::<Arc<dyn FeedEventStorage>>(feed_events.clone());
    }
}

/// Captures media settings storage handles for one server-function request.
pub fn media_configuration_context_provider(
    media: Arc<dyn MediaStorage>,
    user_config: Arc<dyn UserConfigStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
) -> impl Fn() + Clone + Send + Sync + 'static {
    move || {
        provide_context::<Arc<dyn MediaStorage>>(media.clone());
        provide_context::<Arc<dyn UserConfigStorage>>(user_config.clone());
        provide_context::<Arc<dyn SiteConfigStorage>>(site_config.clone());
    }
}

/// Captures Theme storage for one server-function request.
pub fn theme_context_provider(
    themes: Arc<dyn ThemeStorage>,
) -> impl Fn() + Clone + Send + Sync + 'static {
    move || provide_context::<Arc<dyn ThemeStorage>>(themes.clone())
}

/// Captures server-owned operation services for one server-function request.
pub fn service_context_provider(
    mailer: Arc<dyn MailSender>,
    content_locks: Arc<MediaContentLocks>,
    media_manager: Arc<MediaManager>,
    theme_asset_manager: Arc<ThemeAssetManager>,
    theme_operation_coordinator: Arc<ThemeOperationCoordinator>,
    theme_manager: Arc<ThemeManager>,
    secure_cookies: bool,
) -> impl Fn() + Clone + Send + Sync + 'static {
    move || {
        provide_mailer_context(&mailer);
        provide_media_content_locks_context(&content_locks);
        provide_media_manager_context(&media_manager);
        provide_theme_asset_manager_context(&theme_asset_manager);
        provide_theme_operation_coordinator_context(&theme_operation_coordinator);
        provide_theme_manager_context(&theme_manager);
        provide_context(web::auth::CookieSettings {
            secure: secure_cookies,
        });
    }
}

/// Captures the publisher's concrete service and web-facing capability.
pub fn publisher_context_provider(
    publisher: Arc<crate::publisher::PublisherService>,
) -> impl Fn() + Clone + Send + Sync + 'static {
    move || {
        provide_context(Arc::clone(&publisher));
        provide_context::<Arc<dyn web::websub::WebsubPublisher>>(publisher.clone());
    }
}

/// Captures media-reference ownership resolution for one server-function request.
pub fn post_media_ownership_context_provider(
    ownership: PostMediaOwnership,
) -> impl Fn() + Clone + Send + Sync + 'static {
    move || provide_context(ownership.clone())
}

/// Adds filesystem and ownership extensions used by media handlers.
pub fn with_media_extensions(
    app: Router,
    ownership: PostMediaOwnership,
    manager: Arc<MediaManager>,
    content_locks: Arc<MediaContentLocks>,
    storage_path: Arc<std::path::PathBuf>,
) -> Router {
    app.layer(axum::Extension(ownership))
        .layer(axum::Extension(manager))
        .layer(axum::Extension(content_locks))
        .layer(axum::Extension(storage_path))
}

/// Adds post and account storage extensions used by request handlers.
pub fn with_post_account_extensions(
    app: Router,
    posts: Arc<dyn PostStorage>,
    audiences: Arc<dyn AudienceStorage>,
    users: Arc<dyn UserStorage>,
    user_config: Arc<dyn UserConfigStorage>,
) -> Router {
    app.layer(axum::Extension(posts))
        .layer(axum::Extension(audiences))
        .layer(axum::Extension(users))
        .layer(axum::Extension(user_config))
}

/// Adds Theme and media configuration extensions used by request handlers.
pub fn with_theme_media_extensions(
    app: Router,
    themes: Arc<dyn ThemeStorage>,
    site_config: Arc<dyn SiteConfigStorage>,
    media: Arc<dyn MediaStorage>,
    feed_cache: Arc<dyn storage::FeedCacheStorage>,
) -> Router {
    app.layer(axum::Extension(themes))
        .layer(axum::Extension(site_config))
        .layer(axum::Extension(media))
        .layer(axum::Extension(feed_cache))
}

/// Adds publisher and request-scoped storage extensions used by request handlers.
pub fn with_publisher_extensions(
    app: Router,
    publisher: Arc<crate::publisher::PublisherService>,
    feed_events: Arc<dyn FeedEventStorage>,
    sessions: Arc<dyn SessionStorage>,
    write_scope: WriteScope,
) -> Router {
    app.layer(axum::Extension(publisher))
        .layer(axum::Extension(feed_events))
        .layer(axum::Extension(sessions))
        .layer(axum::Extension(write_scope))
}

#[cfg(test)]
mod tests {
    use super::*;
    use leptos::prelude::{Owner, expect_context};
    use storage::test_support::{Backend, noop_mailer};

    #[tokio::test]
    async fn storage_context_providers_publish_every_exact_handle() {
        let env = Backend::Sqlite.setup().await;
        let accounts = account_context_provider(
            env.users(),
            env.sessions(),
            env.invites(),
            env.email_verifications(),
            env.password_resets(),
        );
        let publication = publication_context_provider(
            env.posts(),
            env.write_scope(),
            env.subscriptions(),
            env.audiences(),
            env.feed_events(),
        );
        let media_configuration =
            media_configuration_context_provider(env.media(), env.user_config(), env.site_config());
        let themes = theme_context_provider(env.themes());
        let owner = Owner::new();

        owner.with(|| {
            accounts();
            publication();
            media_configuration();
            themes();

            expect_context::<Arc<dyn UserStorage>>();
            expect_context::<Arc<dyn SessionStorage>>();
            expect_context::<Arc<dyn InviteStorage>>();
            expect_context::<Arc<dyn EmailVerificationStorage>>();
            expect_context::<Arc<dyn PasswordResetStorage>>();
            expect_context::<Arc<dyn PostStorage>>();
            expect_context::<WriteScope>();
            expect_context::<Arc<dyn SubscriptionStorage>>();
            expect_context::<Arc<dyn AudienceStorage>>();
            expect_context::<Arc<dyn FeedEventStorage>>();
            expect_context::<Arc<dyn MediaStorage>>();
            expect_context::<Arc<dyn UserConfigStorage>>();
            expect_context::<Arc<dyn SiteConfigStorage>>();
            expect_context::<Arc<dyn ThemeStorage>>();
        });
    }

    #[tokio::test]
    async fn service_context_providers_publish_every_exact_service() {
        let env = Backend::Sqlite.setup().await;
        let storage_path = Arc::new(env.base.path().to_path_buf());
        let content_locks = Arc::new(MediaContentLocks::new(Arc::clone(&storage_path)));
        let resolver: Arc<dyn storage::MediaReferenceOwnershipResolver> =
            Arc::new(crate::media_ownership::LiveMediaReferenceOwnershipResolver::new());
        let ownership = PostMediaOwnership::new(
            resolver.clone(),
            env.base.instance_id().clone(),
            env.site_config(),
        );
        let media_manager = Arc::new(MediaManager::new(
            env.media(),
            env.posts(),
            env.site_config(),
            env.write_scope(),
            Arc::clone(&content_locks),
            env.base.instance_id().clone(),
            resolver,
        ));
        let theme_asset_manager = Arc::new(ThemeAssetManager::new(
            env.themes(),
            env.write_scope(),
            storage_path,
        ));
        let theme_manager = Arc::new(ThemeManager::new(
            env.themes(),
            env.media(),
            env.write_scope(),
            Arc::clone(&content_locks),
        ));
        let theme_operation_coordinator = Arc::new(ThemeOperationCoordinator::new());
        let mailer = noop_mailer();
        let publisher = Arc::new(crate::publisher::PublisherService::new(
            env.base.path().to_path_buf(),
            env.publisher(),
            env.write_scope(),
        ));
        let services = service_context_provider(
            mailer.clone(),
            Arc::clone(&content_locks),
            Arc::clone(&media_manager),
            Arc::clone(&theme_asset_manager),
            Arc::clone(&theme_operation_coordinator),
            Arc::clone(&theme_manager),
            true,
        );
        let publisher_context = publisher_context_provider(Arc::clone(&publisher));
        let ownership_context = post_media_ownership_context_provider(ownership);
        let owner = Owner::new();

        owner.with(|| {
            services();
            publisher_context();
            ownership_context();

            assert!(Arc::ptr_eq(
                &expect_context::<Arc<dyn MailSender>>(),
                &mailer,
            ));
            assert!(Arc::ptr_eq(
                &expect_context::<Arc<MediaContentLocks>>(),
                &content_locks,
            ));
            assert!(Arc::ptr_eq(
                &expect_context::<Arc<MediaManager>>(),
                &media_manager,
            ));
            assert!(Arc::ptr_eq(
                &expect_context::<Arc<ThemeAssetManager>>(),
                &theme_asset_manager,
            ));
            assert!(Arc::ptr_eq(
                &expect_context::<Arc<ThemeOperationCoordinator>>(),
                &theme_operation_coordinator,
            ));
            assert!(Arc::ptr_eq(
                &expect_context::<Arc<ThemeManager>>(),
                &theme_manager,
            ));
            assert!(expect_context::<web::auth::CookieSettings>().secure);
            assert!(Arc::ptr_eq(
                &expect_context::<Arc<crate::publisher::PublisherService>>(),
                &publisher,
            ));
            expect_context::<Arc<dyn web::websub::WebsubPublisher>>();
            expect_context::<PostMediaOwnership>();
        });
    }
}
